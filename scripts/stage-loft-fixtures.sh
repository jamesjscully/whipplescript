#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

LOFT_REPO="${1:-}"
SOURCE_FIXTURE_DIR="$ROOT/examples/loft-fixtures/v0.1"
OVERWRITE="${WHIPPLETREE_OVERWRITE_LOFT_FIXTURES:-0}"

if [[ -z "$LOFT_REPO" ]]; then
  echo "usage: scripts/stage-loft-fixtures.sh <local-loft-repo>" >&2
  exit 2
fi

if [[ ! -d "$LOFT_REPO/.git" ]]; then
  echo "not a local git repo: $LOFT_REPO" >&2
  exit 2
fi

SPEC_PATH="spec/loft-v0.1.md"

if [[ ! -f "$LOFT_REPO/$SPEC_PATH" ]]; then
  echo "Loft repo is missing spec/loft-v0.1.md" >&2
  exit 2
fi

mapfile -t fixture_files < <(loft_fixture_files "$SOURCE_FIXTURE_DIR")

if [[ "${#fixture_files[@]}" -eq 0 ]]; then
  echo "Whippletree compatibility fixture manifest has no fixtures: $(loft_manifest_path "$SOURCE_FIXTURE_DIR")" >&2
  exit 2
fi

for fixture in "$LOFT_FIXTURE_MANIFEST" "${fixture_files[@]}"; do
  if [[ ! -f "$SOURCE_FIXTURE_DIR/$fixture" ]]; then
    echo "Whippletree compatibility fixture is missing: $SOURCE_FIXTURE_DIR/$fixture" >&2
    exit 2
  fi
done

mkdir -p "$LOFT_REPO/$LOFT_FIXTURE_DIR"

for fixture in "$LOFT_FIXTURE_MANIFEST" "${fixture_files[@]}"; do
  source="$SOURCE_FIXTURE_DIR/$fixture"
  target="$LOFT_REPO/$LOFT_FIXTURE_DIR/$fixture"
  if [[ -f "$target" && "$OVERWRITE" != "1" ]] && ! cmp -s "$source" "$target"; then
    echo "target fixture differs; set WHIPPLETREE_OVERWRITE_LOFT_FIXTURES=1 to replace: $target" >&2
    exit 2
  fi
  install -m 0644 "$source" "$target"
done

echo "Staged Loft fixtures in $LOFT_REPO/$LOFT_FIXTURE_DIR"
echo
echo "Review and commit these files in the Loft repo:"
git -C "$LOFT_REPO" status --short -- "$SPEC_PATH" "$LOFT_FIXTURE_DIR"
echo
echo "After committing Loft spec and fixtures, run:"
echo "  scripts/add-loft-submodule.sh $LOFT_REPO vendor/loft"
