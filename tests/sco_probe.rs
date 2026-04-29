use std::cell::RefCell;
use std::time::{Duration, Instant};

use bthman::sco_probe::{
    bytes_for_duration, classify, probe_source, ProbeAction, ProbeResult, ProbeRunner, ProbeState,
    DEFAULT_PROBE_DURATION,
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

#[test]
fn probe_state_allows_first_probe() {
    let mut state = ProbeState::new();
    let now = Instant::now();
    assert!(state.should_probe(
        "bluez_input.AA:BB:CC:DD:EE:FF",
        now,
        Duration::from_secs(20)
    ));
}

#[test]
fn probe_state_blocks_second_probe_within_cooldown() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    assert!(state.should_probe("bluez_input.AA:BB:CC:DD:EE:FF", t0, Duration::from_secs(20)));
    let t1 = t0 + Duration::from_secs(5);
    assert!(!state.should_probe("bluez_input.AA:BB:CC:DD:EE:FF", t1, Duration::from_secs(20)));
}

#[test]
fn probe_state_force_bypasses_cooldown() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    let source = "bluez_input.AA:BB:CC:DD:EE:FF";
    assert!(state.should_probe(source, t0, Duration::from_secs(20)));
    assert_eq!(
        state.next_action_with_force(
            source,
            t0 + Duration::from_secs(5),
            Duration::from_secs(20),
            true
        ),
        ProbeAction::Probe
    );
}

#[test]
fn probe_state_allows_second_probe_after_cooldown() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    assert!(state.should_probe("bluez_input.AA:BB:CC:DD:EE:FF", t0, Duration::from_secs(20)));
    let t1 = t0 + Duration::from_secs(21);
    assert!(state.should_probe("bluez_input.AA:BB:CC:DD:EE:FF", t1, Duration::from_secs(20)));
}

#[test]
fn probe_state_tracks_sources_independently() {
    let mut state = ProbeState::new();
    let now = Instant::now();
    assert!(state.should_probe(
        "bluez_input.AA:BB:CC:DD:EE:FF",
        now,
        Duration::from_secs(20)
    ));
    assert!(state.should_probe(
        "bluez_input.11:22:33:44:55:66",
        now,
        Duration::from_secs(20)
    ));
    assert!(!state.should_probe(
        "bluez_input.AA:BB:CC:DD:EE:FF",
        now,
        Duration::from_secs(20)
    ));
}

#[test]
fn all_zero_arms_follow_up_wakeup() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    state.record_probe("bluez_input.AA:BB:CC:DD:EE:FF", &ProbeResult::AllZero, t0);
    assert_eq!(
        state.next_action(
            "bluez_input.AA:BB:CC:DD:EE:FF",
            t0 + Duration::from_secs(2),
            Duration::from_secs(20)
        ),
        ProbeAction::FollowUpProbe
    );
    assert_eq!(
        state.next_wakeup(t0 + Duration::from_secs(1)),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn has_signal_clears_follow_up() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    let source = "bluez_input.AA:BB:CC:DD:EE:FF";
    state.record_probe(source, &ProbeResult::AllZero, t0);
    state.record_probe(source, &ProbeResult::HasSignal, t0 + Duration::from_secs(1));
    assert_eq!(state.next_wakeup(t0 + Duration::from_secs(1)), None);
    assert!(!state.prior_all_zero_recent(source, t0 + Duration::from_secs(1)));
}

#[test]
fn seqnum_recent_expires_after_window() {
    let mut state = ProbeState::new();
    let t0 = Instant::now();
    state.record_seqnum_failure(t0);
    assert!(state.seqnum_recent(t0 + Duration::from_secs(5)));
    assert!(!state.seqnum_recent(t0 + Duration::from_secs(6)));
}

#[test]
fn remediation_in_progress_is_visible_by_source() {
    let mut state = ProbeState::new();
    let source = "bluez_input.AA:BB:CC:DD:EE:FF";
    state.set_remediation_in_progress(source);
    assert!(state.is_remediation_in_progress(source));
    state.clear_remediation_in_progress_for_addr("AA:BB:CC:DD:EE:FF");
    assert!(!state.is_remediation_in_progress(source));
}
