use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, info};

use crate::config::Config;
use crate::pactl::{self, PactlRunner, RealPactl};
use crate::sco_probe::{
    self, ProbeRunner, ProbeState, RealProbe, DEFAULT_PROBE_COOLDOWN, DEFAULT_PROBE_DURATION,
};
use crate::wpctl;

pub fn reconcile(config: &Config, trigger: &str) -> Result<()> {
    let mut state = ProbeState::new();
    reconcile_persistent(config, trigger, &mut state)
}

pub fn reconcile_persistent(
    config: &Config,
    trigger: &str,
    probe_state: &mut ProbeState,
) -> Result<()> {
    reconcile_with(
        &RealPactl,
        &RealProbe,
        probe_state,
        &wpctl::external_recorder_active,
        &|| thread::sleep(Duration::from_secs(1)),
        config,
        trigger,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn reconcile_with(
    runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    probe_state: &mut ProbeState,
    recorder_active: &dyn Fn() -> bool,
    post_change: &dyn Fn(),
    config: &Config,
    trigger: &str,
) -> Result<()> {
    info!("Reconciling: {}", trigger);
    if recorder_active() {
        info!("External recorder active, skipping reconciliation");
        return Ok(());
    }
    let cards_dump = runner.run(&["list", "cards"])?;
    let mut changed = false;
    for card in pactl::list_bluetooth_cards(runner)? {
        if pactl::prefer_headset_profile(runner, &cards_dump, &card, &config.preferred_profiles)? {
            changed = true;
        }
    }
    if changed {
        post_change();
    }
    if pactl::fix_default_source(runner, &cards_dump, config.input_volume)? {
        changed = true;
    }
    if !changed {
        info!("No changes needed");
    }
    if config.probe_stuck_sco {
        probe_bluetooth_sources(runner, probe, probe_state, &cards_dump)?;
    }
    Ok(())
}

fn probe_bluetooth_sources(
    runner: &dyn PactlRunner,
    probe: &dyn ProbeRunner,
    probe_state: &mut ProbeState,
    cards_dump: &str,
) -> Result<()> {
    for card in pactl::list_bluetooth_cards(runner)? {
        let active = pactl::get_active_profile(cards_dump, &card);
        if !active.starts_with("headset-head-unit") {
            continue;
        }
        let addr = card
            .strip_prefix("bluez_card.")
            .unwrap_or(&card)
            .replace('_', ":");
        let source = format!("bluez_input.{}", addr);
        if pactl::source_is_muted(runner, &source) {
            continue;
        }
        if !probe_state.should_probe(&source, Instant::now(), DEFAULT_PROBE_COOLDOWN) {
            debug!("Probe skipped (cooldown): {}", source);
            continue;
        }
        let result = sco_probe::probe_source(probe, &source, DEFAULT_PROBE_DURATION);
        sco_probe::log_result(&source, &result);
    }
    Ok(())
}
