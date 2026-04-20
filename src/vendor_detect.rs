use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::info;

pub fn detect_preferred_profiles(defaults: &[String], broken: &HashSet<String>) -> Vec<String> {
    detect_in(Path::new("/sys/class/bluetooth"), defaults, broken)
}

pub fn detect_in(bt_class: &Path, defaults: &[String], broken: &HashSet<String>) -> Vec<String> {
    if !bt_class.is_dir() {
        return defaults.to_vec();
    }
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(bt_class) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => return defaults.to_vec(),
    };
    entries.sort();
    for hci_dir in entries {
        let vendor_file = hci_dir.join("device").join("..").join("idVendor");
        let Ok(contents) = std::fs::read_to_string(&vendor_file) else {
            continue;
        };
        let vendor_id = contents.trim().to_lowercase();
        if broken.contains(&vendor_id) {
            let name = hci_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            info!(
                "{}: USB vendor {} has broken LC3 HFP, preferring mSBC",
                name, vendor_id
            );
            return defaults
                .iter()
                .filter(|p| p.as_str() != "headset-head-unit")
                .cloned()
                .collect();
        }
    }
    defaults.to_vec()
}
