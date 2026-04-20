use bthman::sleep_monitor::{classify, SleepLine};

#[test]
fn header_detected() {
    assert_eq!(
        classify(
            "signal sender=:1.5 serial=12 path=/org/freedesktop/login1; \
             interface=org.freedesktop.login1.Manager; member=PrepareForSleep"
        ),
        Some(SleepLine::Header)
    );
}

#[test]
fn boolean_true_detected() {
    assert_eq!(classify("   boolean true"), Some(SleepLine::True));
    assert_eq!(classify("Boolean True"), Some(SleepLine::True));
}

#[test]
fn boolean_false_detected() {
    assert_eq!(classify("   boolean false"), Some(SleepLine::False));
    assert_eq!(classify("BOOLEAN FALSE"), Some(SleepLine::False));
}

#[test]
fn unrelated_line_returns_none() {
    assert_eq!(classify("some random line"), None);
    assert_eq!(classify(""), None);
    assert_eq!(classify("string \"foo\""), None);
}

#[test]
fn header_without_payload_match_is_still_header() {
    assert_eq!(
        classify("interface=... member=PrepareForSleep"),
        Some(SleepLine::Header)
    );
}
