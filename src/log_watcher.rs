use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;
use tracing::info;

use crate::sco_probe::ConcealmentTracker;

const CONCEALMENT_TOPIC: &str = "spa.bluez5.source.media";

static CONCEALMENT_LOGGING_NOTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    ConcealmentSustained,
}

pub struct LogWatcher {
    child: Child,
    rx: Receiver<LogEvent>,
}

impl LogWatcher {
    pub fn spawn(tracker: ConcealmentTracker) -> Result<Self> {
        spawn_with_command("journalctl", tracker)
    }

    pub fn spawn_with_command(command: &str, tracker: ConcealmentTracker) -> Result<Self> {
        spawn_with_command(command, tracker)
    }

    pub fn rx(&self) -> Receiver<LogEvent> {
        self.rx.clone()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

fn spawn_with_command(command: &str, tracker: ConcealmentTracker) -> Result<LogWatcher> {
    note_if_concealment_logging_unseen(command);
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
            "produced PLC audio",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning journalctl wireplumber watcher")?;
    let stdout = child.stdout.take().context("journalctl stdout missing")?;
    let (tx, rx) = unbounded();
    spawn_reader(stdout, tx, tracker)?;
    Ok(LogWatcher { child, rx })
}

pub fn read_events<R: Read>(reader: R, tx: Sender<LogEvent>, tracker: &ConcealmentTracker) {
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        if is_concealment(&line) && tracker.record_block(Instant::now()) {
            let _ = tx.send(LogEvent::ConcealmentSustained);
        }
    }
}

pub fn concealment_logging_seen(text: &str) -> bool {
    text.lines().any(|line| {
        line.contains(&format!("D {}", CONCEALMENT_TOPIC))
            || (line.contains(CONCEALMENT_TOPIC) && journal_priority(line) == Some(7))
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    tx: Sender<LogEvent>,
    tracker: ConcealmentTracker,
) -> Result<()> {
    thread::Builder::new()
        .name("bthman-log-watcher".into())
        .spawn(move || read_events(reader, tx, &tracker))
        .context("spawning log watcher reader")?;
    Ok(())
}

// The topic only logs while an HFP link is up, so a boot without a call yet
// also lands here. That is why this is INFO rather than a warning.
fn note_if_concealment_logging_unseen(command: &str) {
    let Ok(output) = Command::new(command)
        .args([
            "--user",
            "-u",
            "wireplumber.service",
            "-b",
            "--grep",
            CONCEALMENT_TOPIC,
            "-n",
            "1",
            "--no-pager",
            "--output=json",
        ])
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    if !concealment_logging_seen(&text) && !CONCEALMENT_LOGGING_NOTED.swap(true, Ordering::Relaxed)
    {
        info!(
            "concealment_unconfirmed: no {} DEBUG lines from WirePlumber this boot; concealment detection needs WIREPLUMBER_DEBUG=W,{}:D",
            CONCEALMENT_TOPIC, CONCEALMENT_TOPIC
        );
    }
}

fn is_concealment(line: &str) -> bool {
    line.contains("produced PLC audio")
}

fn journal_priority(line: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(line).ok()?;
    let priority = value.get("PRIORITY")?;
    priority
        .as_u64()
        .or_else(|| priority.as_str()?.parse::<u64>().ok())
}
