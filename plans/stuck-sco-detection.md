# Stuck SCO detection and remediation

## Problem

After rapid profile churn or link stress (Vesktop mic test is a reliable
reproducer), some Bluetooth headsets (notably AirPods Pro) leave the HFP SCO
transport in a state where:

- The card shows `Active Profile: headset-head-unit*`
- PipeWire's `spa.bluez5.source.media` decode buffer advances on schedule
- `pactl list sources` shows the `bluez_input.*` source `RUNNING`
- `parecord` off the source produces bit-exact-zero samples (not just quiet)

Downlink still works. The user hears audio. Uplink is silent.

The only known remediation is to disconnect and reconnect the device at the
BlueZ layer. PipeWire's profile switch alone does not recover it.

## Goals

1. Detect the stuck-SCO state automatically, with low false-positive risk.
2. Optionally, remediate by disconnecting/reconnecting the device.
3. Keep the change incremental: land detection first, validate false-positive
   rate in the field, then enable remediation behind an opt-in flag.

## Non-goals

- Monitoring audio continuously. We only probe when we have strong reason to
  expect mic traffic.
- Detecting low-signal mics, echo, packet loss, or codec quality issues. Those
  are out of scope; the probe checks a specific failure mode only.
- Changing any PipeWire / WirePlumber config. We cooperate with the existing
  autoswitch path.

## Detection design

### Signal

Real AirPods mic samples in a quiet room still have a noise floor that produces
non-zero int16 samples. The stuck state produces a bit-exact zero stream.
Checking for `all(byte == 0)` across a short raw capture is robust.

Reference captures from the 2026-04-23 incident give a concrete baseline for
"healthy" vs. "stuck" on AirPods Pro in a quiet room:

- Stuck: 62,400 / 62,400 bytes bit-exact zero.
- Healthy (post-reconnect, same room, same seat): 35,494 / 64,000 non-zero bytes
  (~55%), max int16 sample magnitude ~554.

If Phase 2 ever wants a richer signal than strict bit-exact zero (e.g., "fewer
than N non-zero bytes in the window"), these numbers are the starting point.

### Gating

Probe only when all of these hold:

1. The Bluetooth card's active profile starts with `headset-head-unit`.
2. The source is not muted (otherwise the zero stream is expected).
3. The per-source 20s cooldown has elapsed since the last probe, so rapid pactl
   events during call setup don't trigger back-to-back probes.

Field update (2026-04-23): the original gating also required
`source_output_count(bluez_input.<addr>) > 0` as a "has anyone actually opened a
recording stream" check. This missed a real AirPods Pro stuck-SCO recurrence
where the profile had stabilized on `headset-head-unit` with no active recorder,
yet a manual `parecord` capture returned 62,400 bit-exact zero bytes. Dropping
the source-output gate was the right call: the SCO link state is observable
regardless of whether an app is attached, and the cooldown already rate-limits
the parecord overhead.

Push-to-talk and app-level mute interaction:

- pttman (the author's PTT daemon) mutes via `pactl set-source-mute`, i.e. at
  the device level. `pactl get-source-mute` on the same source reflects it
  immediately, so the probe's mute gate correctly skips while PTT is released.
- App-level mutes (Discord mute button, Zoom mute, etc.) mute a specific
  `source-output`, not the source. `parecord` opens its own source-output, so it
  sees the raw SCO stream either way. Stuck state still reads as all-zero;
  healthy state still has noise floor.
- Known limitation: a ~10ms race between `get-source-mute` and `parecord` spawn
  means a PTT press/release straddling that window could produce a
  partial-capture. The `all_zero` predicate fails closed on any non-zero byte,
  so the only false-positive direction is "unmuted at check, muted before
  parecord reads". The 500ms capture window makes this very unlikely.

### Probe

Shell out to `parecord`:

```sh
parecord \
  --device=bluez_input.<addr> \
  --channels=1 \
  --rate=16000 \
  --format=s16le \
  --file-format=raw \
  --latency-msec=50 \
  -
```

Read ~500 ms of samples, check every byte. If all are zero, report
`ProbeResult::AllZero`. Otherwise `ProbeResult::HasSignal`.

### Confirmation

A single all-zero probe is not enough. Phase 2 will require two consecutive
all-zero probes taken N seconds apart before acting. Real silence will not stay
bit-perfect-zero across two windows; stuck SCO will.

Phase 1 logs single-probe results only, so we can validate the false-positive
rate on real systems before enabling remediation.

### Evidence capture on recurrence

When stuck SCO is observed live, dump diagnostic state to `tmp/` before journald
rotates the window out. The 2026-04-23 incident at 14:30 was already
unrecoverable from journald by 15:01; retention rotated past the event within
~30 minutes of active logging. Capture before remediating, into
`tmp/stuck-sco-<UTC-timestamp>/`:

- `stuck.raw`: a ~500ms `parecord` off the stuck source at 16kHz s16le mono
  (proof of the bit-exact-zero state).
- `journal.txt`: `journalctl --since "<T-5min>" --until "<T+1min>"`, wide enough
  to catch any `BT_PKT_SEQNUM`, `sco-io`, or HFP negotiation precursor.
- `cards.txt`, `sources.txt`: `pactl list cards` / `pactl list sources` at the
  moment of failure.
- `info.txt`: `bluetoothctl info <addr>` for link-layer state.

After running `bluetoothctl disconnect <addr> && connect` to remediate, capture
a matching `post.raw` off the same source so future signal-tuning work has a
known-good counterexample. `tmp/` is already gitignored; dumps are small (<1 MB
total). Keep them around until the next recurrence so we accumulate a comparison
set.

## Architecture

### New module `src/sco_probe.rs`

```rust
pub trait ProbeRunner {
    fn capture_raw(&self, source: &str, duration: Duration) -> Option<Vec<u8>>;
}

pub struct RealProbe;

pub enum ProbeResult {
    AllZero,
    HasSignal,
    Unavailable,
}

pub fn probe_source(runner: &dyn ProbeRunner, source: &str) -> ProbeResult;
```

`RealProbe` shells out to `parecord` with a short duration. Mockable via
`ProbeRunner` for unit tests, same pattern as `PactlRunner`.

### `pactl.rs` additions

Add a helper that returns the number of active source-outputs on a given source
name:

```rust
pub fn source_output_count(runner: &dyn PactlRunner, source: &str) -> Result<usize>;
```

Parses `pactl list source-outputs` and counts entries whose `Source: <idx>` maps
to the given source name. Cheap; the output is typically small.

### `reconcile.rs` integration

After the existing `prefer_headset_profile` and `fix_default_source` logic, add
a probe step:

```rust
if active_profile_is_hfp && !source_is_muted(runner, &source) {
    match probe_source(probe_runner, &source) {
        ProbeResult::AllZero => warn!("Probe: {} producing zero-valued samples (possible stuck SCO)", source),
        ProbeResult::HasSignal => info!("Probe: {} has signal", source),
        ProbeResult::Unavailable => debug!("Probe skipped: parecord unavailable or capture failed"),
    }
}
```

No remediation in phase 1. Just structured log output.

### Manual probe subcommand

`bthman probe` runs the same capture-and-classify path on demand, bypassing all
daemon gating except the muted-source check. Useful for reproducing stuck-SCO
interactively when the daemon's event stream has not triggered a reconcile, or
for scripting a health check. Takes an optional `--source=<name>` for targeted
probing and `--duration-ms=<N>` for a longer capture window.

### `deps.rs`

Add `parecord` to the soft-dependency list (warn if missing, don't bail):

```
warn!("parecord not found in PATH; stuck-SCO detection will be disabled");
```

### Tests

`tests/sco_probe.rs`:

- Fake `ProbeRunner` returning all-zero buffer → `AllZero`
- Fake returning any non-zero byte → `HasSignal`
- Fake returning `None` (capture failed) → `Unavailable`
- Empty buffer → `Unavailable` (can't conclude anything from zero samples)

`tests/reconcile.rs` covers:

- HFP active, probe returns `AllZero` → logs warning, does not disconnect
- HFP active, source muted → no probe
- Card on A2DP → no probe

`tests/probe_cmd.rs` covers the manual subcommand: explicit-source probe fires
even with no recorder present, muted explicit source is still skipped,
auto-discovery iterates every HFP-active card, non-HFP cards are skipped during
discovery.

## Phase 2 (out of this PR)

Superseded by `plans/stuck-sco-phase-2.md`; auto-recovery now defaults to `on`
and can be disabled with `--auto-recover-stuck-sco=off`.

Once the phase 1 logs look clean, add:

- Two-probe confirmation with N-second gap (state tracked on the card)
- `--auto-recover-stuck-sco` config flag
- Rate limiting: at most one auto-recovery per device per 5 minutes
- Minimum link uptime before allowing a recovery, to prevent loops on
  hard-broken devices
- Remediation via `bluetoothctl disconnect <addr>` followed by the existing
  `Scheduler` reconnect pipeline
- Trigger cadence beyond reconcile events. Phase 1 only probes when an event has
  already kicked `reconcile`. The 2026-04-23 14:30 incident happened after pactl
  churn had settled, so no further reconcile fired and the daemon would have
  stayed silent without manual intervention. A low-frequency background probe
  (e.g., every 60s while any card is HFP-active) closes the passive-detection
  gap and pairs naturally with the two-probe confirmation window.
- Short-HFP-window handling. The 2026-04-23 16:12 event had only a ~7-second HFP
  window (16:12:17 activate, 16:12:24 back to A2DP after the app gave up on the
  silent mic). A two-probe confirmation with a multi-second gap would have
  missed it entirely; even the single Phase 1 probe barely caught it. Options to
  consider: (a) treat a `BT_PKT_SEQNUM` log plus a single all-zero capture as
  sufficient evidence, skipping the second probe when the secondary signal is
  present; (b) keep two-probe but with a tight ~2-second gap; (c) probe
  aggressively (e.g., every 2s) for the first 3s after HFP activates and then
  fall back to the cooldown. Option (a) is the most attractive given the
  BT_PKT_SEQNUM correlation from the same event.

Remediation feasibility was validated manually on 2026-04-23:
`bluetoothctl disconnect 74:77:86:8B:31:E5 && connect` completed in under a
second, the existing reconcile picked up the new state, the profile returned to
`headset-head-unit` without intervention, and the next capture showed a healthy
noise floor. Evidence that the Phase 2 remediation path is practical end-to-end
on real hardware; no further plumbing needed beyond the flag, the confirmation
state machine, and the rate-limit/uptime guards above.

## Open questions

- Do any AirPods firmware versions legitimately transmit bit-exact zeros during
  a long silence? Empirically no (noise floor always present), but worth
  confirming with `parecord` captures during normal use before phase 2.
- Secondary signal: wireplumber emits
  `spa.bluez5.sco-io ... failed to set BT_PKT_SEQNUM` immediately before the
  stuck state. Correlation confirmed on the 2026-04-23 16:12 recurrence: the
  wireplumber log appeared at 16:12:17, bthman's reconcile probe captured
  all-zero one second later at 16:12:18. One datapoint so far; a second
  confirmation would make this solid enough to treat as a primary detector with
  parecord as fallback. Particularly useful for the short-HFP-window case (see
  Phase 2).
