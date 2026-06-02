#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  step_materializes_minimal_noop_fact
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  old_effect_runs_after_keep_revision_with_old_attribution
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_phase_review_creates_requests_and_runs_fixture_turns
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_provider_language_e2e_runs_agent_matrix_and_baml_reviews
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_native_fixture_records_provider_lifecycle_and_artifacts_from_source_workflow

echo "control-plane driver checks passed"
