# bthman

Manage Bluetooth HFP profiles and reconnect headsets after suspend/resume on
Linux systems running PipeWire + WirePlumber or PulseAudio.

## The Problem

Bluetooth headsets on Linux have a couple of recurring annoyances:

1. After connecting, the HFP codec sometimes lands on LC3-24kHz, which some USB
   Bluetooth adapters (notably MediaTek `0e8d`) advertise but cannot actually
   drive, resulting in choppy or dropped mic audio. The fix is to switch the
   card profile to mSBC if the system is known to not support it.
2. When the headset negotiates HFP, the default source can end up on
   `bluez_output.<addr>.monitor` instead of the real `bluez_input.<addr>` mic,
   making the mic look dead to applications.
3. After suspend/resume, paired audio devices often disconnect and do not come
   back on their own. You have to open a tray applet and click them.
4. PipeWire frequently sets the mic input volume lower than configured.

`bthman` runs as a service (systemd or OpenRC) and handles all four.

## How It Works

- **Profile selection.** Preferred profile order defaults to `LC3-24kHz`, then
  `mSBC`, then any other HFP profile. On USB adapters in the broken-vendor list
  (MediaTek `0e8d` by default), `LC3-24kHz` is dropped so mSBC wins instead.
- **Default source.** When the card profile is an HFP variant, `bthman` ensures
  the default source is the headset's `bluez_input.<addr>` (not the `.monitor`).
- **Input volume.** Raises the default source volume to the configured target
  (100% by default) when it is below that.
- **Suspend/resume reconnect.** `dbus-monitor` watches
  `org.freedesktop.login1.PrepareForSleep`. On the suspend edge, `bthman`
  snapshots connected audio devices. On resume, it schedules reconnect attempts
  with exponential backoff (`[0.0, 0.5, 1.5, 3.5]` seconds).
- **External recorder deference.** If
  `wpctl settings bluetooth.autoswitch-to-headset-profile` is `false`, `bthman`
  assumes a recorder is holding the A2DP profile on purpose and skips the
  reconcile pass.

## Requirements

- PipeWire with WirePlumber (or PulseAudio)
- `pactl`
- `wpctl`
- `bluetoothctl`
- `dbus-monitor`
- A Linux distribution with systemd or OpenRC (elogind for OpenRC)

## Installation

### cargo (recommended)

```bash
cargo install bthman
bthman install-service
```

This installs `bthman` to `~/.cargo/bin/` and sets up the service for your init
system. To start it:

```bash
systemctl --user start bthman.service          # systemd
rc-service --user bthman start                 # OpenRC 0.60+
sudo rc-service bthman start                   # older OpenRC
```

### Prebuilt binary

Download the tarball for your platform from the
[GitHub releases page](https://github.com/mwolson/bthman/releases), extract, and
move `bthman` to `~/.local/bin/` (or `/usr/local/bin/`). Then:

```bash
bthman install-service
```

### install.sh (systemd, source build)

```bash
git clone https://github.com/mwolson/bthman.git
cd bthman
./install.sh
systemctl --user start bthman.service
```

`install.sh` runs `cargo build --release`, copies the binary to `~/.local/bin/`,
and calls `bthman install-service`.

### OpenRC system service (older OpenRC)

```bash
sudo cargo install --root /usr/local bthman
sudo bthman install-service
sudo rc-service bthman start
```

On OpenRC versions before 0.60, `install-service` installs a system-level init
script to `/etc/init.d/bthman` and adds it to the default runlevel. To configure
the user and environment for the daemon, create `/etc/conf.d/bthman`:

```sh
command_user="youruser"
supervise_daemon_args="--env XDG_RUNTIME_DIR=/run/user/1000"
```

Replace `1000` with your user's UID (`id -u youruser`).

## Usage

The service runs automatically. To check status:

#### systemd

```bash
systemctl --user status bthman.service
journalctl --user -u bthman.service -f
```

#### OpenRC (user, 0.60+)

```bash
rc-service --user bthman status
```

#### OpenRC (system, older)

```bash
rc-service bthman status
```

### Commands

```text
bthman                    Run as a daemon (default)
bthman --once             Reconcile once and exit
bthman install-service    Install and enable the service (systemd or OpenRC)
bthman uninstall-service  Disable and remove the service (systemd or OpenRC)
```

### Daemon options

These flags apply to the daemon and to `--once`. They can also be set in the
config file, one per line.

```text
--preferred-profile=PROFILE   Repeatable. Profile priority order.
--input-volume=N              Mic volume target percent (default: 100).
--broken-vendor=HEX           Repeatable. USB vendor IDs for which to skip LC3.
--debounce-ms=N               Event debounce window in ms (default: 500).
```

### Configuration file

`bthman` reads defaults from `~/.config/bthman.conf` (or
`$XDG_CONFIG_HOME/bthman.conf`). One `--flag=value` per line; blank lines and
`#` comments are allowed. Example:

```text
# skip LC3-24kHz on MediaTek BT adapters
--broken-vendor=0e8d
--preferred-profile=headset-head-unit-msbc
--input-volume=85
```

Unrecognized flags cause an error at startup. Command-line arguments always take
precedence over the config file.

Send `SIGHUP` to the running daemon (or `systemctl --user reload bthman`) to
reload the config file without restarting. Parse errors are logged at WARN and
the previous config keeps running.

## Uninstall

```bash
bthman uninstall-service
cargo uninstall bthman       # or: rm ~/.local/bin/bthman
rm -f ~/.config/bthman.conf
```

## Development

The Rust toolchain is pinned via `rust-toolchain.toml`; `rustup` auto-installs
it (with `clippy`, `rustfmt`, and `rust-analyzer`) on your first `cargo`
invocation in the project directory.

```bash
bun run test                # unit tests
bun run test:integration    # Docker-based integration tests
bun run test:all            # both
```

### Hooks

```bash
bun run hooks:check         # run checks against working tree
lefthook install            # install git hooks
```

The pre-commit hook runs `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.

## License

MIT
