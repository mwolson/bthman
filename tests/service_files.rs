use bthman::service_files::{OPENRC_SYSTEM, OPENRC_USER, SYSTEMD_UNIT};

#[test]
fn systemd_unit_matches_repo_file() {
    let on_disk = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/systemd/bthman.service"
    ))
    .unwrap();
    assert_eq!(SYSTEMD_UNIT, on_disk);
}

#[test]
fn openrc_user_matches_repo_file() {
    let on_disk =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/openrc-user/bthman"))
            .unwrap();
    assert_eq!(OPENRC_USER, on_disk);
}

#[test]
fn openrc_system_matches_repo_file() {
    let on_disk =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/openrc-system/bthman"))
            .unwrap();
    assert_eq!(OPENRC_SYSTEM, on_disk);
}

#[test]
fn systemd_contains_execstart_and_execreload() {
    assert!(SYSTEMD_UNIT.contains("ExecStart="));
    assert!(SYSTEMD_UNIT.contains("ExecReload="));
}

#[test]
fn openrc_uses_supervise_daemon() {
    assert!(OPENRC_USER.contains("supervise-daemon"));
    assert!(OPENRC_SYSTEM.contains("supervise-daemon"));
}

#[test]
fn openrc_defines_command() {
    assert!(OPENRC_USER.contains("command="));
    assert!(OPENRC_SYSTEM.contains("command="));
}
