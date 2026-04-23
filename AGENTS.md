# Agent Instructions

## Project overview

bthman (bluetooth-headset-manager) is a Rust daemon that manages Bluetooth HFP
profiles on Linux and reconnects paired audio devices after suspend/resume. It
monitors PulseAudio/PipeWire card events via `pactl --format=json subscribe` and
D-Bus `PrepareForSleep` signals via `dbus-monitor`.

## Planning

Prefer to write plans in the `plans/` directory.

## Conventions

- Single-binary Rust crate with a small number of focused modules.
- Minimal dependencies. No tokio, no zbus. Shell out to `pactl`, `wpctl`,
  `bluetoothctl`, and `dbus-monitor` instead.
- Keep code comments minimal.
- When making changes to data in existing code, try to keep things in
  alphabetical order when it's reasonable to do so.
- Prefer top-down control flow: caller first, then callee.
- When writing bash scripts: `#!/bin/bash`, 4-space indentation, fail-fast
  dependency checks.

## Diagnosing live incidents

When the user reports a live Bluetooth/audio issue, capture evidence to `tmp/`
immediately, before journald rotates the window out. On the author's machine,
journald has been observed to lose ~30 minutes of history within a few minutes
of active logging, so "I'll grab the logs later" does not work. Minimum set:

- `journalctl --since "<T-5min>" --until "<T+1min>" > tmp/journal-<ts>.txt`
- `pactl list cards > tmp/cards-<ts>.txt`
- `pactl list sources > tmp/sources-<ts>.txt`
- For suspected stuck-SCO or mic-silence issues: a short `parecord` dump off the
  affected source plus `bluetoothctl info <addr>`.

`tmp/` is gitignored. Prefer verbose over sparse when capturing; pruning later
is cheap. Specific incident classes have their own checklists in `plans/` (e.g.,
`plans/stuck-sco-detection.md` §"Evidence capture on recurrence").

## Key files

- `src/main.rs` -- clap parse, dispatch to subcommand
- `src/cli.rs` -- `Cli` and `Overrides` structs
- `src/config.rs` -- `~/.config/bthman.conf` parser and `Config` struct
- `src/daemon.rs` -- watch-mode outer loop, event select
- `src/reconcile.rs` -- core profile + source + volume reconciliation
- `src/pactl.rs` -- `PactlRunner` trait + card/source parsing
- `src/wpctl.rs` -- external recorder detection via `wpctl settings`
- `src/bluetoothctl.rs` -- paired/info/connect primitives
- `src/sleep_monitor.rs` -- `dbus-monitor` reader + sleep edge classifier
- `src/reconnect.rs` -- `Scheduler` with exponential backoff
- `src/vendor_detect.rs` -- `/sys/class/bluetooth` USB vendor lookup
- `src/events.rs` -- `DaemonEvent` + `SignalKind` enums
- `src/service/` -- install/uninstall dispatch by init system
- `src/service_files.rs` -- `include_str!` copies of unit/init scripts
- `systemd/bthman.service` -- systemd user service
- `openrc-user/bthman` -- OpenRC 0.60+ user init script
- `openrc-system/bthman` -- OpenRC pre-0.60 system init script
- `install.sh` -- convenience installer for source builds

## Dev loop tools

### Toolchain

The Rust toolchain is pinned via `rust-toolchain.toml` (channel, profile,
components). `rustup` auto-installs it on first `cargo` invocation, so editors
pick up a version-matched `rust-analyzer` with no manual steps. Bump the pin
deliberately; CI installs the pinned toolchain via
`rustup show active-toolchain || rustup toolchain install`.

### Running tests

```sh
bun run test                # unit tests (cargo test)
bun run test:integration    # Docker-based integration tests
bun run test:all            # both
```

Unit tests live in `tests/` and exercise the public API of each module.
Integration tests under `integration-tests/` run three Dockerfiles (systemd on
Debian bookworm, OpenRC user on Alpine edge, OpenRC system on Alpine 3.21) and
shell out to fake binaries under `integration-tests/shared/`.

### Building

```sh
cargo build --release            # produces target/release/bthman
cargo run -- --once              # reconcile once against the live system
cargo run                        # run as daemon (default)
```

### Pre-commit hooks

Lefthook runs these in parallel (see `lefthook.yaml`):

- `md-format` -- Prettier formatting for Markdown files
- `cargo-fmt` -- `cargo fmt --all -- --check`
- `cargo-clippy` -- `cargo clippy --all-targets -- -D warnings`
- `cargo-test` -- full unit test suite

Run checks against the working tree (no staging required):

```sh
bun run hooks:check
```

Integration tests are intentionally kept out of pre-commit; they take minutes
and need Docker. CI runs them in `publish.yml` before every tag release.

## Releasing

### Pre-release steps

1. Check for uncommitted changes:

   ```sh
   git status
   ```

   If there are uncommitted changes, offer to commit them before proceeding.

2. Fetch latest tags:

   ```sh
   git fetch --tags
   ```

3. Run `bun run hooks:check` and confirm everything passes. CI (see
   `.github/workflows/publish.yml`) runs on tag push, so this is the last
   opportunity to catch lint, format, and unit-test failures before publish. The
   pre-commit hook alone is not enough: its `glob` gate filters on
   `{staged_files}`, which can silently skip entire check groups when the staged
   set does not match.

4. Update the version in `Cargo.toml` and `package.json`. Run
   `cargo update -p bthman` to refresh `Cargo.lock`. The `VERSION` constant in
   the binary is derived from `env!("CARGO_PKG_VERSION")`, so it cannot drift.
   Commit all three files with message `chore: bump version to <version>`. This
   must be its own commit, not combined with other changes, unless the user
   explicitly agrees to that.

5. Push the version-bump commit:

   ```sh
   git push
   ```

   CI runs on push to `main` via `ci.yml` (fmt + clippy + test). The tag-push
   `publish.yml` adds integration tests and the crates.io publish.

### Creating the release

When the user provides a version (or indicates major/minor/bugfix):

1. Create and push the tag:

   ```sh
   git tag v<version>
   git push origin v<version>
   ```

2. Wait for `publish.yml` to pass before drafting release notes. This run is
   what publishes to crates.io, so if it fails the release is incomplete:

   ```sh
   gh run list --limit 1         # grab the run id
   gh run watch <run-id> --exit-status
   ```

   If CI fails, fix the issue on `main`, delete the tag locally and remotely
   (`git push origin :refs/tags/v<version> && git tag -d v<version>`), re-tag,
   and push again.

3. Examine each commit since the last tag:

   ```sh
   git log <previous-tag>..HEAD --oneline
   ```

   For each commit, run `git show <commit>` to see the full commit message and
   diff.

4. The publish workflow creates the GitHub release as a draft with the three
   platform tarballs attached and auto-generated notes. Confirm it exists:

   ```sh
   gh release view v<version>
   ```

5. Enhance the draft's release notes with more context:
   - Use insights from examining each commit in step 3
   - Group related changes under descriptive headings (e.g., "### Refactored X",
     "### Fixed Y")
   - Use bullet lists within each section to describe the changes
   - Include a brief summary of what changed and why it matters
   - Keep the "Full Changelog" link at the bottom
   - Update the release with `gh release edit v<version> --notes "..."`

   Ordering guidelines:
   - Put user-visible changes first (new features, bug fixes, breaking changes)
   - Put under-the-hood changes later (refactoring, internal improvements, docs)
   - Within each section, order by user impact (most impactful first)

6. Publish the release (flip off the draft flag):

   ```sh
   gh release edit v<version> --draft=false
   ```

7. Tell the user to review the published release and provide a link:

   ```
   https://github.com/mwolson/bthman/releases
   ```
