use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

pub const PROBE_RATE: u32 = 16000;
pub const PROBE_CHANNELS: u32 = 1;
pub const PROBE_SAMPLE_BYTES: usize = 2;
pub const DEFAULT_PROBE_DURATION: Duration = Duration::from_millis(500);
pub const DEFAULT_PROBE_COOLDOWN: Duration = Duration::from_secs(20);
pub const FOLLOW_UP_DELAY: Duration = Duration::from_secs(2);
pub const FOLLOW_UP_WINDOW: Duration = Duration::from_secs(3);
pub const MIN_HFP_UPTIME: Duration = Duration::from_secs(3);
pub const PROBE_GRACE: Duration = Duration::from_secs(1);
pub const REMEDIATION_RATE_LIMIT: Duration = Duration::from_secs(300);
pub const SEQNUM_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct ProbeState {
    sources: HashMap<String, SourceState>,
    devices: HashMap<String, DeviceState>,
    hfp_addrs: HashSet<String>,
    last_seqnum_failure: Option<Instant>,
    remediation_in_progress: HashSet<String>,
}

#[derive(Debug, Default)]
struct SourceState {
    last_probe: Option<Instant>,
    last_all_zero: Option<Instant>,
    follow_up_due: Option<Instant>,
}

#[derive(Debug, Default)]
struct DeviceState {
    first_hfp_seen: Option<Instant>,
    last_remediation: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeAction {
    Probe,
    FollowUpProbe,
    Skip(SkipReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Cooldown,
    RemediationInProgress,
}

impl ProbeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_action(&self, source: &str, now: Instant, cooldown: Duration) -> ProbeAction {
        self.next_action_with_force(source, now, cooldown, false)
    }

    pub fn next_action_with_force(
        &self,
        source: &str,
        now: Instant,
        cooldown: Duration,
        force: bool,
    ) -> ProbeAction {
        if self.is_remediation_in_progress(source) {
            return ProbeAction::Skip(SkipReason::RemediationInProgress);
        }
        let Some(state) = self.sources.get(source) else {
            return ProbeAction::Probe;
        };
        if state.follow_up_due.is_some_and(|due| now >= due) {
            return ProbeAction::FollowUpProbe;
        }
        if force {
            return ProbeAction::Probe;
        }
        if state
            .last_probe
            .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
        {
            return ProbeAction::Skip(SkipReason::Cooldown);
        }
        ProbeAction::Probe
    }

    pub fn should_probe(&mut self, source: &str, now: Instant, cooldown: Duration) -> bool {
        let should_probe = matches!(
            self.next_action(source, now, cooldown),
            ProbeAction::Probe | ProbeAction::FollowUpProbe
        );
        if should_probe {
            self.sources
                .entry(source.to_string())
                .or_default()
                .last_probe = Some(now);
        }
        should_probe
    }

    pub fn record_probe(&mut self, source: &str, result: &ProbeResult, now: Instant) {
        let state = self.sources.entry(source.to_string()).or_default();
        state.last_probe = Some(now);
        match result {
            ProbeResult::AllZero | ProbeResult::NoData => {
                state.last_all_zero = Some(now);
                state.follow_up_due = Some(now + FOLLOW_UP_DELAY);
            }
            ProbeResult::HasSignal => {
                state.last_all_zero = None;
                state.follow_up_due = None;
            }
            ProbeResult::Unavailable => {}
        }
    }

    pub fn record_seqnum_failure(&mut self, now: Instant) {
        self.last_seqnum_failure = Some(now);
    }

    pub fn seqnum_recent(&self, now: Instant) -> bool {
        self.last_seqnum_failure
            .is_some_and(|last| now.saturating_duration_since(last) <= SEQNUM_WINDOW)
    }

    pub fn prior_all_zero_recent(&self, source: &str, now: Instant) -> bool {
        self.sources
            .get(source)
            .and_then(|state| state.last_all_zero)
            .is_some_and(|last| now.saturating_duration_since(last) <= FOLLOW_UP_WINDOW)
    }

    pub fn record_hfp_seen(&mut self, addr: &str, now: Instant) {
        self.hfp_addrs.insert(addr.to_string());
        let state = self.devices.entry(addr.to_string()).or_default();
        state.first_hfp_seen.get_or_insert(now);
    }

    pub fn retain_hfp_addrs<'a, I>(&mut self, addrs: I)
    where
        I: IntoIterator<Item = &'a String>,
    {
        let current: HashSet<String> = addrs.into_iter().cloned().collect();
        let previous: Vec<String> = self.hfp_addrs.difference(&current).cloned().collect();
        for addr in previous {
            self.record_hfp_left(&addr);
        }
        self.hfp_addrs = current;
    }

    pub fn record_hfp_left(&mut self, addr: &str) {
        self.devices.remove(addr);
        self.sources.remove(&source_from_addr(addr));
    }

    pub fn hfp_uptime(&self, addr: &str, now: Instant) -> Option<Duration> {
        self.devices
            .get(addr)
            .and_then(|state| state.first_hfp_seen)
            .map(|seen| now.saturating_duration_since(seen))
    }

    pub fn last_remediation_recent(&self, addr: &str, now: Instant) -> bool {
        self.devices
            .get(addr)
            .and_then(|state| state.last_remediation)
            .is_some_and(|last| now.saturating_duration_since(last) < REMEDIATION_RATE_LIMIT)
    }

    pub fn record_remediation(&mut self, addr: &str, now: Instant) {
        self.devices
            .entry(addr.to_string())
            .or_default()
            .last_remediation = Some(now);
    }

    pub fn set_remediation_in_progress(&mut self, source: &str) {
        self.remediation_in_progress.insert(source.to_string());
    }

    pub fn clear_remediation_in_progress_for_addr(&mut self, addr: &str) {
        self.remediation_in_progress.remove(&source_from_addr(addr));
    }

    pub fn is_remediation_in_progress(&self, source: &str) -> bool {
        self.remediation_in_progress.contains(source)
    }

    pub fn next_wakeup(&self, now: Instant) -> Option<Duration> {
        self.sources
            .values()
            .filter_map(|state| state.follow_up_due)
            .map(|due| due.saturating_duration_since(now))
            .min()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    AllZero,
    HasSignal,
    NoData,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capture {
    Data(Vec<u8>),
    Failed,
    NoData,
}

fn source_from_addr(addr: &str) -> String {
    format!("bluez_input.{}", addr)
}

pub trait ProbeRunner {
    fn capture_raw(&self, source: &str, duration: Duration) -> Capture;
}

pub struct RealProbe;

impl ProbeRunner for RealProbe {
    fn capture_raw(&self, source: &str, duration: Duration) -> Capture {
        let target_bytes = bytes_for_duration(duration);
        // pacat 17.0-98 (PipeWire's pulse-compat) writes zero bytes when the
        // output path is "-"; /dev/stdout works correctly via the piped fd.
        let mut child = match Command::new("parecord")
            .args([
                "--device",
                source,
                "--channels",
                &PROBE_CHANNELS.to_string(),
                "--rate",
                &PROBE_RATE.to_string(),
                "--format",
                "s16le",
                "--file-format=raw",
                "--latency-msec=50",
                "/dev/stdout",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(err) => {
                debug!("parecord spawn failed: {}", err);
                return Capture::Failed;
            }
        };

        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Capture::Failed;
        };

        // A stuck SCO link can leave parecord blocked with no data, so reads
        // happen on a helper thread and the deadline is enforced here.
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || forward_chunks(stdout, tx));

        let wait = duration + PROBE_GRACE;
        let deadline = Instant::now() + wait;
        let mut buf = Vec::with_capacity(target_bytes);
        let mut timed_out = false;
        while buf.len() < target_bytes {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    timed_out = true;
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();

        if buf.is_empty() && timed_out {
            debug!("parecord produced no data within {}ms", wait.as_millis());
            return Capture::NoData;
        }
        if buf.len() < target_bytes / 2 {
            debug!(
                "parecord produced {} bytes (wanted {}); treating as unavailable",
                buf.len(),
                target_bytes
            );
            return Capture::Failed;
        }
        Capture::Data(buf)
    }
}

fn forward_chunks(mut stdout: impl Read, tx: mpsc::Sender<Vec<u8>>) {
    let mut scratch = [0u8; 4096];
    loop {
        match stdout.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(scratch[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(err) => {
                debug!("parecord read failed: {}", err);
                break;
            }
        }
    }
}

pub fn bytes_for_duration(duration: Duration) -> usize {
    let secs = duration.as_secs_f64();
    let samples = (secs * PROBE_RATE as f64).round() as usize;
    samples * PROBE_CHANNELS as usize * PROBE_SAMPLE_BYTES
}

pub fn probe_source(runner: &dyn ProbeRunner, source: &str, duration: Duration) -> ProbeResult {
    match runner.capture_raw(source, duration) {
        Capture::Data(buf) => classify(&buf),
        Capture::Failed => ProbeResult::Unavailable,
        Capture::NoData => ProbeResult::NoData,
    }
}

pub fn classify(buf: &[u8]) -> ProbeResult {
    if buf.is_empty() {
        return ProbeResult::Unavailable;
    }
    if buf.iter().any(|&b| b != 0) {
        ProbeResult::HasSignal
    } else {
        ProbeResult::AllZero
    }
}

pub fn log_result(source: &str, result: &ProbeResult) {
    match result {
        ProbeResult::AllZero => warn!(
            "Probe: {} producing zero-valued samples (possible stuck SCO); \
             reconnect the device to recover",
            source
        ),
        ProbeResult::HasSignal => {
            info!("Probe: {} has signal", source);
        }
        ProbeResult::NoData => warn!(
            "Probe: {} delivered no audio before the capture deadline (possible stuck SCO); \
             reconnect the device to recover",
            source
        ),
        ProbeResult::Unavailable => {
            info!(
                "Probe: {} unavailable (parecord missing or capture failed)",
                source
            );
        }
    }
}
