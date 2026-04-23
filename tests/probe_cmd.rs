use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use bthman::pactl::PactlRunner;
use bthman::probe_cmd::run_manual_probe_with;
use bthman::sco_probe::ProbeRunner;

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
\t\toff: Off
\tActive Profile: {}
",
        active
    )
}

#[test]
fn explicit_source_is_probed_even_with_no_recorder_present() {
    // No source-outputs exist; auto-probe gating would have skipped this.
    // The manual command must still fire the probe.
    let fake = FakePactl::new(vec![(
        vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
        "Mute: no\n",
    )]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    run_manual_probe_with(
        &fake,
        &probe,
        Some("bluez_input.AA:BB:CC:DD:EE:FF"),
        Duration::from_millis(500),
    )
    .unwrap();
    assert_eq!(
        probe.calls.borrow().as_slice(),
        &["bluez_input.AA:BB:CC:DD:EE:FF".to_string()]
    );
    assert!(
        !fake.has_call(&["list", "short", "source-outputs"]),
        "manual probe must not consult source-output count"
    );
}

#[test]
fn explicit_source_is_skipped_when_muted() {
    let fake = FakePactl::new(vec![(
        vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
        "Mute: yes\n",
    )]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    run_manual_probe_with(
        &fake,
        &probe,
        Some("bluez_input.AA:BB:CC:DD:EE:FF"),
        Duration::from_millis(500),
    )
    .unwrap();
    assert!(
        probe.calls.borrow().is_empty(),
        "muted sources would always capture zero; probe would be a false positive"
    );
}

#[test]
fn auto_discovery_probes_every_hfp_active_card() {
    let cards = "Card #1
\tName: bluez_card.AA_BB_CC_DD_EE_FF
\tActive Profile: headset-head-unit
Card #2
\tName: bluez_card.11_22_33_44_55_66
\tActive Profile: headset-head-unit-msbc
";
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], cards),
        (
            vec!["list", "cards", "short"],
            "1\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n2\tbluez_card.11_22_33_44_55_66\tmodule\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.AA:BB:CC:DD:EE:FF"],
            "Mute: no\n",
        ),
        (
            vec!["get-source-mute", "bluez_input.11:22:33:44:55:66"],
            "Mute: no\n",
        ),
    ]);
    let mut buf = vec![0u8; 16000];
    buf[100] = 5;
    let probe = RecordingProbe::new(Some(buf));
    run_manual_probe_with(&fake, &probe, None, Duration::from_millis(500)).unwrap();
    let calls = probe.calls.borrow();
    assert!(calls.contains(&"bluez_input.AA:BB:CC:DD:EE:FF".to_string()));
    assert!(calls.contains(&"bluez_input.11:22:33:44:55:66".to_string()));
    assert_eq!(calls.len(), 2);
}

#[test]
fn auto_discovery_skips_non_hfp_cards() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], &cards_dump("a2dp-sink")),
        (
            vec!["list", "cards", "short"],
            "42\tbluez_card.AA_BB_CC_DD_EE_FF\tmodule\n",
        ),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    run_manual_probe_with(&fake, &probe, None, Duration::from_millis(500)).unwrap();
    assert!(probe.calls.borrow().is_empty());
}

#[test]
fn auto_discovery_with_no_bluez_cards_is_a_noop() {
    let fake = FakePactl::new(vec![
        (vec!["list", "cards"], ""),
        (vec!["list", "cards", "short"], ""),
    ]);
    let probe = RecordingProbe::new(Some(vec![0u8; 16000]));
    run_manual_probe_with(&fake, &probe, None, Duration::from_millis(500)).unwrap();
    assert!(probe.calls.borrow().is_empty());
}
