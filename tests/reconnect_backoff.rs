use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bthman::reconnect::{BluetoothOps, Completion, Scheduler};

struct FakeOps {
    attempts: AtomicUsize,
    connected: bool,
}

impl FakeOps {
    fn new(connected: bool) -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            connected,
        }
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl BluetoothOps for FakeOps {
    fn device_is_connected(&self, _addr: &str) -> bool {
        self.connected
    }

    fn try_disconnect(&self, _addr: &str) -> bool {
        true
    }

    fn try_reconnect(&self, _addr: &str) -> bool {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        false
    }
}

fn default_backoff() -> Vec<Duration> {
    vec![
        Duration::from_millis(0),
        Duration::from_millis(500),
        Duration::from_millis(1500),
        Duration::from_millis(3500),
    ]
}

#[test]
fn scheduler_starts_empty() {
    let sched = Scheduler::new(default_backoff());
    assert!(sched.is_empty());
    assert_eq!(sched.next_due(), None);
}

#[test]
fn schedule_queues_at_first_delay() {
    let mut sched = Scheduler::new(default_backoff());
    let t0 = Instant::now();
    sched.schedule(t0, std::iter::once("AA:BB:CC:DD:EE:FF".to_string()));
    assert!(!sched.is_empty());
    assert_eq!(sched.next_due(), Some(t0));
}

#[test]
fn four_attempts_then_give_up() {
    let mut sched = Scheduler::new(default_backoff());
    let ops = FakeOps::new(false);
    let t0 = Instant::now();
    sched.schedule(t0, std::iter::once("AA:BB:CC:DD:EE:FF".to_string()));

    sched.process(t0, &ops);
    assert_eq!(ops.attempts(), 1);
    assert!(!sched.is_empty());

    sched.process(t0 + Duration::from_millis(499), &ops);
    assert_eq!(ops.attempts(), 1);

    sched.process(t0 + Duration::from_millis(500), &ops);
    assert_eq!(ops.attempts(), 2);

    sched.process(t0 + Duration::from_millis(1500), &ops);
    assert_eq!(ops.attempts(), 3);

    sched.process(t0 + Duration::from_millis(3500), &ops);
    assert_eq!(ops.attempts(), 4);
    assert!(sched.is_empty());
}

#[test]
fn reconnect_stops_when_device_returns() {
    let mut sched = Scheduler::new(default_backoff());
    let ops = FakeOps::new(true);
    let t0 = Instant::now();
    sched.schedule(t0, std::iter::once("AA:BB:CC:DD:EE:FF".to_string()));
    sched.process(t0, &ops);
    assert_eq!(ops.attempts(), 0);
    assert!(sched.is_empty());
    assert_eq!(
        sched.take_completed(),
        vec![Completion::Connected {
            addr: "AA:BB:CC:DD:EE:FF".to_string()
        }]
    );
}

#[test]
fn multiple_addresses_tracked_independently() {
    let mut sched = Scheduler::new(default_backoff());
    let ops = FakeOps::new(false);
    let t0 = Instant::now();
    sched.schedule(
        t0,
        vec![
            "AA:BB:CC:DD:EE:FF".to_string(),
            "11:22:33:44:55:66".to_string(),
        ],
    );
    sched.process(t0, &ops);
    assert_eq!(ops.attempts(), 2);
    assert!(!sched.is_empty());
}

#[test]
fn scheduler_reports_exhausted_completion() {
    let mut sched = Scheduler::new(vec![Duration::ZERO]);
    let ops = FakeOps::new(false);
    let t0 = Instant::now();
    sched.schedule(t0, std::iter::once("AA:BB:CC:DD:EE:FF".to_string()));

    sched.process(t0, &ops);

    assert_eq!(
        sched.take_completed(),
        vec![Completion::Exhausted {
            addr: "AA:BB:CC:DD:EE:FF".to_string(),
            attempts: 1
        }]
    );
    assert!(sched.take_completed().is_empty());
}
