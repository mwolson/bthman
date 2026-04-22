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

### Gating

Probe only when all of these hold:

1. The Bluetooth card's active profile starts with `headset-head-unit`.
2. At least one source-output exists on `bluez_input.<addr>`, i.e. an
   application has actually opened a recording stream. No point probing a
   dormant mic.
3. The source is not muted (otherwise the zero stream is expected).
4. A short settle delay has elapsed since the last profile change for this card,
   so we don't probe mid-transition.

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
if active_profile_is_hfp
    && source_output_count(runner, &source) > 0
    && !source_is_muted(runner, &source)
{
    match probe_source(probe_runner, &source) {
        ProbeResult::AllZero => warn!("Probe: {} producing zero-valued samples (possible stuck SCO)", source),
        ProbeResult::HasSignal => info!("Probe: {} has signal", source),
        ProbeResult::Unavailable => debug!("Probe skipped: parecord unavailable or capture failed"),
    }
}
```

No remediation in phase 1. Just structured log output.

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

`tests/reconcile.rs`: extend the fake `PactlRunner` to serve
`pactl list source-outputs` output; add cases for:

- HFP active, no source-outputs → no probe
- HFP active, source-output present, probe returns `AllZero` → logs warning,
  does not disconnect
- Card on A2DP → no probe regardless of source-outputs

## Phase 2 (out of this PR)

Once the phase 1 logs look clean, add:

- Two-probe confirmation with N-second gap (state tracked on the card)
- `--auto-recover-stuck-sco` config flag (default off)
- Rate limiting: at most one auto-recovery per device per 5 minutes
- Minimum link uptime before allowing a recovery, to prevent loops on
  hard-broken devices
- Remediation via `bluetoothctl disconnect <addr>` followed by the existing
  `Scheduler` reconnect pipeline

## Open questions

- Do any AirPods firmware versions legitimately transmit bit-exact zeros during
  a long silence? Empirically no (noise floor always present), but worth
  confirming with `parecord` captures during normal use before phase 2.
- Does Waydroid's source-output count as an "active recording" even when the app
  is muted application-side? Phase 1 logs will tell us.
