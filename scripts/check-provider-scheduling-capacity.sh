#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
  program_agent_declarations_drive_capacity_blocks
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
  kernel_expires_leases_and_retries_failed_effects
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-kernel \
  restart_recovers_running_pi_native_run_from_terminal_evidence
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_native_provider_launch_failure_records_durable_boundary_failure
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_native_fixture_stress_records_one_terminal_per_effect
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  running_cancel_supported_provider_acknowledges_cancellation

echo "provider scheduling and capacity checks passed"
