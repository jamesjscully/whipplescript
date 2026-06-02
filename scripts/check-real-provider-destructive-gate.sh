#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"
EXPECTED_ACK="I_UNDERSTAND_THIS_PROVIDER_TARGET_IS_DISPOSABLE"
SECRET_TARGET="codex-disposable-sk-test-secret-token-1234567890"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

run_gate() {
  local name="$1"
  shift
  set +e
  WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
  WHIPPLESCRIPT_REAL_PROVIDERS=codex \
  WHIPPLESCRIPT_REAL_PROVIDER_FIXTURE_GATE_ONLY=1 \
  WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT="$TMPDIR/$name.jsonl" \
    "$@" "$ROOT/scripts/check-real-providers.sh" \
    >"$TMPDIR/$name.stdout" 2>"$TMPDIR/$name.stderr"
  local status=$?
  set -e
  printf '%s' "$status"
}

skip_status="$(run_gate skip env)"
if [[ "$skip_status" -ne 0 ]]; then
  echo "expected non-destructive fixture gate run to pass, got $skip_status" >&2
  exit 1
fi
if ! rg -q '"phase":"destructive.fixture.skip".*"status":"skip"' "$TMPDIR/skip.jsonl"; then
  echo "expected destructive fixture skip record" >&2
  cat "$TMPDIR/skip.jsonl" >&2
  exit 1
fi

missing_status="$(run_gate missing env WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS=1)"
if [[ "$missing_status" -ne 2 ]]; then
  echo "expected missing disposable marker run to fail with exit 2, got $missing_status" >&2
  exit 1
fi
if ! rg -q '"phase":"destructive.fixture.missing".*"check":"disposable-target".*"status":"fail"' "$TMPDIR/missing.jsonl"; then
  echo "expected missing disposable target failure record" >&2
  cat "$TMPDIR/missing.jsonl" >&2
  exit 1
fi

pass_status="$(run_gate pass env \
  WHIPPLESCRIPT_CODEX_DESTRUCTIVE_TESTS=1 \
  "WHIPPLESCRIPT_CODEX_DISPOSABLE_TARGET=$SECRET_TARGET" \
  "WHIPPLESCRIPT_CODEX_DISPOSABLE_ACK=$EXPECTED_ACK")"
if [[ "$pass_status" -ne 0 ]]; then
  echo "expected acknowledged disposable marker run to pass, got $pass_status" >&2
  exit 1
fi
if ! rg -q '"phase":"destructive.fixture.ok".*"check":"disposable-target".*"status":"pass"' "$TMPDIR/pass.jsonl"; then
  echo "expected disposable target pass record" >&2
  cat "$TMPDIR/pass.jsonl" >&2
  exit 1
fi
if rg -q "$SECRET_TARGET" "$TMPDIR"; then
  echo "destructive fixture gate leaked disposable target value" >&2
  rg -n "$SECRET_TARGET" "$TMPDIR" >&2 || true
  exit 1
fi

echo "real-provider destructive fixture gate checks passed"
