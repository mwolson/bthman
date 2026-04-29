use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;

use crate::cli::{AutoRecoverMode, Overrides};
use crate::vendor_detect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub preferred_profiles: Vec<String>,
    pub input_volume: u32,
    pub auto_recover_stuck_sco: AutoRecoverMode,
    pub broken_vendors: HashSet<String>,
    pub debounce: Duration,
    pub probe_stuck_sco: bool,
    pub reconnect_timeout: Duration,
    pub reconnect_backoff: Vec<Duration>,
    preferred_profiles_user_set: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preferred_profiles: vec!["headset-head-unit".into(), "headset-head-unit-msbc".into()],
            input_volume: 100,
            auto_recover_stuck_sco: AutoRecoverMode::Off,
            broken_vendors: ["0e8d".into()].into_iter().collect(),
            debounce: Duration::from_millis(500),
            probe_stuck_sco: true,
            reconnect_timeout: Duration::from_secs(8),
            reconnect_backoff: vec![
                Duration::from_millis(0),
                Duration::from_millis(500),
                Duration::from_millis(1500),
                Duration::from_millis(3500),
            ],
            preferred_profiles_user_set: false,
        }
    }
}

impl Config {
    pub fn build(overrides: &Overrides, conf_path: Option<&Path>) -> Result<Self> {
        let mut config = Self::default();
        if let Some(path) = conf_path {
            if path.exists() {
                config.apply_file(path)?;
            }
        }
        config.apply_overrides(overrides);
        if !config.preferred_profiles_user_set {
            config.preferred_profiles = vendor_detect::detect_preferred_profiles(
                &config.preferred_profiles,
                &config.broken_vendors,
            );
        }
        Ok(config)
    }

    pub fn apply_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let entries = parse_conf(&text, path)?;
        for (flag, value) in entries {
            self.apply_flag(&flag, &value, path)?;
        }
        Ok(())
    }

    pub fn apply_overrides(&mut self, overrides: &Overrides) {
        if let Some(profiles) = &overrides.preferred_profiles {
            self.preferred_profiles = profiles.clone();
            self.preferred_profiles_user_set = true;
        }
        if let Some(volume) = overrides.input_volume {
            self.input_volume = volume;
        }
        if let Some(mode) = overrides.auto_recover_stuck_sco {
            self.auto_recover_stuck_sco = mode;
        }
        if let Some(vendors) = &overrides.broken_vendors {
            self.broken_vendors = vendors.iter().map(|v| v.to_lowercase()).collect();
        }
        if let Some(ms) = overrides.debounce_ms {
            self.debounce = Duration::from_millis(ms);
        }
        if let Some(probe) = overrides.probe_stuck_sco {
            self.probe_stuck_sco = probe;
        }
    }

    fn apply_flag(&mut self, flag: &str, value: &str, path: &Path) -> Result<()> {
        match flag {
            "--broken-vendor" => {
                self.broken_vendors.insert(value.to_lowercase());
            }
            "--debounce-ms" => {
                let ms: u64 = value.parse().map_err(|_| {
                    anyhow!("invalid --debounce-ms in {}: {}", path.display(), value)
                })?;
                self.debounce = Duration::from_millis(ms);
            }
            "--input-volume" => {
                self.input_volume = value.parse().map_err(|_| {
                    anyhow!("invalid --input-volume in {}: {}", path.display(), value)
                })?;
            }
            "--auto-recover-stuck-sco" => {
                self.auto_recover_stuck_sco = parse_auto_recover_mode(value).ok_or_else(|| {
                    anyhow!(
                        "invalid --auto-recover-stuck-sco in {} (expected off/dry-run/on): {}",
                        path.display(),
                        value
                    )
                })?;
            }
            "--preferred-profile" => {
                if !self.preferred_profiles_user_set {
                    self.preferred_profiles.clear();
                    self.preferred_profiles_user_set = true;
                }
                self.preferred_profiles.push(value.to_string());
            }
            "--probe-stuck-sco" => {
                self.probe_stuck_sco = parse_bool(value).ok_or_else(|| {
                    anyhow!(
                        "invalid --probe-stuck-sco in {} (expected true/false): {}",
                        path.display(),
                        value
                    )
                })?;
            }
            _ => bail!("unsupported flag '{}' in {}", flag, path.display()),
        }
        Ok(())
    }
}

pub fn default_conf_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("bthman.conf"))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Some(true),
        "false" | "no" | "0" | "off" => Some(false),
        _ => None,
    }
}

fn parse_auto_recover_mode(value: &str) -> Option<AutoRecoverMode> {
    match value.to_lowercase().as_str() {
        "false" | "no" | "0" | "off" => Some(AutoRecoverMode::Off),
        "dry-run" | "dry_run" => Some(AutoRecoverMode::DryRun),
        "true" | "yes" | "1" | "on" => Some(AutoRecoverMode::On),
        _ => None,
    }
}

pub fn parse_conf(text: &str, path: &Path) -> Result<Vec<(String, String)>> {
    let line_re = Regex::new(r"^(--[a-z][a-z0-9-]*)=(.+)$").expect("conf regex");
    let mut entries = Vec::new();
    for (line_num, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let captures = line_re.captures(line).ok_or_else(|| {
            anyhow!(
                "malformed line {} in {}: {}",
                line_num + 1,
                path.display(),
                line
            )
        })?;
        entries.push((captures[1].to_string(), captures[2].to_string()));
    }
    Ok(entries)
}
