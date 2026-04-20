use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::thread;

use crossbeam_channel::{unbounded, Receiver, Sender};
use tracing::warn;

use crate::deps;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepLine {
    Header,
    True,
    False,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepTransition {
    Suspend,
    Resume,
}

pub fn classify(line: &str) -> Option<SleepLine> {
    if line.contains("member=PrepareForSleep") {
        return Some(SleepLine::Header);
    }
    match line.trim().to_lowercase().as_str() {
        "boolean true" => Some(SleepLine::True),
        "boolean false" => Some(SleepLine::False),
        _ => None,
    }
}

pub fn spawn() -> (Option<Child>, Receiver<SleepTransition>) {
    let (tx, rx) = unbounded();
    let child = spawn_inner(tx);
    (child, rx)
}

fn spawn_inner(tx: Sender<SleepTransition>) -> Option<Child> {
    if deps::which("dbus-monitor").is_none() {
        warn!("dbus-monitor not found; suspend-resume reconnect disabled");
        return None;
    }
    let mut child = Command::new("dbus-monitor")
        .args([
            "--system",
            "type='signal',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let spawn_result = thread::Builder::new()
        .name("bthman-sleep-monitor".into())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            let mut pending = false;
            for line in reader.lines().map_while(Result::ok) {
                match classify(&line) {
                    Some(SleepLine::Header) => pending = true,
                    Some(SleepLine::True) if pending => {
                        pending = false;
                        let _ = tx.send(SleepTransition::Suspend);
                    }
                    Some(SleepLine::False) if pending => {
                        pending = false;
                        let _ = tx.send(SleepTransition::Resume);
                    }
                    _ => {}
                }
            }
        });
    if spawn_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    Some(child)
}
