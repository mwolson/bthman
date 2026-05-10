use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AutoRecoverMode {
    Off,
    DryRun,
    On,
}

impl AutoRecoverMode {
    pub fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn action(self) -> &'static str {
        match self {
            Self::Off => "log_only",
            Self::DryRun => "would_remediate",
            Self::On => "remediate",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "bthman",
    version,
    about = "Manage Bluetooth HFP profile selection and reconnect headsets after resume"
)]
pub struct Cli {
    /// Preferred HFP profile in priority order (repeatable)
    #[arg(long = "preferred-profile", value_name = "PROFILE")]
    pub preferred_profile: Vec<String>,

    /// Mic volume target percent (default 100)
    #[arg(long = "input-volume", value_name = "N")]
    pub input_volume: Option<u32>,

    /// USB vendor IDs for which to skip LC3 HFP (repeatable, hex)
    #[arg(long = "broken-vendor", value_name = "HEX")]
    pub broken_vendor: Vec<String>,

    /// Event debounce window in milliseconds (default 500)
    #[arg(long = "debounce-ms", value_name = "N")]
    pub debounce_ms: Option<u64>,

    /// Probe bluetooth HFP sources for stuck-SCO silence (default true)
    #[arg(long = "probe-stuck-sco", value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub probe_stuck_sco: Option<bool>,

    /// Auto-recover confirmed stuck-SCO by disconnecting and reconnecting the device
    #[arg(long = "auto-recover-stuck-sco", value_name = "MODE")]
    pub auto_recover_stuck_sco: Option<AutoRecoverMode>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install and enable the service (systemd or OpenRC)
    InstallService,
    /// Reconcile profiles once and exit
    Once,
    /// Probe Bluetooth HFP sources for stuck-SCO silence and exit
    ///
    /// Bypasses the daemon's auto-probe gating: fires even when no application
    /// has an active recording stream on the source, and even when the
    /// cooldown window has not elapsed. Muted sources are still skipped,
    /// because zero-valued samples from a muted source are expected.
    Probe {
        /// Specific source to probe (default: all HFP-active bluez sources)
        #[arg(long, value_name = "NAME")]
        source: Option<String>,
        /// Probe duration in milliseconds (default: 500)
        #[arg(long = "duration-ms", value_name = "N")]
        duration_ms: Option<u64>,
    },
    /// Disable and remove the service (systemd or OpenRC)
    UninstallService,
}

#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub preferred_profiles: Option<Vec<String>>,
    pub input_volume: Option<u32>,
    pub auto_recover_stuck_sco: Option<AutoRecoverMode>,
    pub broken_vendors: Option<Vec<String>>,
    pub debounce_ms: Option<u64>,
    pub probe_stuck_sco: Option<bool>,
}

pub fn overrides(cli: &Cli) -> Overrides {
    Overrides {
        preferred_profiles: if cli.preferred_profile.is_empty() {
            None
        } else {
            Some(cli.preferred_profile.clone())
        },
        input_volume: cli.input_volume,
        auto_recover_stuck_sco: cli.auto_recover_stuck_sco,
        broken_vendors: if cli.broken_vendor.is_empty() {
            None
        } else {
            Some(cli.broken_vendor.clone())
        },
        debounce_ms: cli.debounce_ms,
        probe_stuck_sco: cli.probe_stuck_sco,
    }
}
