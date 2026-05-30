#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/skills/whipplescript-author/SKILL.md"
DIST_DIR="${WHIPPLESCRIPT_SKILL_DIST_DIR:-$ROOT/dist}"
PACKAGE="$DIST_DIR/whipplescript-author-skill.tar.gz"
CHECKSUM="$PACKAGE.sha256"

if [[ ! -f "$SOURCE" ]]; then
  echo "missing source skill: $SOURCE" >&2
  exit 1
fi

mkdir -p "$DIST_DIR"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

mkdir -p "$tmpdir/whipplescript-author"
cp "$SOURCE" "$tmpdir/whipplescript-author/SKILL.md"

tar \
  --sort=name \
  --mtime='UTC 2026-01-01' \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -czf "$PACKAGE" \
  -C "$tmpdir" \
  whipplescript-author

sha256sum "$PACKAGE" > "$CHECKSUM"
echo "Packaged whipplescript-author skill:"
echo "  $PACKAGE"
echo "  $CHECKSUM"
