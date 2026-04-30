use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use bthman::cli::{AutoRecoverMode, Overrides};
use bthman::config::Config;
use bthman::pactl::PactlRunner;
use bthman::reconcile::{reconcile_with, reconcile_with_reconnect};
use bthman::reconnect::{BluetoothOps, Scheduler};
use bthman::sco_probe::{ProbeResult, ProbeRunner, ProbeState};

struct NoProbe;

impl ProbeRunner for NoProbe {
    fn capture_raw(&self, _source: &str, _duration: Duration) -> Option<Vec<u8>> {
        None
    }
}

struct RecordingProbe {
    calls: RefCell<Vec<String>>,
    response: RefCell<Option<Vec<u8>>>,
}

impl RecordingProbe {
    fn new(response: Option<Vec<u8>>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            response: RefCell::new(response),
        }
    }
}

impl ProbeRunner for RecordingProbe {
    fn capture_raw(&self, source: &str, _duration: Duration) -> Option<Vec<u8>> {
        self.calls.borrow_mut().push(source.to_string());
        self.response.borrow().clone()
    }
}

struct QueueProbe {
    calls: RefCell<Vec<String>>,
    responses: RefCell<VecDeque<Option<Vec<u8>>>>,
}

impl QueueProbe {
    fn new(responses: Vec<Option<Vec<u8>>>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            responses: RefCell::new(responses.into()),
        }
    }
}

impl ProbeRunner for QueueProbe {
    fn capture_raw(&self, source: &str, _duration: Duration) -> Option<Vec<u8>> {
        self.calls.borrow_mut().push(source.to_string());
        self.responses.borrow_mut().pop_front().flatten()
    }
}

struct FakePactl {
    responses: RefCell<VecDeque<(Vec<String>, String)>>,
    calls: RefCell<Vec<Vec<String>>>,
}

struct FakeBluetooth {
    disconnects: RefCell<Vec<String>>,
}

impl FakeBluetooth {
    fn new() -> Self {
        Self {
            disconnects: RefCell::new(Vec::new()),
        }
    }
}

impl BluetoothOps for FakeBluetooth {
    fn device_is_connected(&self, _addr: &str) -> bool {
        false
    }

    fn try_disconnect(&self, addr: &str) -> bool {
        self.disconnects.borrow_mut().push(addr.to_string());
        true
    }

    fn try_reconnect(&self, _addr: &str) -> bool {
        false
    }
}

impl FakePactl {
    fn new(responses: Vec<(Vec<&str>, &str)>) -> Self {
        Self::new_owned(
            responses
                .into_iter()
                .map(|(args, body)| {
                    (
                        args.into_iter().map(String::from).collect(),
                        body.to_string(),
                    )
                })
                .collect(),
        )
    }

    fn new_owned(responses: Vec<(Vec<String>, String)>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn has_call(&self, args: &[&str]) -> bool {
        let expected: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.calls.borrow().iter().any(|c| c == &expected)
    }
}

impl PactlRunner for FakePactl {
    fn run(&self, args: &[&str]) -> Result<String> {
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.calls.borrow_mut().push(args_owned.clone());
        let mut responses = self.responses.borrow_mut();
        if let Some(pos) = responses.iter().position(|(a, _)| *a == args_owned) {
            let (_, body) = responses.remove(pos).unwrap();
            Ok(body)
        } else {
            Ok(String::new())
        }
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.calls.borrow_mut().push(args_owned.clone());
        let mut responses = self.responses.borrow_mut();
        if let Some(pos) = responses.iter().position(|(a, _)| *a == args_owned) {
            let (_, body) = responses.remove(pos).unwrap();
            body
        } else {
            String::new()
        }
    }
}

fn cards_dump(active: &str) -> String {
    format!(
        "Card #42
\tName: bluez_card.AA_BB_CC_DD_EE_FF
\tDriver: module-bluez5-device.c
\tProfiles:
\t\theadset-head-unit: Headset Head Unit (HSP/HFP) (codec LC3-24kHz)
\t\theadset-head-unit-msbc: Headset Head Unit (HSP/HFP) (codec mSBC)
\t\toff: Off
\tActive Profile: {}
",
        active
    )
}

fn test_config(preferred: Vec<&str>) -> Config {
    let mut config = base_config(preferred);
    config.probe_stuck_sco = false;
    config
}

fn base_config(preferred: Vec<&str>) -> Config {
    let overrides = Overrides {
        preferred_profiles: Some(preferred.into_iter().map(String::from).collect()),
        ..Default::default()
    };
    Config::build(&overrides, None).unwrap()
}

fn remediation_config(mode: AutoRecoverMode) -> Config {
    let mut config = base_config(vec!["headset-head-unit"]);
    config.auto_recover_stuck_sco = mode;
    config
}

fn hfp_probe_responses() -> Vec<(Vec<String>, String)> {
    vec![
        (
            vec!["list".into(), "cards".into()],
            cards_dump("headset-head-unit"),
        ),
        (
            vec!["list".into(), "cards".into(), "short".into()],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n".into(),
        ),
        (
            vec!["list".into(), "cards".into(), "short".into()],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n".into(),
        ),
        (
            vec!["list".into(), "cards".into(), "short".into()],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n".into(),
        ),
        (
            vec!["info".into()],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n".into(),
        ),
        (
            vec![
                "get-source-mute".into(),
                "bluez_input.AA:BB:CC:DD:EE:FF".into(),
            ],
            "Mute: no\n".into(),
        ),
        (
            vec![
                "get-source-mute".into(),
                "bluez_input.AA:BB:CC:DD:EE:FF".into(),
            ],
            "Mute: no\n".into(),
        ),
    ]
}

fn hfp_fake() -> FakePactl {
    FakePactl::new_owned(hfp_probe_responses())
}

#[test]
fn corrects_profile_when_external_recorder_active() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec![
                "set-card-profile",
                "bluez_card.AA_BB_CC_DD_EE_FF",
                "headset-head-unit-msbc",
            ],
            "",
        ),
    ]);
    let config = test_config(vec!["headset-head-unit-msbc"]);
    reconcile_with(
        &fake,
        &NoProbe,
        &mut ProbeState::new(),
        &|| true,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(fake.has_call(&[
        "set-card-profile",
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "headset-head-unit-msbc"
    ]));
    assert!(!fake.has_call(&["info"]));
}

#[test]
fn no_change_when_active_matches_preferred() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Server String: foo\nDefault Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
    ]);
    let config = test_config(vec!["headset-head-unit", "headset-head-unit-msbc"]);
    reconcile_with(
        &fake,
        &NoProbe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(!fake.has_call(&[
        "set-card-profile",
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "headset-head-unit"
    ]));
    assert!(!fake.has_call(&["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"]));
}

#[test]
fn switches_card_profile_to_preferred() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec![
                "set-card-profile",
                "bluez_card.AA_BB_CC_DD_EE_FF",
                "headset-head-unit-msbc",
            ],
            "",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
    ]);
    let config = test_config(vec!["headset-head-unit-msbc"]);
    reconcile_with(
        &fake,
        &NoProbe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(fake.has_call(&[
        "set-card-profile",
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "headset-head-unit-msbc"
    ]));
}

#[test]
fn switches_default_source_from_monitor() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("off")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_output.AA:BB:CC:DD:EE:FF.monitor\n",
        ),
        (
            vec!["list", "short", "sources"],
            "1\tbluez_input.AA:BB:CC:DD:EE:FF\tmodule\n",
        ),
        (
            vec!["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "",
        ),
    ]);
    let config = test_config(vec!["headset-head-unit"]);
    reconcile_with(
        &fake,
        &NoProbe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(fake.has_call(&["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"]));
}

#[test]
fn does_not_switch_when_preferred_unavailable() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("off")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_output.AA:BB:CC:DD:EE:FF.monitor\n",
        ),
        (vec!["list", "short", "sources"], "1\tsome_other_source\n"),
    ]);
    let config = test_config(vec!["headset-head-unit"]);
    reconcile_with(
        &fake,
        &NoProbe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(!fake.has_call(&["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"]));
}

#[test]
fn probe_skipped_when_card_is_not_hfp() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("a2dp-sink")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_output.AA:BB:CC:DD:EE:FF.monitor\n",
        ),
        (
            vec!["list", "short", "sources"],
            "1\tbluez_input.AA:BB:CC:DD:EE:FF\tmodule\n",
        ),
        (
            vec!["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(
        &fake,
        &probe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "probe must not run on non-HFP card"
    );
}

// Gate relaxed: the probe now fires on HFP-active cards regardless of whether
// any application has an open source-output, because AirPods Pro can reach the
// stuck-SCO state even with no recorders attached. The only remaining gates
// are HFP-active, not-muted, and the 20s per-source cooldown.
#[test]
fn probe_runs_when_hfp_active_and_detects_all_zero() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(
        &fake,
        &probe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert_eq!(
        probe.calls.borrow().as_slice(),
        &["bluez_input.AA:BB:CC:DD:EE:FF".to_string()]
    );
    assert!(
        !fake.has_call(&["list", "short", "source-outputs"]),
        "probe_bluetooth_sources must no longer gate on source-output count"
    );
}

#[test]
fn probe_skipped_when_source_is_muted() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: yes\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(
        &fake,
        &probe,
        &mut ProbeState::new(),
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "probe must not run on muted source"
    );
}

#[test]
fn auto_recover_on_disconnects_and_schedules_when_seqnum_recent() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let bluetooth = FakeBluetooth::new();
    let config = remediation_config(AutoRecoverMode::On);
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(11));
    state.record_seqnum_failure(now);

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();

    assert_eq!(
        bluetooth.disconnects.borrow().as_slice(),
        &["AA:BB:CC:DD:EE:FF".to_string()]
    );
    assert!(!scheduler.is_empty());
    assert!(state.is_remediation_in_progress("bluez_input.AA:BB:CC:DD:EE:FF"));
}

#[test]
fn auto_recover_dry_run_does_not_disconnect() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("headset-head-unit")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
        (
            vec!["info"],
            "Default Source: bluez_input.AA:BB:CC:DD:EE:FF\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let bluetooth = FakeBluetooth::new();
    let config = remediation_config(AutoRecoverMode::DryRun);
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(11));
    state.record_seqnum_failure(now);

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();

    assert!(bluetooth.disconnects.borrow().is_empty());
    assert!(scheduler.is_empty());
    assert!(!state.is_remediation_in_progress("bluez_input.AA:BB:CC:DD:EE:FF"));
}

#[test]
fn auto_recover_tier_2_disconnects_after_second_all_zero() {
    let mut responses = hfp_probe_responses();
    responses.extend(hfp_probe_responses());
    let fake = FakePactl::new_owned(responses);
    let probe = QueueProbe::new(vec![Some(vec![0u8; 16000]), Some(vec![0u8; 16000])]);
    let bluetooth = FakeBluetooth::new();
    let config = remediation_config(AutoRecoverMode::On);
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(11));

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "first",
    )
    .unwrap();
    assert!(bluetooth.disconnects.borrow().is_empty());

    state.record_probe(
        "bluez_input.AA:BB:CC:DD:EE:FF",
        &ProbeResult::AllZero,
        std::time::Instant::now() - Duration::from_secs(2),
    );
    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "follow_up_probe",
    )
    .unwrap();

    assert_eq!(
        bluetooth.disconnects.borrow().as_slice(),
        &["AA:BB:CC:DD:EE:FF".to_string()]
    );
    assert!(!scheduler.is_empty());
}

#[test]
fn auto_recover_skips_when_hfp_uptime_is_too_short() {
    let fake = hfp_fake();
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let bluetooth = FakeBluetooth::new();
    let config = remediation_config(AutoRecoverMode::On);
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(9));
    state.record_seqnum_failure(now);

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();

    assert!(bluetooth.disconnects.borrow().is_empty());
    assert!(scheduler.is_empty());
}

#[test]
fn auto_recover_rate_limits_repeated_remediation() {
    let fake = hfp_fake();
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let bluetooth = FakeBluetooth::new();
    let config = remediation_config(AutoRecoverMode::On);
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(11));
    state.record_remediation("AA:BB:CC:DD:EE:FF", now);
    state.record_seqnum_failure(now);

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "test",
    )
    .unwrap();

    assert!(bluetooth.disconnects.borrow().is_empty());
    assert!(scheduler.is_empty());
}

#[test]
fn source_change_trigger_bypasses_probe_cooldown() {
    let mut responses = hfp_probe_responses();
    responses.extend(hfp_probe_responses());
    let fake = FakePactl::new_owned(responses);
    let probe = QueueProbe::new(vec![Some(vec![1u8; 16000]), Some(vec![1u8; 16000])]);
    let config = remediation_config(AutoRecoverMode::DryRun);
    let bluetooth = FakeBluetooth::new();
    let mut scheduler = Scheduler::new(config.reconnect_backoff.clone());
    let mut state = ProbeState::new();
    let now = std::time::Instant::now();
    state.record_hfp_seen("AA:BB:CC:DD:EE:FF", now - Duration::from_secs(11));

    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "startup",
    )
    .unwrap();
    reconcile_with_reconnect(
        &fake,
        &probe,
        &mut state,
        &bluetooth,
        &mut scheduler,
        &|| false,
        &|| {},
        &config,
        "Event 'change' on source #160",
    )
    .unwrap();

    assert_eq!(probe.calls.borrow().len(), 2);
}
