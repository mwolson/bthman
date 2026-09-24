# Development

## Toolchain

The Rust toolchain is pinned in `rust-toolchain.toml`. `rustup` installs it,
with `clippy`, `rustfmt`, and `rust-analyzer`, on the first `cargo` command in
the project directory.

The crate is a single binary with a few focused modules. It shells out to
`pactl`, `wpctl`, `bluetoothctl`, `dbus-monitor`, `parecord`, and `journalctl`
instead of linking audio or D-Bus libraries.

## Building and running

```sh
cargo build --release     # produces target/release/bthman
cargo run -- once         # reconcile once against the live system
cargo run                 # run as a daemon in the foreground
```

## Tests

```sh
bun run test                # unit tests (cargo test)
bun run test:integration    # Docker-based integration tests
bun run test:all            # both
```

Unit tests live in `tests/` and exercise each module's public API with fake
`pactl`, probe, and Bluetooth runners.

Integration tests under `integration-tests/` build three images: systemd on
Debian bookworm, an OpenRC user service on Alpine edge, and an OpenRC system
service on Alpine 3.21. They run the real binary against fake `pactl`,
`bluetoothctl`, `dbus-monitor`, and `wpctl` scripts in
`integration-tests/shared/`. They need Docker and take several minutes, so they
stay out of the pre-commit hook. CI runs them before every tagged release.

## Hooks

```sh
bun run hooks:check         # run every check against the working tree
lefthook install            # install the git hooks
```

The pre-commit hook runs Prettier on Markdown, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test`.
