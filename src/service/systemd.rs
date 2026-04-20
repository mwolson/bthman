use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use tracing::info;

use crate::service;
use crate::service_files;

pub fn install() -> Result<()> {
    if service::is_root() {
        bail!("systemd user install must not be run as root");
    }
    let unit_dir = user_unit_dir()?;
    fs::create_dir_all(&unit_dir).with_context(|| format!("creating {}", unit_dir.display()))?;
    let unit_path = unit_dir.join("bthman.service");
    let unit = service::render_template(service_files::SYSTEMD_UNIT, "%h/.local/bin/bthman")?;
    fs::write(&unit_path, unit).with_context(|| format!("writing {}", unit_path.display()))?;
    info!("Wrote {}", unit_path.display());
    run(&["systemctl", "--user", "daemon-reload"])?;
    run(&["systemctl", "--user", "enable", "bthman.service"])?;
    info!("Enabled bthman.service (user). Start with: systemctl --user start bthman.service");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if service::is_root() {
        bail!("systemd user uninstall must not be run as root");
    }
    let _ = run(&["systemctl", "--user", "stop", "bthman.service"]);
    let _ = run(&["systemctl", "--user", "disable", "bthman.service"]);
    let unit_path = user_unit_dir()?.join("bthman.service");
    if unit_path.exists() {
        fs::remove_file(&unit_path).with_context(|| format!("removing {}", unit_path.display()))?;
        info!("Removed {}", unit_path.display());
    }
    let _ = run(&["systemctl", "--user", "daemon-reload"]);
    Ok(())
}

fn user_unit_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join("systemd").join("user"));
    }
    let home = std::env::var_os("HOME").context("HOME env var not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user"))
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
