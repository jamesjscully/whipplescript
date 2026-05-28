#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p armature-cli -- --version >/dev/null
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p armature-cli -- doctor >/dev/null
