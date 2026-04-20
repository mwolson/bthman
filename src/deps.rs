use std::path::PathBuf;

use anyhow::{bail, Result};
use tracing::warn;

pub fn check_required() -> Result<()> {
    let missing: Vec<&str> = ["pactl", "wpctl"]
        .into_iter()
        .filter(|c| which(c).is_none())
        .collect();
    if !missing.is_empty() {
        for cmd in &missing {
            eprintln!("Error: '{}' is required but not found in PATH.", cmd);
        }
        bail!("missing required commands");
    }
    for cmd in ["bluetoothctl", "dbus-monitor"] {
        if which(cmd).is_none() {
            warn!(
                "{} not found in PATH; suspend-resume reconnect will be degraded",
                cmd
            );
        }
    }
    Ok(())
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(cmd);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            return metadata.is_file() && metadata.permissions().mode() & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        return path.is_file();
    }
    #[cfg(unix)]
    false
}
