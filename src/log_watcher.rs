use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;
use tracing::warn;

static LEVEL_WARNED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    SeqnumFailure,
}

pub struct LogWatcher {
    child: Child,
    rx: Receiver<LogEvent>,
}

impl LogWatcher {
    pub fn spawn() -> Result<Self> {
        spawn_with_command("journalctl")
    }

    pub fn spawn_with_command(command: &str) -> Result<Self> {
        spawn_with_command(command)
    }

    pub fn rx(&self) -> Receiver<LogEvent> {
        self.rx.clone()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

fn spawn_with_command(command: &str) -> Result<LogWatcher> {
    warn_if_wireplumber_level_inadequate(command);
    let mut child = Command::new(command)
        .args([
            "--user",
            "-u",
            "wireplumber.service",
            "-f",
            "-n",
            "0",
            "--no-pager",
            "--grep",
            "failed to set BT_PKT_SEQNUM",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning journalctl wireplumber watcher")?;
    let stdout = child.stdout.take().context("journalctl stdout missing")?;
    let (tx, rx) = unbounded();
    spawn_reader(stdout, tx)?;
    Ok(LogWatcher { child, rx })
}

pub fn read_events<R: Read>(reader: R, tx: Sender<LogEvent>) {
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        if is_seqnum_failure(&line) {
            let _ = tx.send(LogEvent::SeqnumFailure);
        }
    }
}

pub fn level_adequate(text: &str) -> bool {
    text.lines().any(|line| {
        journal_priority(line).is_some_and(|priority| priority >= 6)
            || line.contains(" I ")
            || line.contains(" D ")
            || line.contains("[I]")
            || line.contains("[D]")
    })
}

fn spawn_reader<R: Read + Send + 'static>(reader: R, tx: Sender<LogEvent>) -> Result<()> {
    thread::Builder::new()
        .name("bthman-log-watcher".into())
        .spawn(move || read_events(reader, tx))
        .context("spawning log watcher reader")?;
    Ok(())
}

fn warn_if_wireplumber_level_inadequate(command: &str) {
    let Ok(output) = Command::new(command)
        .args([
            "--user",
            "-u",
            "wireplumber.service",
            "-n",
            "200",
            "--no-pager",
            "--output=json",
        ])
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    if !level_adequate(&text) && !LEVEL_WARNED.swap(true, Ordering::Relaxed) {
        warn!(
            "tier_1_unreachable: WirePlumber log level appears to be WARN; set WIREPLUMBER_DEBUG=I to enable Tier 1 detection"
        );
    }
}

fn is_seqnum_failure(line: &str) -> bool {
    line.contains("failed to set BT_PKT_SEQNUM")
}

fn journal_priority(line: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(line).ok()?;
    let priority = value.get("PRIORITY")?;
    priority
        .as_u64()
        .or_else(|| priority.as_str()?.parse::<u64>().ok())
}
