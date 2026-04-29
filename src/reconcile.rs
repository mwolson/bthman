use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, info};

use crate::config::Config;
use crate::pactl::{self, PactlRunner, RealPactl};
use crate::reconnect::{BluetoothOps, Scheduler};
use crate::remediation::{self, RemediationRequest};
use crate::sco_probe::{
    self, ProbeAction, ProbeResult, ProbeRunner, ProbeState, RealProbe, DEFAULT_PROBE_COOLDOWN,
    DEFAULT_PROBE_DURATION, MIN_HFP_UPTIME,
};
use crate::wpctl;

pub fn reconcile(config: &Config, trigger: &str) -> Result<()> {
    let mut state = ProbeState::new();
    reconcile_persistent(config, trigger, &mut state)
}

pub fn reconcile_persistent(
    config: &Config,
    trigger: &str,
    probe_state: &mut ProbeState,
) -> Result<()> {
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    reconcile_persistent_with_reconnect(
        config,
        trigger,
        probe_state,
        &NoBluetoothOps,
        &mut scheduler,
    )
}

pub fn reconcile_persistent_with_reconnect(
    config: &Config,
    trigger: &str,
    probe_state: &mut ProbeState,
    bluetooth: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
) -> Result<()> {
    reconcile_with_reconnect(
        &RealPactl,
        &RealProbe,
        probe_state,
        bluetooth,
        scheduler,
        &wpctl::external_recorder_active,
        &|| thread::sleep(Duration::from_secs(1)),
        config,
        trigger,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn reconcile_with(
    runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    probe_state: &mut ProbeState,
    recorder_active: &dyn Fn() -> bool,
    post_change: &dyn Fn(),
    config: &Config,
    trigger: &str,
) -> Result<()> {
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    reconcile_with_reconnect(
        runner,
        probe,
        probe_state,
        &NoBluetoothOps,
        &mut scheduler,
        recorder_active,
        post_change,
        config,
        trigger,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn reconcile_with_reconnect(
    runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    probe_state: &mut ProbeState,
    bluetooth: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
    recorder_active: &dyn Fn() -> bool,
    post_change: &dyn Fn(),
    config: &Config,
    trigger: &str,
) -> Result<()> {
    info!("Reconciling: {}", trigger);
    if recorder_active() {
        info!("External recorder active, skipping reconciliation");
        return Ok(());
    }
    let cards_dump = runner.run(&["list", "cards"])?;
    let mut changed = false;
    for card in pactl::list_bluetooth_cards(runner)? {
        if pactl::prefer_headset_profile(runner, &cards_dump, &card, &config.preferred_profiles)? {
            changed = true;
        }
    }
    if changed {
        post_change();
    }
    if pactl::fix_default_source(runner, &cards_dump, config.input_volume)? {
        changed = true;
    }
    if !changed {
        info!("No changes needed");
    }
    if config.probe_stuck_sco {
        let force_probe = should_force_probe(trigger);
        probe_bluetooth_sources(
            runner,
            probe,
            probe_state,
            bluetooth,
            scheduler,
            config,
            &cards_dump,
            force_probe,
        )?;
    }
    Ok(())
}

fn should_force_probe(trigger: &str) -> bool {
    trigger == "seqnum_failure" || trigger.contains(" on source")
}

#[allow(clippy::too_many_arguments)]
fn probe_bluetooth_sources(
    runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    probe_state: &mut ProbeState,
    bluetooth: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
    config: &Config,
    cards_dump: &str,
    force_probe: bool,
) -> Result<()> {
    let now = Instant::now();
    let mut hfp_addrs = std::collections::HashSet::new();
    for card in pactl::list_bluetooth_cards(runner)? {
        let active = pactl::get_active_profile(cards_dump, &card);
        if !active.starts_with("headset-head-unit") {
            continue;
        }
        let addr = card
            .strip_prefix("bluez_card.")
            .unwrap_or(&card)
            .replace('_', ":");
        hfp_addrs.insert(addr.clone());
        probe_state.record_hfp_seen(&addr, now);
        let source = format!("bluez_input.{}", addr);
        if probe_state.is_remediation_in_progress(&source) {
            log_decision(DecisionLog {
                source: &source,
                tier: "na",
                seqnum_recent: probe_state.seqnum_recent(now),
                all_zero_count: 0,
                uptime: probe_state.hfp_uptime(&addr, now).unwrap_or(Duration::ZERO),
                rate_limited: false,
                action: "skip_in_progress",
                addr: &addr,
            });
            continue;
        }
        if pactl::source_is_muted(runner, &source) {
            log_decision(DecisionLog {
                source: &source,
                tier: "na",
                seqnum_recent: probe_state.seqnum_recent(now),
                all_zero_count: 0,
                uptime: probe_state.hfp_uptime(&addr, now).unwrap_or(Duration::ZERO),
                rate_limited: false,
                action: "skip_muted",
                addr: &addr,
            });
            continue;
        }
        let prior_all_zero = probe_state.prior_all_zero_recent(&source, now);
        let action =
            probe_state.next_action_with_force(&source, now, DEFAULT_PROBE_COOLDOWN, force_probe);
        match action {
            ProbeAction::Probe | ProbeAction::FollowUpProbe => {}
            ProbeAction::Skip(_) => {
                debug!("Probe skipped (cooldown): {}", source);
                continue;
            }
        }
        let result = sco_probe::probe_source(probe, &source, DEFAULT_PROBE_DURATION);
        sco_probe::log_result(&source, &result);
        probe_state.record_probe(&source, &result, now);
        handle_probe_result(
            runner,
            bluetooth,
            scheduler,
            probe_state,
            config,
            cards_dump,
            &card,
            &addr,
            &source,
            &result,
            prior_all_zero,
            now,
        );
    }
    probe_state.retain_hfp_addrs(&hfp_addrs);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_probe_result(
    runner: &dyn PactlRunner,
    bluetooth: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
    probe_state: &mut ProbeState,
    config: &Config,
    cards_dump: &str,
    card: &str,
    addr: &str,
    source: &str,
    result: &ProbeResult,
    prior_all_zero: bool,
    now: Instant,
) {
    if !config.auto_recover_stuck_sco.enabled() || !matches!(result, ProbeResult::AllZero) {
        return;
    }

    let uptime = probe_state.hfp_uptime(addr, now).unwrap_or(Duration::ZERO);
    let seqnum_recent = probe_state.seqnum_recent(now);
    let all_zero_count = if prior_all_zero { 2 } else { 1 };

    if probe_state.last_remediation_recent(addr, now) {
        log_decision(DecisionLog {
            source,
            tier: "na",
            seqnum_recent,
            all_zero_count,
            uptime,
            rate_limited: true,
            action: "skip_recent_remediation",
            addr,
        });
        return;
    }
    if uptime < MIN_HFP_UPTIME {
        log_decision(DecisionLog {
            source,
            tier: "na",
            seqnum_recent,
            all_zero_count,
            uptime,
            rate_limited: false,
            action: "skip_min_uptime",
            addr,
        });
        return;
    }
    if !pactl::get_active_profile(cards_dump, card).starts_with("headset-head-unit") {
        log_decision(DecisionLog {
            source,
            tier: "na",
            seqnum_recent,
            all_zero_count,
            uptime,
            rate_limited: false,
            action: "skip_profile_changed",
            addr,
        });
        return;
    }
    if pactl::source_is_muted(runner, source) {
        log_decision(DecisionLog {
            source,
            tier: "na",
            seqnum_recent,
            all_zero_count,
            uptime,
            rate_limited: false,
            action: "skip_muted",
            addr,
        });
        return;
    }

    let tier = if seqnum_recent {
        Some(1)
    } else if prior_all_zero {
        Some(2)
    } else {
        None
    };

    let Some(tier) = tier else {
        log_decision(DecisionLog {
            source,
            tier: "na",
            seqnum_recent: false,
            all_zero_count: 1,
            uptime,
            rate_limited: false,
            action: "arm_follow_up",
            addr,
        });
        return;
    };

    remediation::remediate(
        bluetooth,
        scheduler,
        probe_state,
        RemediationRequest {
            addr,
            all_zero_count,
            mode: config.auto_recover_stuck_sco,
            seqnum_recent,
            source,
            tier,
            uptime_s: uptime.as_secs(),
        },
        now,
    );
}

struct DecisionLog<'a> {
    source: &'a str,
    tier: &'a str,
    seqnum_recent: bool,
    all_zero_count: u8,
    uptime: Duration,
    rate_limited: bool,
    action: &'a str,
    addr: &'a str,
}

fn log_decision(decision: DecisionLog<'_>) {
    info!(
        "stuck_sco_decision: source={} tier={} seqnum_recent={} all_zero_count={} uptime_s={} rate_limited={} action={} addr={}",
        decision.source,
        decision.tier,
        decision.seqnum_recent,
        decision.all_zero_count,
        decision.uptime.as_secs(),
        decision.rate_limited,
        decision.action,
        decision.addr
    );
}

struct NoBluetoothOps;

impl BluetoothOps for NoBluetoothOps {
    fn device_is_connected(&self, _addr: &str) -> bool {
        false
    }

    fn try_disconnect(&self, _addr: &str) -> bool {
        false
    }

    fn try_reconnect(&self, _addr: &str) -> bool {
        false
    }
}
