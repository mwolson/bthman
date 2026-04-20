use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "bthman",
    version,
    about = "Manage Bluetooth HFP profile selection and reconnect headsets after resume"
)]
pub struct Cli {
    /// Reconcile profiles once and exit
    #[arg(long, conflicts_with = "watch")]
    pub once: bool,

    /// Watch for PulseAudio events and reconcile continuously (default)
    #[arg(long)]
    pub watch: bool,

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

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install and enable the service (systemd or OpenRC)
    InstallService,
    /// Disable and remove the service (systemd or OpenRC)
    UninstallService,
}

#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub preferred_profiles: Option<Vec<String>>,
    pub input_volume: Option<u32>,
    pub broken_vendors: Option<Vec<String>>,
    pub debounce_ms: Option<u64>,
}

pub fn overrides(cli: &Cli) -> Overrides {
    Overrides {
        preferred_profiles: if cli.preferred_profile.is_empty() {
            None
        } else {
            Some(cli.preferred_profile.clone())
        },
        input_volume: cli.input_volume,
        broken_vendors: if cli.broken_vendor.is_empty() {
            None
        } else {
            Some(cli.broken_vendor.clone())
        },
        debounce_ms: cli.debounce_ms,
    }
}
