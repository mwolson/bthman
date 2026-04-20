use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::info;

pub trait BluetoothOps {
    fn device_is_connected(&self, addr: &str) -> bool;
    fn try_reconnect(&self, addr: &str) -> bool;
}

pub struct RealOps {
    pub reconnect_timeout: Duration,
}

impl BluetoothOps for RealOps {
    fn device_is_connected(&self, addr: &str) -> bool {
        crate::bluetoothctl::device_is_connected(addr)
    }

    fn try_reconnect(&self, addr: &str) -> bool {
        crate::bluetoothctl::try_reconnect(addr, self.reconnect_timeout)
    }
}

#[derive(Debug, Clone)]
struct Task {
    due: Instant,
    attempt: usize,
}

pub struct Scheduler {
    tasks: HashMap<String, Task>,
    backoff: Vec<Duration>,
}

impl Scheduler {
    pub fn new(backoff: Vec<Duration>) -> Self {
        Self {
            tasks: HashMap::new(),
            backoff,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn schedule<I>(&mut self, now: Instant, addrs: I)
    where
        I: IntoIterator<Item = String>,
    {
        let first_delay = self.backoff.first().copied().unwrap_or(Duration::ZERO);
        let due = now + first_delay;
        for addr in addrs {
            self.tasks.insert(addr, Task { due, attempt: 0 });
        }
    }

    pub fn next_due(&self) -> Option<Instant> {
        self.tasks.values().map(|t| t.due).min()
    }

    pub fn process(&mut self, now: Instant, ops: &dyn BluetoothOps) {
        if self.tasks.is_empty() {
            return;
        }
        let addrs: Vec<String> = self.tasks.keys().cloned().collect();
        for addr in addrs {
            let Some(task) = self.tasks.get(&addr).cloned() else {
                continue;
            };
            if now < task.due {
                continue;
            }
            if ops.device_is_connected(&addr) {
                info!("Reconnect: {} is back, dropping retry", addr);
                self.tasks.remove(&addr);
                continue;
            }
            let attempt = task.attempt;
            info!(
                "Reconnect: attempt {}/{} for {}",
                attempt + 1,
                self.backoff.len(),
                addr
            );
            let ok = ops.try_reconnect(&addr);
            if ok && ops.device_is_connected(&addr) {
                self.tasks.remove(&addr);
                continue;
            }
            let next_attempt = attempt + 1;
            if next_attempt >= self.backoff.len() {
                info!(
                    "Reconnect: giving up on {} after {} attempts",
                    addr,
                    self.backoff.len()
                );
                self.tasks.remove(&addr);
                continue;
            }
            let delta = self
                .backoff
                .get(next_attempt)
                .copied()
                .unwrap_or(Duration::ZERO)
                .saturating_sub(self.backoff.get(attempt).copied().unwrap_or(Duration::ZERO));
            self.tasks.insert(
                addr,
                Task {
                    due: now + delta,
                    attempt: next_attempt,
                },
            );
        }
    }
}
