#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel policy_rejects_
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_native_provider_records_policy_denial_from_source_required_capabilities

echo "native provider policy denial checks passed"
