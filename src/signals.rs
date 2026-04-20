use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::events::SignalKind;

pub struct Handles {
    pub stop: Arc<AtomicBool>,
    pub reload: Arc<AtomicBool>,
    pub rx: Receiver<SignalKind>,
}

impl Handles {
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn take_reload(&self) -> bool {
        self.reload.swap(false, Ordering::SeqCst)
    }
}

pub fn install() -> Result<Handles> {
    let stop = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(AtomicBool::new(false));
    let (tx, rx) = unbounded();
    let stop_for_thread = Arc::clone(&stop);
    let reload_for_thread = Arc::clone(&reload);
    let mut signals = Signals::new([SIGHUP, SIGINT, SIGTERM])?;
    std::thread::Builder::new()
        .name("bthman-signals".into())
        .spawn(move || {
            for sig in &mut signals {
                let kind = match sig {
                    SIGINT | SIGTERM => {
                        stop_for_thread.store(true, Ordering::SeqCst);
                        SignalKind::Stop
                    }
                    SIGHUP => {
                        reload_for_thread.store(true, Ordering::SeqCst);
                        SignalKind::Reload
                    }
                    _ => continue,
                };
                let _ = tx.send(kind);
            }
        })?;
    Ok(Handles { stop, reload, rx })
}
