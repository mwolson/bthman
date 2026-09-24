use bthman::log_watcher::{concealment_logging_seen, read_events, LogEvent, LogWatcher};
use bthman::sco_probe::{ConcealmentTracker, CONCEALMENT_MIN_BLOCKS};
use crossbeam_channel::unbounded;

const PLC_LINE: &str =
    "wireplumber[1]: D spa.bluez5.source.media [media-source.c:569:produce_plc_data]: 0x1: produced PLC audio, frames:180\n";

#[test]
fn concealment_lines_below_threshold_emit_no_event() {
    let (tx, rx) = unbounded();
    let input = PLC_LINE.repeat(CONCEALMENT_MIN_BLOCKS - 1);
    read_events(input.as_bytes(), tx, &ConcealmentTracker::default());
    assert!(rx.try_recv().is_err());
}

#[test]
fn sustained_concealment_emits_one_event() {
    let (tx, rx) = unbounded();
    let tracker = ConcealmentTracker::default();
    let input = PLC_LINE.repeat(CONCEALMENT_MIN_BLOCKS * 2);
    read_events(input.as_bytes(), tx, &tracker);
    assert_eq!(rx.try_recv().unwrap(), LogEvent::ConcealmentSustained);
    assert!(rx.try_recv().is_err());
    assert_eq!(
        tracker.recent_blocks(std::time::Instant::now()),
        CONCEALMENT_MIN_BLOCKS * 2
    );
}

#[test]
fn unrelated_line_emits_no_event() {
    let (tx, rx) = unbounded();
    read_events(
        "wireplumber[1]: unrelated\n".as_bytes(),
        tx,
        &ConcealmentTracker::default(),
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn concealment_logging_seen_accepts_topic_debug_lines() {
    assert!(concealment_logging_seen(
        r#"{"PRIORITY":"7","MESSAGE":"D spa.bluez5.source.media [media-source.c:2036:transport_state_changed]: ready"}"#
    ));
    assert!(concealment_logging_seen(
        "Sep 24 host wireplumber[1]: D spa.bluez5.source.media [decode-buffer.h:1:x]: level\n"
    ));
}

#[test]
fn concealment_logging_seen_rejects_other_topics_and_levels() {
    assert!(!concealment_logging_seen(
        r#"{"PRIORITY":"6","MESSAGE":"I spa.bluez5: failed to set BT_PKT_SEQNUM"}"#
    ));
    assert!(!concealment_logging_seen(
        r#"{"PRIORITY":"7","MESSAGE":"D spa.bluez5.sink.media: ready"}"#
    ));
    assert!(!concealment_logging_seen(""));
}

#[test]
fn spawn_returns_err_when_journalctl_is_missing() {
    let Err(err) =
        LogWatcher::spawn_with_command("/no/such/journalctl", ConcealmentTracker::default())
    else {
        panic!("expected missing journalctl to fail");
    };
    assert!(format!("{:#}", err).contains("spawning journalctl"));
}
