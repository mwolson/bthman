use std::cell::RefCell;
use std::collections::VecDeque;

use anyhow::Result;
use bthman::cli::Overrides;
use bthman::config::Config;
use bthman::pactl::PactlRunner;
use bthman::reconcile::reconcile_with;

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
        self.calls.borrow_mut().push(args_owned);
        String::new()
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
    reconcile_with(&fake, &|| true, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &|| false, &|| {}, &config, "test").unwrap();
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
    reconcile_with(&fake, &|| false, &|| {}, &config, "test").unwrap();
    assert!(!fake.has_call(&["set-default-source", "bluez_input.AA:BB:CC:DD:EE:FF"]));
}
