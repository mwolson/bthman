use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use bthman::sco_probe::{Capture, ProbeRunner, RealProbe, PROBE_GRACE};

const SOURCE: &str = "bluez_input.AA:BB:CC:DD:EE:FF";

// One test per binary: it points PATH at a fake parecord, which is process-wide.
#[test]
fn real_probe_classifies_fake_parecord_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", dir.path().display(), path));
    let duration = Duration::from_millis(200);

    write_fake_parecord(dir.path(), "exec sleep 30");
    let started = Instant::now();
    assert_eq!(RealProbe.capture_raw(SOURCE, duration), Capture::NoData);
    assert!(started.elapsed() < duration + PROBE_GRACE + Duration::from_secs(2));

    write_fake_parecord(dir.path(), "head -c 6400 /dev/zero; exec sleep 30");
    assert_eq!(
        RealProbe.capture_raw(SOURCE, duration),
        Capture::Data(vec![0u8; 6400])
    );

    write_fake_parecord(dir.path(), "exit 1");
    assert_eq!(RealProbe.capture_raw(SOURCE, duration), Capture::Failed);
}

fn write_fake_parecord(dir: &Path, body: &str) {
    let script = dir.join("parecord");
    fs::write(&script, format!("#!/bin/sh\n{}\n", body)).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
}
