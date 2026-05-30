#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_LOFT_REPO="$ROOT/../loft"
LOFT_REPO="${WHIPPLESCRIPT_LOFT_REPO:-${1:-$DEFAULT_LOFT_REPO}}"
VENV="${WHIPPLESCRIPT_LOFT_VENV:-$ROOT/target/loft-cli-venv}"
DEPS="${WHIPPLESCRIPT_LOFT_DEPS:-$ROOT/target/loft-cli-deps}"
WORKSPACE="$(mktemp -d)"
WRAPPER="$WORKSPACE/loft"

cleanup() {
  if [[ "${WHIPPLESCRIPT_KEEP_LOCAL_LOFT_SMOKE:-0}" != "1" ]]; then
    rm -rf "$WORKSPACE"
  else
    echo "Kept local Loft smoke workspace: $WORKSPACE" >&2
  fi
}
trap cleanup EXIT

if [[ ! -f "$LOFT_REPO/pyproject.toml" ]]; then
  if [[ -f "$DEFAULT_LOFT_REPO/pyproject.toml" ]]; then
    echo "Requested Loft repo is missing pyproject.toml, using $DEFAULT_LOFT_REPO instead: $LOFT_REPO" >&2
    LOFT_REPO="$DEFAULT_LOFT_REPO"
  fi
fi

if [[ ! -f "$LOFT_REPO/pyproject.toml" ]]; then
  echo "Loft repo is missing pyproject.toml: $LOFT_REPO" >&2
  exit 2
fi

LOFT_MODULE="${WHIPPLESCRIPT_LOFT_PYTHON_MODULE:-loft}"

if python3 -m venv "$VENV" >/dev/null 2>&1; then
  "$VENV/bin/python" -m pip install --quiet --upgrade pip
  "$VENV/bin/python" -m pip install --quiet --editable "$LOFT_REPO"
  cat >"$WRAPPER" <<EOF
#!/usr/bin/env bash
cd "$WORKSPACE"
exec "$VENV/bin/python" -m "$LOFT_MODULE" "\$@"
EOF
else
  rm -rf "$DEPS"
  python3 -m pip install --quiet --target "$DEPS" "$LOFT_REPO"
  cat >"$WRAPPER" <<EOF
#!/usr/bin/env bash
cd "$WORKSPACE"
export PYTHONPATH="$DEPS"
exec python3 -m "$LOFT_MODULE" "\$@"
EOF
fi
chmod +x "$WRAPPER"

"$WRAPPER" init --name whipplescript-smoke --json >/dev/null
issue="$("$WRAPPER" new "WhippleScript Loft CLI smoke" --json | jq -r '.issue.id')"
"$WRAPPER" show "$issue" --json >/dev/null
"$WRAPPER" ready --json >/dev/null
lease="$("$WRAPPER" claim --ready --actor whipplescript-smoke --ttl 30m --json | jq -r '.lease_id')"
"$WRAPPER" note "$issue" "WhippleScript smoke note" --lease-id "$lease" --json >/dev/null
printf '{"exit_code":0}\n' >"$WORKSPACE/trace-summary.json"
"$WRAPPER" evidence add "$issue" \
  --lease-id "$lease" \
  --kind whipplescript.trace \
  --artifact artifact:whipplescript/smoke/trace.json \
  --json-data trace-summary.json \
  --json >/dev/null
"$WRAPPER" set "$issue" resource_intent '{"reads":[],"writes":[]}' --lease-id "$lease" --json >/dev/null
"$WRAPPER" release "$lease" --json >/dev/null

WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=loft \
WHIPPLESCRIPT_LOFT_CLI="$WRAPPER" \
WHIPPLESCRIPT_LOFT_TEST_ISSUE="$issue" \
WHIPPLESCRIPT_LOFT_REPO="$LOFT_REPO" \
WHIPPLESCRIPT_LOFT_SKIP_REPO_PREFLIGHT=1 \
  "$ROOT/scripts/check-real-providers-report.sh"
