use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

pub const PROBE_RATE: u32 = 16000;
pub const PROBE_CHANNELS: u32 = 1;
pub const PROBE_SAMPLE_BYTES: usize = 2;
pub const DEFAULT_PROBE_DURATION: Duration = Duration::from_millis(500);
pub const DEFAULT_PROBE_COOLDOWN: Duration = Duration::from_secs(20);

#[derive(Debug, Default)]
pub struct ProbeState {
    last_probe: HashMap<String, Instant>,
}

impl ProbeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a probe attempt for `source` at `now` and returns true if the
    /// caller should proceed, or false if `source` was probed within `cooldown`.
    pub fn should_probe(&mut self, source: &str, now: Instant, cooldown: Duration) -> bool {
        if let Some(last) = self.last_probe.get(source) {
            if now.saturating_duration_since(*last) < cooldown {
                return false;
            }
        }
        self.last_probe.insert(source.to_string(), now);
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    AllZero,
    HasSignal,
    Unavailable,
}

pub trait ProbeRunner {
    fn capture_raw(&self, source: &str, duration: Duration) -> Option<Vec<u8>>;
}

pub struct RealProbe;

impl ProbeRunner for RealProbe {
    fn capture_raw(&self, source: &str, duration: Duration) -> Option<Vec<u8>> {
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
                return None;
            }
        };

        let mut stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        };

        let deadline = Instant::now() + duration + Duration::from_millis(500);
        let mut buf = Vec::with_capacity(target_bytes);
        let mut scratch = [0u8; 4096];
        while buf.len() < target_bytes && Instant::now() < deadline {
            match stdout.read(&mut scratch) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&scratch[..n]),
                Err(err) => {
                    debug!("parecord read failed: {}", err);
                    break;
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();

        if buf.len() < target_bytes / 2 {
            debug!(
                "parecord produced {} bytes (wanted {}); treating as unavailable",
                buf.len(),
                target_bytes
            );
            return None;
        }
        Some(buf)
    }
}

pub fn bytes_for_duration(duration: Duration) -> usize {
    let secs = duration.as_secs_f64();
    let samples = (secs * PROBE_RATE as f64).round() as usize;
    samples * PROBE_CHANNELS as usize * PROBE_SAMPLE_BYTES
}

pub fn probe_source(runner: &dyn ProbeRunner, source: &str, duration: Duration) -> ProbeResult {
    let Some(buf) = runner.capture_raw(source, duration) else {
        return ProbeResult::Unavailable;
    };
    classify(&buf)
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
        ProbeResult::Unavailable => {
            info!(
                "Probe: {} unavailable (parecord missing or capture failed)",
                source
            );
        }
    }
}
