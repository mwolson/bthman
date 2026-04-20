use bthman::events::{is_interesting, PactlEvent};
use bthman::pactl::{get_active_profile, has_profile};

const CARDS_DUMP: &str = "Card #42
\tName: bluez_card.AA_BB_CC_DD_EE_FF
\tDriver: module-bluez5-device.c
\tProfiles:
\t\theadset-head-unit: Headset Head Unit (HSP/HFP) (codec LC3-24kHz)
\t\theadset-head-unit-msbc: Headset Head Unit (HSP/HFP) (codec mSBC)
\t\toff: Off
\tActive Profile: headset-head-unit-msbc
Card #43
\tName: bluez_card.11_22_33_44_55_66
\tActive Profile: off
";

#[test]
fn parse_event_card_change() {
    let line = r#"{"event":"change","on":"card","index":42}"#;
    let event = PactlEvent::parse(line).unwrap();
    assert_eq!(event.event, "change");
    assert_eq!(event.on, "card");
    assert_eq!(event.index, Some(42));
}

#[test]
fn parse_event_server_without_index() {
    let line = r#"{"event":"change","on":"server"}"#;
    let event = PactlEvent::parse(line).unwrap();
    assert_eq!(event.on, "server");
    assert_eq!(event.index, None);
}

#[test]
fn parse_event_malformed_returns_none() {
    assert!(PactlEvent::parse("not json").is_none());
    assert!(PactlEvent::parse("").is_none());
}

#[test]
fn event_key_tuples_match_and_differ() {
    let a = PactlEvent::parse(r#"{"event":"change","on":"card","index":1}"#).unwrap();
    let b = PactlEvent::parse(r#"{"event":"change","on":"card","index":1}"#).unwrap();
    let c = PactlEvent::parse(r#"{"event":"change","on":"card","index":2}"#).unwrap();
    assert_eq!(a.key(), b.key());
    assert_ne!(a.key(), c.key());
}

#[test]
fn event_formatted_includes_index_when_present() {
    let with = PactlEvent::parse(r#"{"event":"change","on":"card","index":7}"#).unwrap();
    let without = PactlEvent::parse(r#"{"event":"change","on":"server"}"#).unwrap();
    assert_eq!(with.formatted(), "Event 'change' on card #7");
    assert_eq!(without.formatted(), "Event 'change' on server");
}

#[test]
fn is_interesting_card_and_server_only() {
    let card = PactlEvent::parse(r#"{"event":"change","on":"card","index":1}"#).unwrap();
    let server = PactlEvent::parse(r#"{"event":"change","on":"server"}"#).unwrap();
    let sink = PactlEvent::parse(r#"{"event":"change","on":"sink","index":1}"#).unwrap();
    assert!(is_interesting(&card));
    assert!(is_interesting(&server));
    assert!(!is_interesting(&sink));
}

#[test]
fn get_active_profile_for_known_card() {
    assert_eq!(
        get_active_profile(CARDS_DUMP, "bluez_card.AA_BB_CC_DD_EE_FF"),
        "headset-head-unit-msbc"
    );
    assert_eq!(
        get_active_profile(CARDS_DUMP, "bluez_card.11_22_33_44_55_66"),
        "off"
    );
}

#[test]
fn get_active_profile_for_unknown_card() {
    assert_eq!(get_active_profile(CARDS_DUMP, "bluez_card.none"), "");
}

#[test]
fn has_profile_detects_present_and_absent() {
    assert!(has_profile(
        CARDS_DUMP,
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "headset-head-unit"
    ));
    assert!(has_profile(
        CARDS_DUMP,
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "headset-head-unit-msbc"
    ));
    assert!(!has_profile(
        CARDS_DUMP,
        "bluez_card.AA_BB_CC_DD_EE_FF",
        "a2dp-sink"
    ));
    assert!(!has_profile(
        CARDS_DUMP,
        "bluez_card.11_22_33_44_55_66",
        "headset-head-unit"
    ));
}
