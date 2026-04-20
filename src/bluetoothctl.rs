use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tracing::info;

pub fn list_paired_devices() -> Vec<String> {
    let Ok(output) = Command::new("bluetoothctl")
        .args(["devices", "Paired"])
        .output()
    else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0] == "Device" {
                Some(parts[1].to_string())
            } else {
                None
            }
        })
        .collect()
}

pub fn device_info(addr: &str) -> Option<String> {
    let output = Command::new("bluetoothctl")
        .args(["info", addr])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn is_audio_device(info: &str) -> bool {
    for marker in [
        "Audio Sink",
        "Handsfree",
        "audio-headphones",
        "audio-headset",
    ] {
        if info.contains(marker) {
            return true;
        }
    }
    false
}

pub fn device_is_connected(addr: &str) -> bool {
    device_info(addr)
        .map(|info| info.contains("Connected: yes"))
        .unwrap_or(false)
}

pub fn snapshot_connected_audio_devices() -> HashSet<String> {
    let mut connected = HashSet::new();
    for addr in list_paired_devices() {
        let Some(info) = device_info(&addr) else {
            continue;
        };
        if !is_audio_device(&info) {
            continue;
        }
        if info.contains("Connected: yes") {
            connected.insert(addr);
        }
    }
    connected
}

pub fn try_reconnect(addr: &str, timeout: Duration) -> bool {
    let mut child = match Command::new("bluetoothctl")
        .args(["connect", addr])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {}
            Err(_) => return false,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            info!("Reconnect to {} timed out", addr);
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
