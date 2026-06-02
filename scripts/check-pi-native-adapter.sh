#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo test -p whipplescript-kernel pi_rpc --lib
cargo test -p whipplescript-kernel normalizes_pi_aborted_turn_end --lib
