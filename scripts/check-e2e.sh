#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-cli -- --version >/dev/null
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-cli -- doctor >/dev/null
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel --test e2e
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-cli --test control_plane
