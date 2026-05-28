#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-cli -- --version >/dev/null
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-cli -- doctor >/dev/null
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-kernel --test e2e
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-cli --test control_plane
