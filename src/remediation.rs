use std::thread;
use std::time::{Duration, Instant};

use crate::cli::AutoRecoverMode;
use crate::reconnect::{BluetoothOps, Scheduler};
use crate::sco_probe::ProbeState;
use tracing::{error, info};

pub struct RemediationRequest<'a> {
    pub addr: &'a str,
    pub all_zero_count: u8,
    pub mode: AutoRecoverMode,
    pub seqnum_recent: bool,
    pub source: &'a str,
    pub tier: u8,
    pub uptime_s: u64,
}

pub fn remediate(
    ops: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
    state: &mut ProbeState,
    request: RemediationRequest<'_>,
    now: Instant,
) {
    state.record_remediation(request.addr, now);
    info!(
        "stuck_sco_decision: source={} tier={} seqnum_recent={} all_zero_count={} uptime_s={} rate_limited=false action={} addr={}",
        request.source,
        request.tier,
        request.seqnum_recent,
        request.all_zero_count,
        request.uptime_s,
        request.mode.action(),
        request.addr
    );

    if matches!(request.mode, AutoRecoverMode::DryRun) {
        return;
    }

    state.set_remediation_in_progress(request.source);
    if !ops.try_disconnect(request.addr) {
        error!(
            "stuck_sco_remediation_failed: addr={} stage=disconnect err=bluetoothctl_disconnect_failed",
            request.addr
        );
        state.clear_remediation_in_progress_for_addr(request.addr);
        return;
    }

    thread::sleep(Duration::from_millis(500));
    scheduler.schedule(
        now + Duration::from_millis(500),
        std::iter::once(request.addr.to_string()),
    );
}
