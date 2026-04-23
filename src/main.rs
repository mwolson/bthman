use anyhow::Result;
use bthman::{cli, config, daemon, deps, logging, probe_cmd, service, signals};
use clap::Parser;
use tracing::error;

fn main() {
    logging::init();
    let parsed = cli::Cli::parse();
    match dispatch(parsed) {
        Ok(()) => {}
        Err(err) => {
            error!("{:#}", err);
            std::process::exit(1);
        }
    }
}

fn dispatch(parsed: cli::Cli) -> Result<()> {
    match &parsed.command {
        Some(cli::Command::InstallService) => service::install(),
        Some(cli::Command::Probe {
            source,
            duration_ms,
        }) => probe_cmd::run_manual_probe(source.as_deref(), *duration_ms),
        Some(cli::Command::UninstallService) => service::uninstall(),
        None => run_daemon(parsed),
    }
}

fn run_daemon(parsed: cli::Cli) -> Result<()> {
    deps::check_required()?;
    let cli_config = cli::overrides(&parsed);
    let config = config::Config::build(&cli_config, config::default_conf_path().as_deref())?;
    let sigs = signals::install()?;
    if parsed.once {
        daemon::run_once(&config)
    } else {
        daemon::run_watch(config, cli_config, sigs)
    }
}
