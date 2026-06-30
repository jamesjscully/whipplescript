#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LENGTH="${WHIPPLESCRIPT_TLA_LENGTH:-6}"

if command -v apalache-mc >/dev/null 2>&1; then
  APALACHE=(apalache-mc)
else
  if command -v nix >/dev/null 2>&1; then
    APALACHE=(nix --extra-experimental-features 'nix-command flakes' develop "$ROOT" --command apalache-mc)
  else
    echo "apalache-mc not found and nix is unavailable" >&2
    exit 1
  fi
fi

for MODEL in "$ROOT/models/tla/ControlPlaneLifecycle.tla" "$ROOT/models/tla/NativeProviderLifecycle.tla" "$ROOT/models/tla/ClockSourceLifecycle.tla" "$ROOT/models/tla/InfoflowReleaseBudget.tla" "$ROOT/models/tla/InfoflowLabelCarriage.tla"; do
  "${APALACHE[@]}" typecheck "$MODEL"
  "${APALACHE[@]}" check \
    --cinit=ConstInit \
    --init=Init \
    --next=Next \
    --inv=SafetyInvariants \
    --length="$LENGTH" \
    "$MODEL"
done
