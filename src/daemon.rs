use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{after, never, select, unbounded, Receiver};
use tracing::{error, info, warn};

use crate::bluetoothctl;
use crate::cli::Overrides;
use crate::config::{self, Config};
use crate::events::{self, PactlEvent, SignalKind};
use crate::log_watcher::{LogEvent, LogWatcher};
use crate::reconcile;
use crate::reconnect::{Completion, RealOps, Scheduler};
use crate::sco_probe::ProbeState;
use crate::signals::Handles;
use crate::sleep_monitor::{self, SleepTransition};

enum InnerExit {
    Stopped,
    Restart,
}

pub fn run_once(config: &Config) -> Result<()> {
    reconcile::reconcile(config, "manual")
}

pub fn run_watch(mut config: Config, overrides: Overrides, sigs: Handles) -> Result<()> {
    info!("Preferred HFP profiles: {:?}", config.preferred_profiles);
    let mut probe_state = ProbeState::new();
    if let Err(err) = reconcile::reconcile_persistent(&config, "startup", &mut probe_state) {
        warn!("startup reconcile failed: {:#}", err);
    }

    while !sigs.stop_requested() {
        let (mut pa_child, pa_rx) = spawn_pactl_subscribe()?;
        let (sleep_child_opt, sleep_rx) = sleep_monitor::spawn();
        let mut sleep_child_opt = sleep_child_opt;
        let mut log_watcher_opt = match LogWatcher::spawn() {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                info!(
                    "log_watcher unavailable: {:#}; running in tier-2-only mode",
                    err
                );
                None
            }
        };
        let log_rx = log_watcher_opt
            .as_ref()
            .map(LogWatcher::rx)
            .unwrap_or_else(never);
        let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
        let ops = RealOps {
            reconnect_timeout: config.reconnect_timeout,
        };
        let mut pre_sleep: HashSet<String> = HashSet::new();
        let mut pending_event: Option<PactlEvent> = None;
        let mut deadline: Option<Instant> = None;

        let exit = inner_loop(
            &mut config,
            &overrides,
            &sigs,
            &pa_rx,
            &sleep_rx,
            &log_rx,
            &ops,
            &mut scheduler,
            &mut pre_sleep,
            &mut pending_event,
            &mut deadline,
            &mut probe_state,
        );

        if matches!(exit, InnerExit::Restart) {
            if let Some(ev) = &pending_event {
                let trigger = ev.formatted();
                if let Err(err) = reconcile::reconcile_persistent_with_reconnect(
                    &config,
                    &trigger,
                    &mut probe_state,
                    &ops,
                    &mut scheduler,
                ) {
                    warn!("reconcile failed: {:#}", err);
                }
            }
        }

        stop_child(&mut pa_child);
        if let Some(mut child) = sleep_child_opt.take() {
            stop_child(&mut child);
        }
        if let Some(watcher) = log_watcher_opt.as_mut() {
            stop_child(watcher.child_mut());
        }

        if matches!(exit, InnerExit::Stopped) {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn inner_loop(
    config: &mut Config,
    overrides: &Overrides,
    sigs: &Handles,
    pa_rx: &Receiver<PactlEvent>,
    sleep_rx: &Receiver<SleepTransition>,
    log_rx: &Receiver<LogEvent>,
    ops: &RealOps,
    scheduler: &mut Scheduler,
    pre_sleep: &mut HashSet<String>,
    pending_event: &mut Option<PactlEvent>,
    deadline: &mut Option<Instant>,
    probe_state: &mut ProbeState,
) -> InnerExit {
    loop {
        if sigs.stop_requested() {
            return InnerExit::Stopped;
        }
        if sigs.take_reload() {
            reload_config(config, overrides);
        }

        let now = Instant::now();
        scheduler.process(now, ops);
        handle_scheduler_completions(scheduler, probe_state);

        let timeout = compute_timeout(Instant::now(), *deadline, scheduler, probe_state);
        let wait = after(timeout);

        select! {
            recv(pa_rx) -> msg => match msg {
                Ok(ev) => handle_event(ev, pending_event, deadline, config, probe_state, ops, scheduler),
                Err(_) => {
                    info!("pactl subscribe ended; will restart");
                    return InnerExit::Restart;
                }
            },
            recv(sleep_rx) -> msg => match msg {
                Ok(transition) => handle_sleep(transition, pre_sleep, scheduler),
                Err(_) => {
                    info!("dbus-monitor ended; will restart");
                    return InnerExit::Restart;
                }
            },
            recv(log_rx) -> msg => match msg {
                Ok(LogEvent::SeqnumFailure) => {
                    probe_state.record_seqnum_failure(Instant::now());
                    if let Err(err) = reconcile::reconcile_persistent_with_reconnect(
                        config,
                        "seqnum_failure",
                        probe_state,
                        ops,
                        scheduler,
                    ) {
                        warn!("reconcile failed: {:#}", err);
                    }
                },
                Err(_) => {
                    info!("log_watcher ended; will restart");
                    return InnerExit::Restart;
                }
            },
            recv(sigs.rx) -> msg => match msg {
                Ok(SignalKind::Stop) => return InnerExit::Stopped,
                Ok(SignalKind::Reload) => reload_config(config, overrides),
                Err(_) => {}
            },
            recv(wait) -> _ => {
                if let (Some(ev), Some(dl)) = (pending_event.as_ref(), *deadline) {
                    if Instant::now() >= dl {
                        let trigger = ev.formatted();
                        if let Err(err) = reconcile::reconcile_persistent_with_reconnect(
                            config,
                            &trigger,
                            probe_state,
                            ops,
                            scheduler,
                        )
                        {
                            warn!("reconcile failed: {:#}", err);
                        }
                        *pending_event = None;
                        *deadline = None;
                    }
                }
                if pending_event.is_none() && probe_state.next_wakeup(Instant::now()) == Some(Duration::ZERO) {
                    if let Err(err) = reconcile::reconcile_persistent_with_reconnect(
                        config,
                        "follow_up_probe",
                        probe_state,
                        ops,
                        scheduler,
                    ) {
                        warn!("reconcile failed: {:#}", err);
                    }
                }
            }
        }
    }
}

fn compute_timeout(
    now: Instant,
    debounce_deadline: Option<Instant>,
    scheduler: &Scheduler,
    probe_state: &ProbeState,
) -> Duration {
    let mut timeout = Duration::from_secs(1);
    if let Some(dl) = debounce_deadline {
        let remaining = dl.saturating_duration_since(now);
        if remaining < timeout {
            timeout = remaining;
        }
    }
    if let Some(next) = scheduler.next_due() {
        let remaining = next.saturating_duration_since(now);
        if remaining < timeout {
            timeout = remaining;
        }
    }
    if let Some(remaining) = probe_state.next_wakeup(now) {
        if remaining < timeout {
            timeout = remaining;
        }
    }
    timeout
}

fn handle_scheduler_completions(scheduler: &mut Scheduler, probe_state: &mut ProbeState) {
    for completion in scheduler.take_completed() {
        match completion {
            Completion::Connected { addr } => {
                probe_state.clear_remediation_in_progress_for_addr(&addr)
            }
            Completion::Exhausted { addr, attempts } => {
                probe_state.clear_remediation_in_progress_for_addr(&addr);
                error!(
                    "stuck_sco_remediation_failed: addr={} stage=reconnect attempts={}",
                    addr, attempts
                );
            }
        }
    }
}

fn reload_config(config: &mut Config, overrides: &Overrides) {
    match Config::build(overrides, config::default_conf_path().as_deref()) {
        Ok(new) => {
            info!("Config reloaded");
            *config = new;
        }
        Err(err) => warn!("Config reload failed; keeping previous config: {:#}", err),
    }
}

fn handle_event(
    event: PactlEvent,
    pending: &mut Option<PactlEvent>,
    deadline: &mut Option<Instant>,
    config: &Config,
    probe_state: &mut ProbeState,
    ops: &RealOps,
    scheduler: &mut Scheduler,
) {
    let new_deadline = Instant::now() + config.debounce;
    match pending.as_ref() {
        None => {
            *pending = Some(event);
            *deadline = Some(new_deadline);
        }
        Some(current) => {
            if current.key() == event.key() {
                *deadline = Some(new_deadline);
            } else {
                let trigger = current.formatted();
                if let Err(err) = reconcile::reconcile_persistent_with_reconnect(
                    config,
                    &trigger,
                    probe_state,
                    ops,
                    scheduler,
                ) {
                    warn!("reconcile failed: {:#}", err);
                }
                *pending = Some(event);
                *deadline = Some(new_deadline);
            }
        }
    }
}

fn handle_sleep(
    transition: SleepTransition,
    pre_sleep: &mut HashSet<String>,
    scheduler: &mut Scheduler,
) {
    match transition {
        SleepTransition::Suspend => {
            pre_sleep.clear();
            pre_sleep.extend(bluetoothctl::snapshot_connected_audio_devices());
            if pre_sleep.is_empty() {
                info!("Sleep: no connected audio devices to snapshot");
            } else {
                let mut sorted: Vec<&String> = pre_sleep.iter().collect();
                sorted.sort();
                info!("Sleep: snapshot connected audio devices: {:?}", sorted);
            }
        }
        SleepTransition::Resume => {
            if pre_sleep.is_empty() {
                info!("Resume: no devices to restore");
                return;
            }
            let mut sorted: Vec<&String> = pre_sleep.iter().collect();
            sorted.sort();
            info!("Resume: scheduling reconnect for {:?}", sorted);
            scheduler.schedule(Instant::now(), pre_sleep.iter().cloned());
            pre_sleep.clear();
        }
    }
}

fn spawn_pactl_subscribe() -> Result<(Child, Receiver<PactlEvent>)> {
    let mut child = Command::new("pactl")
        .args(["--format=json", "subscribe"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning pactl subscribe")?;
    let stdout = child.stdout.take().context("pactl stdout missing")?;
    let (tx, rx) = unbounded::<PactlEvent>();
    thread::Builder::new()
        .name("bthman-pactl-reader".into())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(event) = PactlEvent::parse(&line) {
                    if events::is_interesting(&event) {
                        let _ = tx.send(event);
                    }
                }
            }
        })?;
    Ok((child, rx))
}

fn stop_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}
