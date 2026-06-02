#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo test -p whipplescript-kernel codex_app_server --lib
cargo test -p whipplescript-kernel normalizes_codex_started_diff_tool_and_cancelled_terminal --lib
