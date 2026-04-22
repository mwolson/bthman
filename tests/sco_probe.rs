use std::cell::RefCell;
use std::time::Duration;

use bthman::sco_probe::{
    bytes_for_duration, classify, probe_source, ProbeResult, ProbeRunner, DEFAULT_PROBE_DURATION,
};

struct FakeProbe {
    response: RefCell<Option<Option<Vec<u8>>>>,
    calls: RefCell<Vec<(String, Duration)>>,
}

impl FakeProbe {
    fn new(response: Option<Vec<u8>>) -> Self {
        Self {
            response: RefCell::new(Some(response)),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl ProbeRunner for FakeProbe {
    fn capture_raw(&self, source: &str, duration: Duration) -> Option<Vec<u8>> {
        self.calls.borrow_mut().push((source.to_string(), duration));
        self.response.borrow_mut().take().unwrap_or(None)
    }
}

#[test]
fn classify_all_zero_buffer_is_all_zero() {
    let buf = vec![0u8; 16000];
    assert_eq!(classify(&buf), ProbeResult::AllZero);
}

#[test]
fn classify_single_nonzero_byte_is_signal() {
    let mut buf = vec![0u8; 16000];
    buf[12345] = 1;
    assert_eq!(classify(&buf), ProbeResult::HasSignal);
}

#[test]
fn classify_empty_buffer_is_unavailable() {
    assert_eq!(classify(&[]), ProbeResult::Unavailable);
}

#[test]
fn classify_typical_signal_is_signal() {
    let mut buf = vec![0u8; 16000];
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = ((i * 7) % 255) as u8;
    }
    assert_eq!(classify(&buf), ProbeResult::HasSignal);
}

#[test]
fn probe_source_returns_unavailable_when_runner_returns_none() {
    let fake = FakeProbe::new(None);
    let result = probe_source(
        &fake,
        "bluez_input.AA:BB:CC:DD:EE:FF",
        DEFAULT_PROBE_DURATION,
    );
    assert_eq!(result, ProbeResult::Unavailable);
    assert_eq!(fake.calls.borrow().len(), 1);
    assert_eq!(fake.calls.borrow()[0].0, "bluez_input.AA:BB:CC:DD:EE:FF");
    assert_eq!(fake.calls.borrow()[0].1, DEFAULT_PROBE_DURATION);
}

#[test]
fn probe_source_flags_all_zero_buffer() {
    let fake = FakeProbe::new(Some(vec![0u8; 16000]));
    let result = probe_source(
        &fake,
        "bluez_input.AA:BB:CC:DD:EE:FF",
        DEFAULT_PROBE_DURATION,
    );
    assert_eq!(result, ProbeResult::AllZero);
}

#[test]
fn probe_source_flags_signal_when_any_nonzero() {
    let mut buf = vec![0u8; 16000];
    buf[9999] = 42;
    let fake = FakeProbe::new(Some(buf));
    let result = probe_source(
        &fake,
        "bluez_input.AA:BB:CC:DD:EE:FF",
        DEFAULT_PROBE_DURATION,
    );
    assert_eq!(result, ProbeResult::HasSignal);
}

#[test]
fn bytes_for_duration_matches_500ms_at_16khz_mono_s16() {
    assert_eq!(bytes_for_duration(Duration::from_millis(500)), 16000);
    assert_eq!(bytes_for_duration(Duration::from_millis(1000)), 32000);
}
