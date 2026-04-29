use std::path::Path;
use std::time::Duration;

use bthman::cli::{AutoRecoverMode, Overrides};
use bthman::config::{parse_conf, Config};
use tempfile::TempDir;

#[test]
fn parse_conf_accepts_values_blanks_and_comments() {
    let text = "\
# leading comment
--preferred-profile=headset-head-unit-msbc
--input-volume=85

# another comment
--broken-vendor=0e8d
--auto-recover-stuck-sco=dry-run
--debounce-ms=250
";
    let entries = parse_conf(text, Path::new("conf")).unwrap();
    assert_eq!(
        entries,
        vec![
            (
                "--preferred-profile".into(),
                "headset-head-unit-msbc".into()
            ),
            ("--input-volume".into(), "85".into()),
            ("--broken-vendor".into(), "0e8d".into()),
            ("--auto-recover-stuck-sco".into(), "dry-run".into()),
            ("--debounce-ms".into(), "250".into()),
        ]
    );
}

#[test]
fn parse_conf_rejects_malformed_line() {
    let err = parse_conf("--bare-flag\n", Path::new("conf")).unwrap_err();
    assert!(format!("{}", err).contains("malformed"));
}

#[test]
fn config_build_applies_file_values() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(
        &conf,
        "--input-volume=80\n--preferred-profile=headset-head-unit-msbc\n--debounce-ms=250\n",
    )
    .unwrap();
    let config = Config::build(&Overrides::default(), Some(&conf)).unwrap();
    assert_eq!(config.input_volume, 80);
    assert_eq!(
        config.preferred_profiles,
        vec!["headset-head-unit-msbc".to_string()]
    );
    assert_eq!(config.debounce, Duration::from_millis(250));
}

#[test]
fn config_build_cli_overrides_file() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(
        &conf,
        "--input-volume=80\n--preferred-profile=headset-head-unit-msbc\n",
    )
    .unwrap();
    let overrides = Overrides {
        input_volume: Some(95),
        ..Default::default()
    };
    let config = Config::build(&overrides, Some(&conf)).unwrap();
    assert_eq!(config.input_volume, 95);
    assert_eq!(
        config.preferred_profiles,
        vec!["headset-head-unit-msbc".to_string()]
    );
}

#[test]
fn config_build_repeated_preferred_profile() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(
        &conf,
        "--preferred-profile=a\n--preferred-profile=b\n--preferred-profile=c\n",
    )
    .unwrap();
    let config = Config::build(&Overrides::default(), Some(&conf)).unwrap();
    assert_eq!(
        config.preferred_profiles,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

#[test]
fn config_build_unknown_flag_errors() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(&conf, "--unknown=1\n").unwrap();
    let err = Config::build(&Overrides::default(), Some(&conf)).unwrap_err();
    assert!(format!("{}", err).contains("unsupported"));
}

#[test]
fn config_build_invalid_volume_errors() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(&conf, "--input-volume=banana\n").unwrap();
    let err = Config::build(&Overrides::default(), Some(&conf)).unwrap_err();
    assert!(format!("{}", err).contains("invalid --input-volume"));
}

#[test]
fn config_build_broken_vendor_lowercased() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(
        &conf,
        "--preferred-profile=headset-head-unit-msbc\n--broken-vendor=0E8D\n",
    )
    .unwrap();
    let config = Config::build(&Overrides::default(), Some(&conf)).unwrap();
    assert!(config.broken_vendors.contains("0e8d"));
}

#[test]
fn config_build_missing_file_uses_defaults() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("does-not-exist.conf");
    let overrides = Overrides {
        preferred_profiles: Some(vec!["headset-head-unit-msbc".to_string()]),
        ..Default::default()
    };
    let config = Config::build(&overrides, Some(&conf)).unwrap();
    assert_eq!(config.input_volume, 100);
    assert_eq!(config.debounce, Duration::from_millis(500));
}

#[test]
fn config_build_parses_auto_recover_mode() {
    let dir = TempDir::new().unwrap();
    let conf = dir.path().join("bthman.conf");
    std::fs::write(&conf, "--auto-recover-stuck-sco=dry-run\n").unwrap();
    let config = Config::build(&Overrides::default(), Some(&conf)).unwrap();
    assert_eq!(config.auto_recover_stuck_sco, AutoRecoverMode::DryRun);

    let overrides = Overrides {
        auto_recover_stuck_sco: Some(AutoRecoverMode::On),
        ..Default::default()
    };
    let config = Config::build(&overrides, Some(&conf)).unwrap();
    assert_eq!(config.auto_recover_stuck_sco, AutoRecoverMode::On);
}
