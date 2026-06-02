#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"

cargo test -p whipplescript --test control_plane doctor_providers_reports_deterministic_health_posture
cargo test -p whipplescript --test control_plane starts_and_inspects_two_instances_independently
cargo test -p whipplescript --test control_plane artifacts_command_lists_metadata_without_raw_content
cargo test -p whipplescript --test control_plane running_cancel_revision_requests_without_terminal_cancellation
cargo test -p whipplescript --test control_plane operator_incident_bundle_has_stable_status_trace_and_diagnostics_shape
cargo test -p whipplescript native_lifecycle_summary_exposes_redacted_status_for_runs
