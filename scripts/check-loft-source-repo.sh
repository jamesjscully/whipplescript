#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

REPO="${1:-}"
LABEL="${2:-Loft source repo}"
missing=0
fixture_files=()

if [[ -z "$REPO" ]]; then
  echo "usage: scripts/check-loft-source-repo.sh <local-loft-repo> [label]" >&2
  exit 2
fi

if [[ ! -d "$REPO/.git" ]]; then
  echo "$LABEL is not a local git repo: $REPO" >&2
  exit 2
fi

echo "$LABEL: $REPO"
git -C "$REPO" log -1 --oneline || true

SPEC_PATH="spec/loft-v0.1.md"
if ! git -C "$REPO" ls-files --error-unmatch "$SPEC_PATH" >/dev/null 2>&1; then
  echo "$LABEL is missing tracked spec/loft-v0.1.md" >&2
  missing=1
fi

manifest_path="$REPO/$LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST"
if [[ ! -f "$manifest_path" ]]; then
  echo "$LABEL is missing fixture manifest: $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  missing=1
elif ! git -C "$REPO" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >/dev/null 2>&1; then
  echo "$LABEL is missing tracked $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
  missing=1
else
  mapfile -t fixture_files < <(loft_fixture_files "$REPO/$LOFT_FIXTURE_DIR")
  if [[ "${#fixture_files[@]}" -eq 0 ]]; then
    echo "$LABEL fixture manifest has no fixtures: $LOFT_FIXTURE_DIR/$LOFT_FIXTURE_MANIFEST" >&2
    missing=1
  fi
fi

for fixture in "${fixture_files[@]}"; do
  if ! git -C "$REPO" ls-files --error-unmatch "$LOFT_FIXTURE_DIR/$fixture" >/dev/null 2>&1; then
    echo "$LABEL is missing tracked $LOFT_FIXTURE_DIR/$fixture" >&2
    missing=1
  fi
done

if [[ -n "$(git -C "$REPO" status --short)" ]]; then
  echo "$LABEL has uncommitted changes:" >&2
  git -C "$REPO" status --short >&2
  missing=1
fi

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi
