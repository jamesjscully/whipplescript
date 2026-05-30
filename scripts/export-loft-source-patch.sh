#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

LOFT_REPO="${1:-}"
PATCH_PATH="${2:-$ROOT/target/loft-source-fixtures.patch}"
OVERWRITE="${WHIPPLESCRIPT_OVERWRITE_LOFT_PATCH:-0}"

if [[ -z "$LOFT_REPO" ]]; then
  echo "usage: scripts/export-loft-source-patch.sh <local-loft-repo> [patch-path]" >&2
  exit 2
fi

if [[ ! -d "$LOFT_REPO/.git" ]]; then
  echo "not a local git repo: $LOFT_REPO" >&2
  exit 2
fi

if [[ -e "$PATCH_PATH" && "$OVERWRITE" != "1" ]]; then
  echo "patch already exists; set WHIPPLESCRIPT_OVERWRITE_LOFT_PATCH=1 to replace: $PATCH_PATH" >&2
  exit 2
fi

"$ROOT/scripts/stage-loft-fixtures.sh" "$LOFT_REPO" >/dev/null

SPEC_PATH="spec/loft-v0.1.md"

if ! git -C "$LOFT_REPO" diff --cached --quiet -- "$SPEC_PATH" "$LOFT_FIXTURE_DIR"; then
  echo "Loft repo has staged changes under $SPEC_PATH or $LOFT_FIXTURE_DIR" >&2
  echo "Commit or unstage those changes before exporting a patch." >&2
  exit 2
fi

mkdir -p "$(dirname "$PATCH_PATH")"

cleanup() {
  git -C "$LOFT_REPO" reset -q -- "$SPEC_PATH" "$LOFT_FIXTURE_DIR" >/dev/null 2>&1 || true
}
trap cleanup EXIT

git -C "$LOFT_REPO" add --intent-to-add -- "$SPEC_PATH" "$LOFT_FIXTURE_DIR"
git -C "$LOFT_REPO" diff --binary -- "$SPEC_PATH" "$LOFT_FIXTURE_DIR" >"$PATCH_PATH"

if [[ ! -s "$PATCH_PATH" ]]; then
  echo "Loft source patch is empty: $PATCH_PATH" >&2
  exit 2
fi

echo "Wrote Loft source patch: $PATCH_PATH"
