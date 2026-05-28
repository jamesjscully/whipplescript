#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v apalache-mc >/dev/null 2>&1; then
  if command -v nix >/dev/null 2>&1; then
    exec nix --extra-experimental-features 'nix-command flakes' develop "$ROOT" --command "$0"
  fi

  echo "apalache-mc not found and nix is unavailable" >&2
  exit 1
fi

apalache-mc typecheck "$ROOT/models/tla/ControlPlaneLifecycle.tla"
