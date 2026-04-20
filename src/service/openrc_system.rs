use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use tracing::info;

use crate::service;
use crate::service_files;

const INIT_PATH: &str = "/etc/init.d/bthman";

pub fn install() -> Result<()> {
    if !service::is_root() {
        bail!("OpenRC system install must be run as root");
    }
    fs::write(INIT_PATH, service_files::OPENRC_SYSTEM)
        .with_context(|| format!("writing {}", INIT_PATH))?;
    let mut perms = fs::metadata(INIT_PATH)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(INIT_PATH, perms)?;
    info!("Wrote {}", INIT_PATH);
    run(&["rc-update", "add", "bthman", "default"])?;
    info!("Enabled bthman (system). Start with: rc-service bthman start");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if !service::is_root() {
        bail!("OpenRC system uninstall must be run as root");
    }
    let _ = run(&["rc-service", "bthman", "stop"]);
    let _ = run(&["rc-update", "del", "bthman", "default"]);
    if Path::new(INIT_PATH).exists() {
        fs::remove_file(INIT_PATH).with_context(|| format!("removing {}", INIT_PATH))?;
        info!("Removed {}", INIT_PATH);
    }
    Ok(())
}

fn run(args: &[&str]) -> Result<()> {
    let status = Command::new(args[0])
        .args(&args[1..])
        .status()
        .with_context(|| format!("running {}", args.join(" ")))?;
    if !status.success() {
        bail!("{} failed with {}", args.join(" "), status);
    }
    Ok(())
}
