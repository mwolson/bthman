# Troubleshooting

## Watch the daemon

```sh
journalctl --user -u bthman.service -f
```

## The call cannot hear me

1. Check that the mic is not muted in PipeWire:
   `pactl get-source-mute @DEFAULT_SOURCE@`. Muting inside a call app does not
   show up here, and bthman does not probe a muted source.
2. Check that the default source is the headset mic (`bluez_input.<addr>`)
   rather than a `.monitor` or another device: `pactl get-default-source`.
3. Speak while you record a few seconds from the mic. A working mic peaks in the
   thousands on a 16-bit scale while you talk. A quiet room alone reads close to
   zero on headsets with strong noise suppression, so a silent test proves
   nothing.

   ```sh
   parecord --device=@DEFAULT_SOURCE@ --channels=1 --rate=16000 \
     --format=s16le --file-format=raw /dev/stdout | head -c 96000 > mic.raw
   python3 -c 'import array; a = array.array("h", open("mic.raw", "rb").read()); print(max(map(abs, a)))'
   ```

4. If your voice does not come through, disconnect and reconnect the headset
   (`bluetoothctl disconnect <addr>`, then `bluetoothctl connect <addr>`). Avoid
   switching the card profile back and forth on a live link: that has left links
   stuck or failed.
5. If that does not help, put the earbuds back in their case for about 10
   seconds, or power-cycle the headset. That resets the headset itself, which
   reconnecting from the computer cannot do.

To find out why bthman did not recover it on its own, read its decisions below.

## Reading `stuck_sco_decision` lines

Every recovery decision is logged as one line:

```text
stuck_sco_decision: source=bluez_input.AA:BB:CC:DD:EE:FF tier=2 all_zero_count=2 uptime_s=7 rate_limited=false action=remediate addr=AA:BB:CC:DD:EE:FF
```

| Field            | Meaning                                                 |
| ---------------- | ------------------------------------------------------- |
| `tier`           | `2` or `3` when the link is confirmed stuck, else `na`. |
| `all_zero_count` | Silent captures counted toward this decision.           |
| `uptime_s`       | Seconds since bthman first saw the card on HFP.         |
| `rate_limited`   | Whether a recent reconnect blocked this one.            |
| `action`         | What the daemon did.                                    |

| Action                    | Meaning                                                            |
| ------------------------- | ------------------------------------------------------------------ |
| `arm_follow_up`           | First silent capture. A follow-up probe runs in 2 seconds.         |
| `remediate`               | Confirmed stuck. The device is being disconnected and reconnected. |
| `would_remediate`         | Confirmed stuck in `dry-run` mode. Nothing was touched.            |
| `skip_min_uptime`         | The link has been in HFP for under 3 seconds.                      |
| `skip_muted`              | The source is muted, so it was not probed.                         |
| `skip_in_progress`        | A reconnect for this device is already running.                    |
| `skip_profile_changed`    | The card left HFP between the probe and the decision.              |
| `skip_recent_remediation` | The device was reconnected in the last 5 minutes.                  |

A failed reconnect logs `stuck_sco_remediation_failed` with the stage that
failed.

## `Probe: ... has signal` but nobody can hear me

A non-zero capture is not proof of a working mic. With LC3 and mSBC, a stuck
link produces low-level concealment audio instead of zeros. bthman catches that
case only when WirePlumber logs the `spa.bluez5.source.media` topic at DEBUG.
See [WirePlumber logging](stuck-mic-recovery.md#wireplumber-logging).

## `concealment_unconfirmed` at startup

bthman found no DEBUG lines from `spa.bluez5.source.media` in WirePlumber's
journal for the current boot. Either the WirePlumber setting is missing, or no
Bluetooth call has happened since boot. Check WirePlumber's environment:

```sh
systemctl --user show wireplumber.service -p Environment
```

It should include `WIREPLUMBER_DEBUG=W,spa.bluez5.source.media:D`.

## The headset stays in call mode after a call

bthman only probes a source that is already on HFP, and it holds the source only
for the 500 ms capture. If the headset stays in HFP long after every app has
stopped recording, look for another recorder (`pactl list source-outputs`) or
for `wpctl settings bluetooth.autoswitch-to-headset-profile` left at `false` by
another tool.

## Capturing evidence

Journald can rotate recent history out within minutes of heavy logging, so
capture evidence as soon as a problem happens:

```sh
ts=$(date +%Y%m%d-%H%M)
journalctl --since "-10min" > "journal-$ts.txt"
pactl list cards > "cards-$ts.txt"
pactl list sources > "sources-$ts.txt"
bluetoothctl info <addr> > "bluetooth-$ts.txt"
```

For a silent mic, also save a short `parecord` capture from the affected source
while speaking.
