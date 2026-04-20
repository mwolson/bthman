use std::thread;
use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::pactl::{self, PactlRunner, RealPactl};
use crate::wpctl;

pub fn reconcile(config: &Config, trigger: &str) -> Result<()> {
    reconcile_with(
        &RealPactl,
        &wpctl::external_recorder_active,
        &|| thread::sleep(Duration::from_secs(1)),
        config,
        trigger,
    )
}

pub fn reconcile_with(
    runner: &dyn PactlRunner,
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
    Ok(())
}
