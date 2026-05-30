#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

TARGET="${1:-vendor/loft}"
SUBMODULE="$ROOT/$TARGET"

missing=0
fixture_files=()

if [[ ! -d "$SUBMODULE" ]]; then
  echo "Loft submodule directory is missing: $TARGET" >&2
  exit 2
fi

if ! git -C "$SUBMODULE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Loft submodule path is not a git worktree: $TARGET" >&2
  exit 2
fi

if ! git -C "$ROOT" submodule status -- "$TARGET" >/dev/null 2>&1; then
  echo "Loft path is not registered as a git submodule: $TARGET" >&2
  missing=1
fi

if ! git -C "$SUBMODULE" ls-files --error-unmatch spec/loft-v0.1.md >/dev/null 2>&1; then
  echo "Loft submodule is missing tracked spec/loft-v0.1.md" >&2
  missing=1
fi

manifest_path="$SUBMODULE/$LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST"
if [[ ! -f "$manifest_path" ]]; then
  echo "Loft submodule is missing fixture manifest: $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  missing=1
elif ! git -C "$SUBMODULE" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >/dev/null 2>&1; then
  echo "Loft submodule is missing tracked $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  missing=1
else
  mapfile -t fixture_files < <(loft_fixture_files "$SUBMODULE/$LOFT_FIXTURE_DIR")
  if [[ "${#fixture_files[@]}" -eq 0 ]]; then
    echo "Loft submodule fixture manifest has no fixtures: $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
    missing=1
  fi
fi

for fixture in "${fixture_files[@]}"; do
  if ! git -C "$SUBMODULE" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$fixture" >/dev/null 2>&1; then
    echo "Loft submodule is missing tracked $LOFT_FIXTURE_DIR/$fixture" >&2
    missing=1
  fi
done

if [[ -n "$(git -C "$SUBMODULE" status --short)" ]]; then
  echo "Loft submodule has uncommitted changes:" >&2
  git -C "$SUBMODULE" status --short >&2
  missing=1
fi

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi

WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1 \
  "$ROOT/scripts/check-loft-fixtures.sh"

echo "Loft submodule is ready as WhippleScript source of truth: $TARGET"
