#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

SOURCE="${1:-}"
TARGET="${2:-vendor/loft}"

if [[ -z "$SOURCE" ]]; then
  echo "usage: scripts/add-loft-submodule.sh <loft-repo-url-or-path> [target]" >&2
  exit 2
fi

if [[ ! -d "$SOURCE/.git" && "$SOURCE" != http://* && "$SOURCE" != https://* && "$SOURCE" != git@* ]]; then
  echo "source is not a local git repo or supported git URL: $SOURCE" >&2
  exit 2
fi

if [[ -d "$SOURCE/.git" ]]; then
  "$ROOT/scripts/check-loft-source-repo.sh" "$SOURCE" "Loft submodule source repo"
fi

if [[ -e "$ROOT/$TARGET" ]]; then
  echo "target already exists: $TARGET" >&2
  exit 2
fi

git -C "$ROOT" submodule add "$SOURCE" "$TARGET"
git -C "$ROOT" submodule update --init --recursive "$TARGET"

if ! git -C "$ROOT/$TARGET" ls-files --error-unmatch spec/loft-v0.1.md >/dev/null 2>&1; then
  echo "added submodule does not contain tracked spec/loft-v0.1.md" >&2
  exit 1
fi

if ! git -C "$ROOT/$TARGET" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >/dev/null 2>&1; then
  echo "added submodule does not contain tracked $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  exit 1
fi

mapfile -t fixture_files < <(loft_fixture_files "$ROOT/$TARGET/$LOFT_FIXTURE_DIR")

if [[ "${#fixture_files[@]}" -eq 0 ]]; then
  echo "added submodule fixture manifest has no fixtures: $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  exit 1
fi

for fixture in "${fixture_files[@]}"; do
  if ! git -C "$ROOT/$TARGET" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$fixture" >/dev/null 2>&1; then
    echo "added submodule does not contain tracked $LOFT_FIXTURE_DIR/$fixture" >&2
    exit 1
  fi
done

echo "Added Loft submodule at $TARGET"
