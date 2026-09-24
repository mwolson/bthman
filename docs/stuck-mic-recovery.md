# Stuck mic recovery

Some Bluetooth headsets, notably AirPods, can leave the HFP (call mode) uplink
stuck. The card still reports `headset-head-unit`, `bluez_input.<addr>` still
runs, and playback still works, but no microphone audio reaches the computer.
The only known recovery is a full disconnect and reconnect at the BlueZ layer
(`bluetoothctl disconnect <addr>`, then `connect`). Switching the card profile
on a live link does not recover it and can leave the link failed.

bthman detects the stuck state and performs that reconnect for you.

## How a stuck link looks

PipeWire exposes a stuck uplink in one of three ways, depending on the codec and
how the link failed:

| Symptom     | What the capture sees                                      |
| ----------- | ---------------------------------------------------------- |
| All zero    | Every sample is bit-exact zero. Common with CVSD.          |
| No data     | The capture delivers nothing within the deadline.          |
| Concealment | Low-level synthetic noise. PipeWire fills missing packets. |

The third case matters for LC3 and mSBC. When packets stop arriving, PipeWire
runs the codec's packet loss concealment (PLC) on every audio cycle, so a stuck
mic sounds like a very quiet one rather than digital silence. A level check
cannot tell that apart from a working mic in a quiet room, because headsets such
as AirPods suppress background noise heavily. bthman instead watches PipeWire's
own record of concealment, described below.

## Probing

When `--probe-stuck-sco=true` (the default), each reconcile pass runs a 500 ms
`parecord` capture of every Bluetooth source that is on an HFP profile and not
muted. A muted source always reads as zeros, so it is skipped.

A probe of the same source runs at most once every 20 seconds. These triggers
skip the cooldown:

- A new source appears (`Event 'new' on source`).
- The source was muted on the previous pass and is now unmuted.
- A silent capture is waiting for confirmation, and any source event arrives.
- Sustained concealment is reported by the log watcher.

Other `change` events on a source do not skip the cooldown. A probe's own
capture moves the source between idle and running, which emits `change` events,
so letting those through made the daemon probe itself about once a second and
hold the headset in call mode.

`parecord` is a soft dependency. If it is missing from `PATH`, probing is
disabled and a warning is logged at startup.

## Confirming a stuck link

One silent capture is never enough to reconnect. bthman needs two, taken about 2
seconds apart and no more than 3 seconds apart:

- **Tier 2:** two silent captures, where silent means all zero or no data.
- **Tier 3:** the confirming capture is non-zero but taken during sustained
  concealment, and the capture before it was silent or also concealed.

A silent capture in the first 3 seconds after the card enters HFP still
schedules the 2-second follow-up probe, but it does not count toward
confirmation. A link that is still starting can briefly deliver no audio.

There is no Tier 1. An earlier design treated WirePlumber's
`failed to set BT_PKT_SEQNUM` line as supporting evidence, but that line appears
every time an HFP link opens on kernels without that socket option. Paired with
startup silence it reconnected healthy links.

After a reconnect, bthman will not reconnect the same device again for 5
minutes. The reconnect runs through the same backoff schedule as resume
reconnects: attempts at 0, 0.5, 1.5, and 3.5 seconds.

### Recovery modes

`--auto-recover-stuck-sco` controls what happens once a link is confirmed stuck:

| Mode      | Behavior                                                                             |
| --------- | ------------------------------------------------------------------------------------ |
| `on`      | Disconnect and reconnect the device. This is the default.                            |
| `dry-run` | Log the decision and start the 5-minute rate limit, but do not touch the connection. |
| `off`     | Log probe warnings only. No decisions are made or logged.                            |

## Concealment detection

PipeWire logs `produced PLC audio` once per concealed codec block. When no
packets arrive, that is about 133 lines per second for LC3 and mSBC. A healthy
link logs none, whether or not anyone is talking.

bthman tails WirePlumber's journal for those lines. When at least 50 land within
one second, it probes the source right away and repeats at most every 2 seconds
while concealment continues. A capture taken during sustained concealment counts
as silent even if it is not exactly zero.

### WirePlumber logging

The concealment lines are DEBUG level for the `spa.bluez5.source.media` topic,
which WirePlumber does not log by default. Enable that one topic and leave
everything else at WirePlumber's default WARN level:

```sh
systemctl --user edit wireplumber.service
```

```ini
[Service]
Environment=WIREPLUMBER_DEBUG=W,spa.bluez5.source.media:D
```

Restart WirePlumber (`systemctl --user restart wireplumber.service`) when no
call is active, or apply the same level to the running daemon:

```sh
wpctl set-log-level "W,spa.bluez5.source.media:D"
```

While an HFP link is up, the topic logs about one line per second, plus the
concealment lines during a failure. Raising WirePlumber to INFO or DEBUG overall
is not required. INFO adds routing chatter, and full DEBUG is noisy enough to
make unrelated audio problems harder to diagnose.

At startup bthman looks for DEBUG lines from this topic in WirePlumber's journal
for the current boot. If it finds none, it logs `concealment_unconfirmed`. That
also happens after a fresh boot with no Bluetooth call yet, so check again after
one.

Without this setting, or on hosts without journald such as OpenRC systems, Tier
2 still recovers links that go fully silent.

## Tools that manage call mode themselves

If `wpctl settings bluetooth.autoswitch-to-headset-profile` is `false`, bthman
assumes another tool is managing the headset profile on purpose, such as a
push-to-talk recorder or a mic check. It still corrects the HFP codec choice,
but skips the default source, volume, and probe steps until the setting is
`true` again.

## Manual probe

`bthman probe` runs the same capture and classification once, then exits. It
skips the cooldown and the external recorder check. With no arguments it probes
every Bluetooth card on an HFP profile. `--source=<name>` probes one source, and
`--duration-ms=<N>` overrides the 500 ms capture window. Muted sources are
skipped.

The manual probe reports all-zero and no-data captures. It does not watch the
journal, so it cannot report concealment.
