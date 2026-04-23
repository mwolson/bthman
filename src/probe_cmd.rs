use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::pactl::{self, PactlRunner, RealPactl};
use crate::sco_probe::{self, ProbeRunner, RealProbe, DEFAULT_PROBE_DURATION};

pub fn run_manual_probe(source: Option<&str>, duration_ms: Option<u64>) -> Result<()> {
    let duration = duration_ms
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_PROBE_DURATION);
    run_manual_probe_with(&RealPactl, &RealProbe, source, duration)
}

pub fn run_manual_probe_with(
    pactl_runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    source: Option<&str>,
    duration: Duration,
) -> Result<()> {
    let targets = match source {
        Some(s) => vec![s.to_string()],
        None => discover_hfp_sources(pactl_runner)?,
    };

    if targets.is_empty() {
        info!("No HFP-active Bluetooth sources to probe");
        return Ok(());
    }

    for target in &targets {
        if pactl::source_is_muted(pactl_runner, target) {
            info!("Probe skipped (muted): {}", target);
            continue;
        }
        let result = sco_probe::probe_source(probe, target, duration);
        sco_probe::log_result(target, &result);
    }
    Ok(())
}

fn discover_hfp_sources(runner: &dyn PactlRunner) -> Result<Vec<String>> {
    let cards_dump = runner.run(&["list", "cards"])?;
    let mut sources = Vec::new();
    for card in pactl::list_bluetooth_cards(runner)? {
        let active = pactl::get_active_profile(&cards_dump, &card);
        if !active.starts_with("headset-head-unit") {
            continue;
        }
        let addr = card
            .strip_prefix("bluez_card.")
            .unwrap_or(&card)
            .replace('_', ":");
        sources.push(format!("bluez_input.{}", addr));
    }
    Ok(sources)
}
