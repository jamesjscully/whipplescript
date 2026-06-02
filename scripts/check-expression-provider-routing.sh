#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-parser \
  agent_ref
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript-parser \
  validates_provider_matrix_guard_expressions
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_evaluates_shared_expression_kernel_for_guards_and_assertions
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --test control_plane \
  dev_provider_language_e2e_runs_agent_matrix_and_baml_reviews
cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript \
  generated_model_search_runs_lowered_expression_fixture

echo "expression provider-routing checks passed"
