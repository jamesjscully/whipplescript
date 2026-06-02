#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo test -p whipplescript-kernel claude_agent_sdk --lib
cargo test -p whipplescript-kernel normalizes_claude_terminal_events --lib
