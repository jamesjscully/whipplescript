#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"

cargo test -p whipplescript-store replay_reconstructs_active_revision_cancelled_effects_and_requests
cargo test -p whipplescript-store replay_reconstructs_terminal_runs_leases_and_resolved_cancel_requests
cargo test -p whipplescript-store replay_reconstructs_expired_leases_without_reclaiming_cancel_requested_effects
cargo test -p whipplescript-store reconstructs_terminal_diagnostics_from_event_payloads
cargo test -p whipplescript-kernel mock_agent_harness_records_artifacts_evidence_and_turn_fact
cargo test -p whipplescript-kernel recovery_appends_terminal_from_provider_evidence_once_after_append_gap
cargo test -p whipplescript-kernel recovery_preserves_artifact_evidence_after_capture_before_terminal_gap
cargo test -p whipplescript-kernel restart_reconciles_running_provider_run_with_persisted_terminal_evidence
