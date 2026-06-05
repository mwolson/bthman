use bthman::log_watcher::{level_adequate, read_events, LogEvent, LogWatcher};
use crossbeam_channel::unbounded;

#[test]
fn seqnum_line_emits_event() {
    let (tx, rx) = unbounded();
    read_events(
        "wireplumber[1]: failed to set BT_PKT_SEQNUM: Protocol not available\n".as_bytes(),
        tx,
    );
    assert_eq!(rx.try_recv().unwrap(), LogEvent::SeqnumFailure);
    assert!(rx.try_recv().is_err());
}

#[test]
fn unrelated_line_emits_no_event() {
    let (tx, rx) = unbounded();
    read_events("wireplumber[1]: unrelated\n".as_bytes(), tx);
    assert!(rx.try_recv().is_err());
}

#[test]
fn level_probe_accepts_info_or_debug() {
    assert!(level_adequate(
        r#"{"PRIORITY":"6","MESSAGE":"s-node: saving stream props"}"#
    ));
    assert!(level_adequate(
        r#"{"PRIORITY":"7","MESSAGE":"spa.bluez5: ready"}"#
    ));
    assert!(level_adequate(
        "Apr 26 host wireplumber[1]: I spa.bluez5: ready\n"
    ));
    assert!(level_adequate(
        "Apr 26 host wireplumber[1]: D spa.bluez5: ready\n"
    ));
}

#[test]
fn level_probe_rejects_warn_and_error_only() {
    assert!(!level_adequate(
        r#"{"PRIORITY":"4","MESSAGE":"spa.bluez5: warn"}
{"PRIORITY":"3","MESSAGE":"spa.bluez5: error"}"#
    ));
    assert!(!level_adequate(
        "Apr 26 host wireplumber[1]: W spa.bluez5: warn\nApr 26 host wireplumber[1]: E spa.bluez5: error\n"
    ));
}

#[test]
fn spawn_returns_err_when_journalctl_is_missing() {
    let Err(err) = LogWatcher::spawn_with_command("/no/such/journalctl") else {
        panic!("expected missing journalctl to fail");
    };
    assert!(format!("{:#}", err).contains("spawning journalctl"));
}
