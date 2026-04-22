use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use bthman::cli::Overrides;
use bthman::config::Config;
use bthman::pactl::PactlRunner;
use bthman::reconcile::reconcile_with;
use bthman::sco_probe::ProbeRunner;

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

struct FakePactl {
    responses: RefCell<VecDeque<(Vec<String>, String)>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakePactl {
    fn new(responses: Vec<(Vec<&str>, &str)>) -> Self {
        Self {
            responses: RefCell::new(
                responses
                    .into_iter()
                    .map(|(args, body)| {
                        (
                            args.into_iter().map(String::from).collect(),
                            body.to_string(),
                        )
                    })
                    .collect(),
            ),
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

#[test]
fn skips_when_external_recorder_active() {
    let fake = FakePactl::new(vec![]);
    let config = test_config(vec!["headset-head-unit"]);
    reconcile_with(&fake, &NoProbe, &|| true, &|| {}, &config, "test").unwrap();
    assert!(fake.calls.borrow().is_empty());
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
    reconcile_with(&fake, &NoProbe, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &NoProbe, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &NoProbe, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &NoProbe, &|| false, &|| {}, &config, "test").unwrap();
    assert!(!fake.has_call(&["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"]));
}

#[test]
fn probe_skipped_when_no_source_outputs_present() {
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
            vec!["list", "short", "sources"],
            "77\tbluez_input.AA:BB:CC:DD:EE:FF\tmodule\n",
        ),
        (vec!["list", "short", "source-outputs"], ""),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(&fake, &probe, &|| false, &|| {}, &config, "test").unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "probe must not run when no source-outputs are present"
    );
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
    reconcile_with(&fake, &probe, &|| false, &|| {}, &config, "test").unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "probe must not run on non-HFP card"
    );
}

#[test]
fn probe_runs_when_hfp_active_with_recorder_and_detects_all_zero() {
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
            vec!["list", "short", "sources"],
            "77\tbluez_input.AA:BB:CC:DD:EE:FF\tmodule\n",
        ),
        (
            vec!["list", "short", "source-outputs"],
            "99\t77\t88\tPipeWire\ts16le 2ch 16000Hz\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(&fake, &probe, &|| false, &|| {}, &config, "test").unwrap();
    assert_eq!(
        probe.calls.borrow().as_slice(),
        &["bluez_input.AA:BB:CC:DD:EE:FF".to_string()]
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
            vec!["list", "short", "sources"],
            "77\tbluez_input.AA:BB:CC:DD:EE:FF\tmodule\n",
        ),
        (
            vec!["list", "short", "source-outputs"],
            "99\t77\t88\tPipeWire\ts16le 2ch 16000Hz\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: yes\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    let config = base_config(vec!["headset-head-unit"]);
    reconcile_with(&fake, &probe, &|| false, &|| {}, &config, "test").unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "probe must not run on muted source"
    );
}
