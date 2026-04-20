use std::collections::HashSet;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use tracing::info;

pub trait PactlRunner {
    fn run(&self, args: &[&str]) -> Result<String>;
    fn run_ok(&self, args: &[&str]) -> String;
}

pub struct RealPactl;

impl PactlRunner for RealPactl {
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("pactl")
            .args(args)
            .output()
            .with_context(|| format!("running pactl {}", args.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "pactl {} failed: {}",
                args.join(" "),
                stderr.trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn run_ok(&self, args: &[&str]) -> String {
        match Command::new("pactl").args(args).output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
            Err(_) => String::new(),
        }
    }
}

pub fn list_bluetooth_cards(runner: &dyn PactlRunner) -> Result<Vec<String>> {
    let out = runner.run(&["list", "cards", "short"])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            fields
                .get(1)
                .filter(|n| n.starts_with("bluez_card."))
                .map(|s| s.to_string())
        })
        .collect())
}

pub fn get_active_profile(cards_dump: &str, card_name: &str) -> String {
    let mut found = false;
    for line in cards_dump.lines() {
        if let Some(name) = line.strip_prefix("\tName: ") {
            found = name == card_name;
            continue;
        }
        if found {
            if let Some((_, rest)) = line.split_once("Active Profile: ") {
                return rest.trim().to_string();
            }
        }
    }
    String::new()
}

pub fn has_profile(cards_dump: &str, card_name: &str, profile_name: &str) -> bool {
    let needle = format!("{}:", profile_name);
    let mut found = false;
    for line in cards_dump.lines() {
        if line.starts_with("Card #") {
            found = false;
            continue;
        }
        if let Some(name) = line.strip_prefix("\tName: ") {
            found = name == card_name;
            continue;
        }
        if found && line.trim_start().starts_with(&needle) {
            return true;
        }
    }
    false
}

pub fn prefer_headset_profile(
    runner: &dyn PactlRunner,
    cards_dump: &str,
    card_name: &str,
    preferred: &[String],
) -> Result<bool> {
    let active = get_active_profile(cards_dump, card_name);
    if active.is_empty() {
        info!("{}: active profile unavailable", card_name);
        return Ok(false);
    }
    info!("{}: active profile {}", card_name, active);
    if !active.starts_with("headset-head-unit") {
        return Ok(false);
    }
    for want in preferred {
        if active == *want {
            return Ok(false);
        }
        if has_profile(cards_dump, card_name, want) {
            info!("Switching {} from {} to {}", card_name, active, want);
            runner.run(&["set-card-profile", card_name, want])?;
            return Ok(true);
        }
    }
    info!("{}: no preferred profile available", card_name);
    Ok(false)
}

pub fn fix_default_source(
    runner: &dyn PactlRunner,
    cards_dump: &str,
    input_volume: u32,
) -> Result<bool> {
    let info_out = runner.run(&["info"])?;
    let default_source = info_out
        .lines()
        .find_map(|line| line.strip_prefix("Default Source: "))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if default_source.is_empty() {
        info!("Default source unavailable");
        return Ok(false);
    }

    let mut preferred = find_headset_source(runner, cards_dump)?;
    if preferred.is_none() {
        let monitor_re = Regex::new(r"^bluez_output\.(.+)\.monitor$").expect("monitor regex");
        if let Some(caps) = monitor_re.captures(&default_source) {
            preferred = Some(format!("bluez_input.{}", &caps[1]));
        }
    }

    let Some(preferred) = preferred else {
        info!("Default source OK: {}", default_source);
        return Ok(false);
    };

    if preferred == default_source {
        info!("Default source OK: {}", default_source);
        if preferred.starts_with("bluez_input.") {
            fix_source_volume(runner, &preferred, input_volume);
        }
        return Ok(false);
    }

    let sources = runner.run(&["list", "short", "sources"])?;
    let source_names: HashSet<&str> = sources
        .lines()
        .filter_map(|line| line.split('\t').nth(1))
        .collect();
    if !source_names.contains(preferred.as_str()) {
        info!(
            "Want to switch default source to {}, but it is unavailable",
            preferred
        );
        return Ok(false);
    }

    info!(
        "Switching default source from {} to {}",
        default_source, preferred
    );
    runner.run(&["set-default-source", &preferred])?;
    fix_source_volume(runner, &preferred, input_volume);
    Ok(true)
}

pub fn fix_source_volume(runner: &dyn PactlRunner, source: &str, target: u32) {
    let volume_out = runner.run_ok(&["get-source-volume", source]);
    let target_str = format!("{}%", target);
    if volume_out.contains(&target_str) {
        return;
    }
    info!("Setting {} volume to {}", source, target_str);
    let _ = runner.run_ok(&["set-source-volume", source, &target_str]);
}

pub fn find_headset_source(runner: &dyn PactlRunner, cards_dump: &str) -> Result<Option<String>> {
    for card in list_bluetooth_cards(runner)? {
        let active = get_active_profile(cards_dump, &card);
        if active.is_empty() || !active.starts_with("headset-head-unit") {
            continue;
        }
        let addr = card
            .strip_prefix("bluez_card.")
            .unwrap_or(&card)
            .replace('_', ":");
        return Ok(Some(format!("bluez_input.{}", addr)));
    }
    Ok(None)
}
