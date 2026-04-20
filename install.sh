#!/bin/bash

set -euo pipefail

for cmd in cargo; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: '$cmd' is required but not found in PATH." >&2
        exit 1
    fi
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

BIN_DIR="$HOME/.local/bin"

echo "Building bthman (release)..."
cargo build --release

mkdir -p "$BIN_DIR"
cp "$SCRIPT_DIR/target/release/bthman" "$BIN_DIR/bthman"
chmod +x "$BIN_DIR/bthman"
echo "  Installed $BIN_DIR/bthman"

echo ""
echo "Running bthman install-service..."
exec "$BIN_DIR/bthman" install-service
