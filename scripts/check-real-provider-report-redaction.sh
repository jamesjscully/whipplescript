#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"
SECRET="sk-test-secret-token-1234567890"

cleanup() {
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

set +e
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS="$SECRET" \
WHIPPLESCRIPT_REAL_PROVIDER_REPORT="$TMPDIR/report.md" \
WHIPPLESCRIPT_REAL_PROVIDER_PREFLIGHT_REPORT="$TMPDIR/preflight.jsonl" \
WHIPPLESCRIPT_REAL_PROVIDER_REPORT_DIR="$TMPDIR/reports" \
  "$ROOT/scripts/check-real-providers-report.sh" >"$TMPDIR/stdout.txt" 2>"$TMPDIR/stderr.txt"
status=$?
set -e

if [[ "$status" -ne 2 ]]; then
  echo "expected redaction fixture to fail provider selection with exit 2, got $status" >&2
  exit 1
fi

if grep -R -q -- "$SECRET" "$TMPDIR"; then
  echo "real-provider report redaction fixture leaked raw secret" >&2
  grep -R -n -- "$SECRET" "$TMPDIR" >&2 || true
  exit 1
fi

if [[ ! -f "$TMPDIR/reports/sk-REDACTED.json" ]]; then
  echo "expected sanitized per-provider report filename" >&2
  find "$TMPDIR" -maxdepth 2 -type f -print >&2
  exit 1
fi

if ! grep -q -- "sk-REDACTED" "$TMPDIR/report.md" "$TMPDIR/preflight.jsonl" "$TMPDIR/reports/sk-REDACTED.json"; then
  echo "expected redacted token marker in report artifacts" >&2
  exit 1
fi

echo "real-provider report redaction fixture passed"
