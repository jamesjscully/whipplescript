#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"

cargo test -p whipplescript-store cancellation_requests_are_idempotent_and_resolve_on_terminal_completion
cargo test -p whipplescript-store cancellation_request_after_terminal_completion_is_rejected
cargo test -p whipplescript-store cancellation_request_resolves_on_timeout_and_rejects_late_completion
cargo test -p whipplescript-store duplicate_terminal_completion_rolls_back_event
cargo test -p whipplescript-store contradictory_terminal_completion_rolls_back_event_even_with_distinct_key
cargo test -p whipplescript --test control_plane running_cancel_revision_requests_without_terminal_cancellation
cargo test -p whipplescript --test control_plane running_cancel_supported_provider_acknowledges_cancellation
