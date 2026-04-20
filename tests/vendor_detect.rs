use std::collections::HashSet;
use std::os::unix::fs::symlink;
use std::path::Path;

use bthman::vendor_detect::detect_in;
use tempfile::TempDir;

fn defaults() -> Vec<String> {
    vec![
        "headset-head-unit".to_string(),
        "headset-head-unit-msbc".to_string(),
    ]
}

fn broken() -> HashSet<String> {
    ["0e8d".to_string()].into_iter().collect()
}

fn make_adapter(root: &Path, name: &str, vendor: &str) {
    let hci = root.join("bluetooth").join(name);
    std::fs::create_dir_all(&hci).unwrap();
    let dev_parent = root.join(format!("parent-{}", name));
    let subdev = dev_parent.join("subdev");
    std::fs::create_dir_all(&subdev).unwrap();
    std::fs::write(dev_parent.join("idVendor"), format!("{}\n", vendor)).unwrap();
    symlink(&subdev, hci.join("device")).unwrap();
}

#[test]
fn missing_bluetooth_dir_returns_defaults() {
    let dir = TempDir::new().unwrap();
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, defaults());
}

#[test]
fn no_adapters_returns_defaults() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("bluetooth")).unwrap();
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, defaults());
}

#[test]
fn non_matching_vendor_keeps_lc3() {
    let dir = TempDir::new().unwrap();
    make_adapter(dir.path(), "hci0", "8087");
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, defaults());
}

#[test]
fn broken_vendor_skips_lc3() {
    let dir = TempDir::new().unwrap();
    make_adapter(dir.path(), "hci0", "0e8d");
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, vec!["headset-head-unit-msbc".to_string()]);
}

#[test]
fn missing_idvendor_keeps_lc3() {
    let dir = TempDir::new().unwrap();
    let hci = dir.path().join("bluetooth").join("hci0");
    std::fs::create_dir_all(&hci).unwrap();
    let dev_parent = dir.path().join("parent-hci0");
    let subdev = dev_parent.join("subdev");
    std::fs::create_dir_all(&subdev).unwrap();
    symlink(&subdev, hci.join("device")).unwrap();
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, defaults());
}

#[test]
fn multi_adapter_one_broken() {
    let dir = TempDir::new().unwrap();
    make_adapter(dir.path(), "hci0", "8087");
    make_adapter(dir.path(), "hci1", "0e8d");
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, vec!["headset-head-unit-msbc".to_string()]);
}

#[test]
fn mixed_case_vendor_matches() {
    let dir = TempDir::new().unwrap();
    make_adapter(dir.path(), "hci0", "0E8D");
    let result = detect_in(&dir.path().join("bluetooth"), &defaults(), &broken());
    assert_eq!(result, vec!["headset-head-unit-msbc".to_string()]);
}
