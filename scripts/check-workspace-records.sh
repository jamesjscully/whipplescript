#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"

cargo test -p whipplescript-store workspace --lib
cargo test -p whipplescript-kernel rejects_unknown_workspace_policy --lib
cargo test -p whipplescript-kernel policy_rejects_unsupported_workspace_policy --lib
