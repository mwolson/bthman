# Stuck SCO Phase 2: auto-remediation

## Scope

Phase 1 (shipped 2026-04-23) detects stuck-SCO and logs a WARN. Phase 2 adds
auto-remediation: when the detector confirms stuck SCO, disconnect the device at
the BlueZ layer and reconnect it via the existing Scheduler.

Default-off behind `--auto-recover-stuck-sco`. Rolls out on the author's machine
first to gather field data on false-positive rate before considering a default
flip.

## Prerequisites

Already in place:

- `sco_probe::probe_source` / `classify` / `log_result`
- `sco_probe::ProbeState` with per-source cooldown tracking
- `reconcile::probe_bluetooth_sources` integration
- `bthman probe` manual subcommand
- `bluetoothctl::try_reconnect` (connect-only) and `Scheduler` (backoff retry)
- Evidence from two 2026-04-23 recurrences, including a 1-second BT_PKT_SEQNUM
  lead on the 16:12 event

## Design

### Detection: two tiers, all-zero is mandatory

Phase 1 captures audio and emits WARN on a single all-zero buffer. Phase 2 needs
a confirmation step before destroying the connection. The detector has two
tiers, but **both tiers require a real all-zero capture**. The `BT_PKT_SEQNUM`
log line is treated as a confidence booster on top of an all-zero, never as a
standalone trigger.

1. **Tier 1 (boosted): all-zero + recent BT_PKT_SEQNUM failure.** If a
   `BT_PKT_SEQNUM` log event has been observed within the last 5 seconds and the
   current probe returns `AllZero`, remediate immediately. Handles the
   short-HFP-window case (the 2026-04-23 16:12 event had only a 7-second HFP
   window; a two-probe-with-gap fallback would have missed it).
2. **Tier 2 (fallback): two consecutive all-zero probes within 3 seconds.** Used
   when no recent SEQNUM event was observed, when the system has no journalctl
   (Alpine/OpenRC), or when WirePlumber's log level is at the WARN default (see
   "WirePlumber log level prerequisite" below). The first all-zero arms a
   follow-up probe in ~2 seconds; if the follow-up also returns `AllZero`,
   remediate. If it returns `HasSignal` or `Unavailable`, clear state and stand
   down.

The same upstream gates from Phase 1 apply to both tiers: card active on HFP,
source not muted, cooldown elapsed.

Rationale for requiring an all-zero in Tier 1: we have one confirmed correlation
between BT_PKT_SEQNUM and stuck SCO (the 16:12 event). One data point is enough
to use SEQNUM as a tie-breaker but not enough to use it as the sole authority
for ripping a connection down. The all-zero capture is the ground truth; SEQNUM
only changes how many captures we wait for.

### Subscription alternatives investigated

Before committing to journalctl tailing for the BT_PKT_SEQNUM signal, the
following passive subscription paths were investigated:

- **org.bluez.MediaTransport1.State (PropertiesChanged)**: BlueZ thinks the
  transport is still `active` during a stuck-SCO event. The SCO socket
  acquisition succeeded; BlueZ is unaware that the kernel/firmware stopped
  delivering audio. State does not transition.
- **org.bluez.Bearer.BREDR1.Disconnected**: only fires on full BREDR teardown,
  not SCO stall.
- **org.pipewire.Telephony.AudioGatewayTransport1.State (PipeWire user-bus)**:
  object only exists while a call is active, and PipeWire's
  `spa/plugins/bluez5/sco-io.c` `sco_io_on_ready` handles `SPA_IO_ERR` /
  `SPA_IO_HUP` by removing the IO source with no state propagation. The
  keepalive then injects silence frames, which is exactly the symptom we
  observe. State does not transition for this case either.
- **Kernel mgmt netlink, debugfs, rfkill, udev**: none expose "SCO socket alive
  but silent". The kernel does not know the audio is silent; that is why
  BT_PKT_SEQNUM matters in the first place (the kernel's setsockopt probe is the
  only thing that fails).
- **WirePlumber source for BT_PKT_SEQNUM**: the string lives in PipeWire's
  `spa/plugins/bluez5/decode-buffer.h` `spa_bt_recvmsg_init`, emitted via
  `spa_log_info` (level 3, INFO). No DBus signal, no property mutation, no
  callback. Logging is the only emit surface, and the level matters: see
  "WirePlumber log level prerequisite" below.

The conclusion: by design, no DBus or kernel event surfaces the "SCO alive but
silent" failure mode. PipeWire's keepalive deliberately masks it. Log tailing is
the only path short of patching upstream. This section exists so the alternative
is not re-investigated later under the assumption it was just overlooked.

### WirePlumber log level prerequisite

The `BT_PKT_SEQNUM` line is emitted at `SPA_LOG_LEVEL_INFO` (level 3).
WirePlumber's stock default level is `WARN` (level 2). This means **on a default
WirePlumber install, the BT_PKT_SEQNUM line never reaches journald** and the
Tier 1 detector path is unreachable.

The user's machine originally had `WIREPLUMBER_DEBUG=D` set in their dotfiles
(commit `f245a2f` 2026-04-19,
`~/dotfiles/config/systemd-user/wireplumber.service.d/debug.conf`), which is why
we observed the line at all on the 2026-04-23 16:12 event. It now uses
`WIREPLUMBER_DEBUG=I`, which still emits the detector signal without DEBUG-level
log volume.

The minimum level required for the detector is `WIREPLUMBER_DEBUG=I` (info). `I`
is sufficient and substantially less noisy than `D`: in the 16:12 capture, INFO
lines were 1,920 vs DEBUG's 25,068 (~7% of the volume). Recommend `I` to users
in the README; DEBUG should only be used for short targeted capture sessions.

To set this:

- systemd user (Arch, Fedora, recent Debian):
  `systemctl --user edit wireplumber.service` and add
  `[Service]\nEnvironment=WIREPLUMBER_DEBUG=I`, then
  `systemctl --user restart wireplumber.service`.
- Or: `~/.config/environment.d/wireplumber.conf` with `WIREPLUMBER_DEBUG=I`.
- OpenRC: irrelevant (no journalctl anyway, see fallback section).

**Startup detection**: `LogWatcher::spawn()` performs a one-shot probe of the
last ~200 wireplumber journal lines and looks for any line at INFO or DEBUG
level (`spa_bt_recvmsg_init` cannot be tested directly because it only fires on
actual SCO setup). Detection is asymmetric:

- If at least one INFO or DEBUG line is seen, the level is high enough. Proceed
  normally.
- If no INFO/DEBUG lines are seen in the last 200 entries, log a one-time WARN
  (`tier_1_unreachable: WirePlumber log level appears to be WARN; set WIREPLUMBER_DEBUG=I to enable Tier 1 detection`).
  Continue spawning the log watcher anyway (it costs nothing and may fire if the
  level changes later); the daemon transparently runs in tier-2-only mode until
  the watcher actually starts producing events.

Detection runs once at watcher spawn (which happens per inner-loop iteration,
but the result is cached for the daemon's lifetime to avoid repeating the warn
after pactl-subscribe restarts).

### Remediation path

On confirmed stuck SCO (Tier 1 or Tier 2):

```
1. re-check active profile is still headset-head-unit (cards_dump from current pass)
2. re-check the source is still not muted
3. mark probe_state.remediation_in_progress.insert(source)
4. log structured: stuck_sco_decision: source=<s> tier=<n> seqnum_recent=<bool> action=remediate
5. bluetoothctl disconnect <addr>
6. settle 500ms (let BlueZ release the ACL)
7. scheduler.schedule([addr]) using existing backoff [0.0, 0.5, 1.5, 3.5]
8. on Scheduler success or final failure, remove addr from remediation_in_progress
```

Steps 1-2 close a TOCTOU window: the card may have left HFP between the all-zero
capture and the disconnect call. Both checks are cheap (we already have the
cards_dump from the current reconcile pass) and bail to LogOnly if either fails.
The disconnect is a one-shot; the reconnect reuses the existing pipeline so it
inherits retry, timeout, and logging behavior. No new reconnect code.

`remediation_in_progress` is consulted by `probe_bluetooth_sources` at the top
of the per-source loop: if the source is in the set, skip probing. Prevents a
second probe firing in the gap between disconnect and the Scheduler completing
reconnect, which would re-arm the detector on a card that is legitimately
offline.

### Safeguards

- **Opt-in**: default `--auto-recover-stuck-sco=off`. Env, CLI, config file.
- **Per-device rate limit**: at most one auto-recovery per device per 5 min.
  Prevents reconnect loops on genuinely broken hardware.
- **Minimum link uptime**: skip remediation within 10 seconds of the device
  first appearing on HFP. Prevents boot-time thrash and lets a natural app retry
  recover from a stuck first negotiation.
- **External recorder deference**: already exists in reconcile, keep it.
  Remediation is skipped whenever the reconcile pass is skipped.
- **In-flight remediation gate**: `remediation_in_progress` set above.
- **Dry-run mode**: `--auto-recover-stuck-sco=dry-run` logs
  `stuck_sco_decision: ... action=would_remediate` and trips the rate limiter
  but does not actually disconnect. Useful for validating the confirmation logic
  before trusting the trigger.

### Tier-2-only mode (OpenRC, no-systemd, default-log-level)

Three populations cannot reach Tier 1 and run in Tier-2-only mode:

1. **Alpine/OpenRC**: bthman's integration tests cover this
   (`integration-tests/openrc-user/`, `integration-tests/openrc-system/`).
   Alpine does not ship journald, so `journalctl` will not exist.
2. **systemd hosts with no user session for wireplumber**: rare on modern
   distros but possible.
3. **systemd hosts at WirePlumber's default WARN log level**: the most common
   case. The watcher spawns successfully but never sees an event because the
   source line is INFO. See "WirePlumber log level prerequisite" above.

Phase 2 treats Tier-2-only as a first-class supported mode, not graceful
failure:

- Populations 1 and 2: `LogWatcher::spawn()` returns `Err` (binary missing, unit
  not found, no systemd user session). bthman logs one INFO at startup
  (`log_watcher unavailable: <reason>; running in tier-2-only mode`) and
  continues without it.
- Population 3: `LogWatcher::spawn()` succeeds; the startup probe (see
  "WirePlumber log level prerequisite") detects the WARN level and logs a
  one-time WARN telling the user how to opt in. The watcher stays running in
  case the level is raised later.
- The log watcher slot in the inner-loop `select!` is `Option<Receiver>`; if
  `None`, that arm is a no-op (use `crossbeam_channel::never()`).
- `last_seqnum_failure` stays `None` until/unless an event fires. The Tier 1
  branch is unreachable in practice; the Tier 2 branch handles all detection.
- Detection is strictly slower in this mode (need two probes within 3s, which
  restricts what HFP windows we can catch). This is acceptable because the
  tradeoff is documented and the user can upgrade to Tier 1 by setting one env
  var.

A future enhancement could read WirePlumber's stderr from a known log file on
OpenRC hosts (path is per-deployment, set in
`integration-tests/openrc-user/conf.d/wireplumber`). Out of Phase 2 scope.

`src/logging.rs` already has soft systemd detection (it suppresses timestamps
when `JOURNAL_STREAM` or `INVOCATION_ID` is set). The Phase 2 log watcher is the
first feature that would use journalctl; it must degrade as cleanly as
`logging.rs` does.

## Architecture

### New module `src/log_watcher.rs`

```rust
pub struct LogWatcher {
    child: Child,
    rx: Receiver<LogEvent>,
}

pub enum LogEvent {
    SeqnumFailure,
}

impl LogWatcher {
    pub fn spawn() -> Result<Self>;
    pub fn rx(&self) -> &Receiver<LogEvent>;
}
```

Spawns:

```sh
journalctl --user -u wireplumber.service \
  -f -n 0 --no-pager \
  --grep 'failed to set BT_PKT_SEQNUM'
```

`--grep` filters at the journalctl layer so our reader only sees matching lines.
`-n 0` skips history. `-f` follows indefinitely. Reader thread sends
`LogEvent::SeqnumFailure` on each match. Same pattern as
`spawn_pactl_subscribe`.

Optionality: `LogWatcher::spawn()` returns `Result<Self>`. The caller (in
`run_watch`) treats `Err` as expected on non-systemd hosts and stores
`Option<LogWatcher>`. The `select!` arm uses `crossbeam_channel::never()` when
the watcher is absent. See "Tier-2-only mode" above.

After successful spawn, `LogWatcher` runs the one-time level probe described in
"WirePlumber log level prerequisite": shells out to
`journalctl --user -u wireplumber.service -n 200 --no-pager` and scans the
output for any line matching `^... [ID] ` (the WirePlumber level marker). If no
INFO/DEBUG line is found, log the one-time WARN. The probe runs synchronously
during spawn so the daemon's startup log clearly indicates which mode it is in.

Verify before merging: `journalctl -f --grep` was added in systemd v237. Debian
bookworm ships v252, Alpine ships nothing, Arch ships current. The flag is
universally available on systemd hosts current bthman supports; spot-check on
bookworm to be safe.

If wireplumber is restarted mid-session, `journalctl -f` continues across the
restart (same unit, new process). If the unit is removed and re-added,
`journalctl` exits; the inner-loop restart path picks up the new unit on the
next iteration.

### Refactored `ProbeState` (per-source state machine)

Replace the current `last_probe: HashMap` shape with a per-source state struct
and a single API surface that returns the next action.

```rust
pub struct ProbeState {
    sources: HashMap<String, SourceState>,
    devices: HashMap<String, DeviceState>,  // keyed by addr
    last_seqnum_failure: Option<Instant>,
    remediation_in_progress: HashSet<String>,  // source names
}

struct SourceState {
    last_probe: Option<Instant>,
    last_all_zero: Option<Instant>,
    follow_up_due: Option<Instant>,
}

struct DeviceState {
    first_hfp_seen: Option<Instant>,
    last_remediation: Option<Instant>,
}

pub enum ProbeAction {
    Skip { reason: SkipReason },
    Probe,
    FollowUpProbe,
}

impl ProbeState {
    pub fn next_action(&self, source: &str, now: Instant, cooldown: Duration) -> ProbeAction;
    pub fn record_probe(&mut self, source: &str, result: ProbeResult, now: Instant);
    pub fn record_seqnum_failure(&mut self, now: Instant);
    pub fn record_hfp_seen(&mut self, addr: &str, now: Instant);
    pub fn record_hfp_left(&mut self, addr: &str);
    pub fn next_wakeup(&self, now: Instant) -> Option<Duration>;
    /* etc */
}
```

`next_wakeup()` returns the earliest deadline across all sources'
`follow_up_due` fields. The inner-loop `compute_timeout()` consults it alongside
the existing debounce and scheduler deadlines.

This refactor folds the "follow-up probe deadline" mentioned in the Open
Questions section into the state itself. The inner loop does not own a separate
timer; it just polls `probe_state.next_wakeup()` and triggers a reconcile when
the deadline expires.

`remediation_in_progress` is read at the top of `probe_bluetooth_sources`; if
the source is present, that source is skipped this pass.

### Remediation helper

New module `src/remediation.rs`:

```rust
pub fn remediate(
    bluetoothctl: &dyn BluetoothOps,
    scheduler: &mut Scheduler,
    state: &mut ProbeState,
    addr: &str,
    source: &str,
    mode: AutoRecoverMode,
) -> Result<()>;
```

1. Logs structured event:
   `stuck_sco_decision: source=<s> tier=<n> seqnum_recent=<bool> all_zero_count=<n> action=<remediate|would_remediate> addr=<a>`
2. If `mode == DryRun`, sets `state.last_remediation` and returns.
3. Otherwise, inserts source into `remediation_in_progress`.
4. Calls `bluetoothctl.disconnect(addr)`. On error, logs ERROR
   `stuck_sco_remediation_failed: addr=<a> stage=disconnect err=<msg>`, removes
   from `remediation_in_progress`, returns.
5. Sleeps 500ms.
6. Calls `scheduler.schedule(now, [addr])`.
7. Returns. The daemon clears `remediation_in_progress` for the addr after the
   Scheduler reports completion (success or final failure); see hook discussion
   below. On final-failure, log ERROR
   `stuck_sco_remediation_failed: addr=<a> stage=reconnect attempts=<n>`.

Add `disconnect(addr: &str) -> bool` to `BluetoothOps` trait in
`src/reconnect.rs` and to `bluetoothctl::try_disconnect` in
`src/bluetoothctl.rs`. Implementation mirrors `try_reconnect` but spawns
`bluetoothctl disconnect <addr>`.

The Scheduler currently has no callback hook to clear `remediation_in_progress`.
Two options:

- (a) Pass a `&mut ProbeState` reference through `Scheduler::process` for the
  reconnect-completion path.
- (b) Add `Scheduler::take_completed() -> Vec<(String, Outcome)>` and have the
  daemon call it after each `process()` to clear `remediation_in_progress` and
  emit the failure-mode ERROR for any exhausted-backoff entries.

Lean toward (b): keeps the Scheduler ignorant of ProbeState and centralizes the
"what do we log on remediation outcome" logic in one place.

### Updated `reconcile::probe_bluetooth_sources`

```rust
for card in hfp_active_cards(cards_dump) {
    let addr = addr_from_card(&card);
    let source = source_from_addr(&addr);

    probe_state.record_hfp_seen(&addr, now);

    if probe_state.is_remediation_in_progress(&source) {
        continue;
    }
    if !is_source_unmuted(runner, &source) {
        continue;
    }
    let action = probe_state.next_action(&source, now, DEFAULT_PROBE_COOLDOWN);
    match action {
        ProbeAction::Skip { .. } => continue,
        ProbeAction::Probe | ProbeAction::FollowUpProbe => {
            let result = probe_source(probe, &source, DEFAULT_PROBE_DURATION);
            probe_state.record_probe(&source, result.clone(), now);
            log_result(&source, &result);

            if !config.auto_recover_stuck_sco.enabled() {
                continue;
            }
            let decision = decide(&result, &source, &addr, probe_state, now, config);
            apply_decision(decision, runner, scheduler, probe_state, &source, &addr);
        }
    }
}

// After the loop: cards that previously had HFP but no longer do
for addr in addrs_no_longer_hfp {
    probe_state.record_hfp_left(&addr);
}
```

`decide()` is a pure function. `apply_decision()` calls into
`remediation::remediate` for the `Remediate` and `DryRunRemediate` cases.

### Wire-up in `daemon.rs`

In `run_watch` (per inner-loop iteration):

```rust
let log_watcher_opt = match LogWatcher::spawn() {
    Ok(w) => Some(w),
    Err(err) => {
        info!("log_watcher unavailable: {}; running in tier-2-only mode", err);
        None
    }
};
```

In `inner_loop`:

```rust
let log_rx = log_watcher_opt
    .as_ref()
    .map(|w| w.rx().clone())
    .unwrap_or_else(crossbeam_channel::never);
```

Extend `select!`:

```rust
recv(log_rx) -> msg => match msg {
    Ok(LogEvent::SeqnumFailure) => probe_state.record_seqnum_failure(Instant::now()),
    Err(_) => {
        if log_watcher_opt.is_some() {
            info!("log_watcher ended; will restart");
            return InnerExit::Restart;
        }
    }
}
```

Extend `compute_timeout`:

```rust
if let Some(d) = probe_state.next_wakeup(now) {
    if d < timeout { timeout = d; }
}
```

When the timeout fires for a `follow_up_due` deadline (rather than the debounce
deadline), call `reconcile_persistent` with trigger `"follow_up_probe"`.

### CLI / Config

```rust
#[arg(long, value_name = "MODE")]
auto_recover_stuck_sco: Option<AutoRecoverMode>,
```

Where `AutoRecoverMode` is `enum { Off, DryRun, On }`. Default `Off`. Parsed
from config file as `auto-recover-stuck-sco = "off"|"dry-run"|"on"`. Threaded
through `Config` and `Overrides`.

Add to README's daemon options list.

### Logging conventions

All Phase 2 detector decisions emit a single structured line with a stable
prefix to make field-data analysis grep-friendly:

```
stuck_sco_decision: source=<s> tier=<1|2|na> seqnum_recent=<true|false>
  all_zero_count=<n> uptime_s=<n> rate_limited=<true|false> action=<a>
  [addr=<a>]
```

Where `action` is one of `log_only`, `record_all_zero`, `arm_follow_up`,
`would_remediate`, `remediate`, `skip_recent_remediation`, `skip_min_uptime`,
`skip_in_progress`, `skip_profile_changed`.

Failure paths:

```
stuck_sco_remediation_failed: addr=<a> stage=<disconnect|reconnect>
  err=<msg> [attempts=<n>]
```

These are not parsed by anything yet, but the stable prefix means we can write a
one-line awk to slice the data when validating dry-run output.

## State machine

`decide()` pseudocode (assumes feature enabled, otherwise return LogOnly):

```text
if probe_result == HasSignal:
    state.clear_all_zero(source)
    state.clear_follow_up(source)
    return LogOnly

if probe_result == Unavailable:
    return LogOnly

# probe_result == AllZero
if state.last_remediation(addr) within 5 min:
    return Skip(SkipReason::RecentRemediation)
if state.first_hfp_seen(addr) within 10 s:
    return Skip(SkipReason::MinUptime)
if !active_profile_still_hfp(cards_dump, card):
    return Skip(SkipReason::ProfileChanged)
if source_is_now_muted(runner, source):
    return Skip(SkipReason::Muted)

seqnum_recent = state.last_seqnum_failure within 5 s
prior_all_zero = state.last_all_zero(source) within 3 s

if seqnum_recent:
    return Remediate(tier=1) or DryRunRemediate(tier=1)
if prior_all_zero:
    return Remediate(tier=2) or DryRunRemediate(tier=2)

# first all-zero, no SEQNUM signal: arm follow-up probe
state.arm_follow_up(source, now + 2s)
return RecordAllZero
```

Notes:

- The 2-second follow-up window must be shorter than the 7-second HFP window
  observed on the 2026-04-23 16:12 event (and the 4-second active-then-revert
  window observed on the 14:30 event). 2s leaves ~5s and ~2s of margin
  respectively.
- `last_seqnum_failure` is global to the daemon (not per-card). This is
  acceptable because the log line does not identify a device, and the only
  HFP-active card present at the time is the one we are probing. If future
  evidence shows multiple HFP-active cards routinely, scope this per-adapter
  (BlueZ adapter object path) or per "any HFP-active card present, by addr".
- `record_all_zero` and `arm_follow_up` are expressed in the API as a single
  `record_probe(AllZero, now)` that updates both the `last_all_zero` field and
  the `follow_up_due` field.

## Tests

`tests/log_watcher.rs`:

- Feeds fake stdin lines through a spawn-less variant of LogWatcher, verifies
  SEQNUM line produces `LogEvent::SeqnumFailure`.
- Non-matching lines produce no events.
- EOF / child exit ends the stream cleanly.
- Level-probe parser: input containing at least one `... I spa.bluez5...` line
  returns "level adequate"; input with only `... W ...` / `... E ...` lines
  returns "level inadequate" (triggers WARN).
- `LogWatcher::spawn()` returns Err when journalctl is missing (test by setting
  `PATH=` on the spawn command).

`tests/sco_probe_state.rs` (extends the existing module test):

- `next_action` returns `Probe` on first call, `Skip(Cooldown)` on immediate
  re-call, `Probe` after cooldown elapses.
- `record_probe(AllZero)` arms `follow_up_due` at now + 2s.
- `record_probe(HasSignal)` clears `follow_up_due` and `last_all_zero`.
- `record_seqnum_failure` sets `last_seqnum_failure`; reads after 5s treat it as
  expired.
- `record_hfp_seen` is idempotent across repeated calls; `record_hfp_left`
  resets `first_hfp_seen` and `last_all_zero` for the addr's source.
- `remediation_in_progress` insert/remove visible to
  `is_remediation_in_progress`.
- `next_wakeup` returns the earliest follow-up deadline across all sources, or
  `None` if none armed.

`tests/decide.rs` (table-driven matrix):

- `(probe_result, seqnum_recent, prior_all_zero, uptime_lt_10s, rate_limited, profile_changed, source_now_muted) -> expected_decision`
- Cases: HasSignal clears state regardless of other inputs. Unavailable is
  LogOnly. AllZero with SEQNUM recent is Tier 1. AllZero with prior all-zero is
  Tier 2. AllZero alone is RecordAllZero + arm follow-up. AllZero with
  rate-limit is Skip(RecentRemediation). AllZero with uptime < 10s is
  Skip(MinUptime). AllZero with profile changed is Skip(ProfileChanged). AllZero
  with muted is Skip(Muted).

`tests/remediation.rs`:

- Successful path: disconnect call recorded, 500ms sleep elapsed,
  scheduler.schedule called with the addr, `remediation_in_progress` contains
  the source.
- Disconnect failure: ERROR logged, `remediation_in_progress` cleared, scheduler
  not called.
- Dry-run: no disconnect call recorded, `last_remediation` updated,
  `would_remediate` line logged.

`tests/reconcile.rs`:

- HFP active + all-zero + SEQNUM recent + feature on: remediation called.
- HFP active + all-zero + no SEQNUM + no prior all-zero + feature on: state
  updated, follow-up armed, no remediation.
- Follow-up reconcile triggered by `next_wakeup`: second all-zero within 3s
  causes remediation.
- Follow-up reconcile: HasSignal clears state, no remediation.
- Source in `remediation_in_progress`: probe skipped.
- `--auto-recover-stuck-sco=off`: Phase 1 behavior preserved (LogOnly regardless
  of state).
- `--auto-recover-stuck-sco=dry-run`: rate limiter trips, no
  `bluetoothctl disconnect` call, `would_remediate` log line emitted.

`tests/daemon.rs` (or integration-style):

- `LogWatcher::spawn()` returning Err results in `log_rx` being a `never()`
  channel and the daemon proceeding without it; an INFO line is emitted.
- Scheduler completion clears `remediation_in_progress` for the addr.

Integration tests (Docker):

- Add a Dockerfile scenario where fake `bluetoothctl` records disconnect and
  connect calls; drive a stuck-SCO reproduction via a fake parecord that returns
  all-zero, verify end-to-end remediation.
- Add an OpenRC scenario where journalctl is missing, the daemon starts, and
  Tier 2 detection (two probes) triggers remediation.

## Rollout

1. Land Phase 2 code with `auto-recover-stuck-sco=off` default. README must
   document the `WIREPLUMBER_DEBUG=I` prerequisite for Tier 1 in the same
   section as the new flag, with the systemd user override snippet inline.
2. Author flips to `dry-run` locally. Observes for a week, collects
   `stuck_sco_decision` log lines. Validates that `would_remediate` events
   correspond to real stuck-SCO and not false positives on normal silence / PTT
   toggles / codec changes.
3. Flip to `on`. Observe for another week. Any unexpected disconnects means flip
   back to `dry-run`, diagnose.
4. Once stable, add a README note recommending opt-in for users who see the
   symptom. The recommendation should mention both the
   `--auto-recover-stuck-sco=on` flag and the `WIREPLUMBER_DEBUG=I` env, and
   note that Tier 2 still works without the env (slower).
5. Default-on is a separate decision, gated on broader field data (multiple
   users, multiple devices). Not part of Phase 2.

## Open questions

- Does `BT_PKT_SEQNUM` failure ever occur without resulting in a stuck SCO? If
  so, the boosted Tier 1 path could remediate after a single all-zero on a
  transient hiccup that would self-resolve. One confirmed correlation so far.
  Dry-run period will surface false-positive rate.
- Journald permission model across different session setups. We assume
  `journalctl --user -u wireplumber.service` is readable by the bthman user. If
  wireplumber runs in a different systemd user session (rare on modern distros),
  the watcher logs an INFO and falls back; verify the Err shape is
  distinguishable from "no events for now".
- 2-second follow-up window is calibrated for the two observed events. Could be
  too tight if HFP windows are sometimes shorter (e.g., 1s). Dry-run period will
  tell.

## What stays out of Phase 2

- Background / periodic probe (not just on reconcile events). The BT_PKT_SEQNUM
  watcher is a more natural passive trigger. Pursue background probing only if
  the watcher turns out to be unreliable.
- Default-on rollout. Requires broader field data.
- Multi-device stuck-SCO correlation. Current design assumes one stuck device at
  a time (plausible given how BlueZ handles SCO); revisit if this assumption
  breaks.
- Reading WirePlumber stderr from a known log file path on OpenRC hosts. Path is
  per-deployment and would require user configuration. Tier 2 fallback covers
  these hosts adequately for now.
- DBus-based detection of stuck SCO. Investigated and rejected; see
  "Subscription alternatives investigated" in Design.
