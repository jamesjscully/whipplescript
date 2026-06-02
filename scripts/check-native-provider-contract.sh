#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"

cargo test -p whipplescript-kernel builtin_capabilities_capture_distinct_native_surfaces --lib
cargo test -p whipplescript-kernel parses_valid_codex_provider_binding_config --lib
cargo test -p whipplescript-kernel native_turn_request_redacts_prompt_and_provider_options --lib
cargo test -p whipplescript-kernel native_provider_event_preserves_shape_without_raw_payload --lib
cargo test -p whipplescript-kernel native_provider_boundary_error_redacts_message_and_evidence --lib
cargo test -p whipplescript-kernel native_adapter_trait_supports_distinct_start_stream_and_cancel_events --lib
cargo test -p whipplescript-kernel run_native_agent_turn_records_lifecycle_artifacts_and_terminal --lib
cargo test -p whipplescript-kernel cancellation_depth_guard --lib
