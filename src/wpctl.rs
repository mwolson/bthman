use std::process::Command;

pub fn external_recorder_active() -> bool {
    let Ok(output) = Command::new("wpctl")
        .args(["settings", "bluetooth.autoswitch-to-headset-profile"])
        .output()
    else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
    stdout.contains("false")
}
