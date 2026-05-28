#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/skills/whippletree-author/SKILL.md"
DIST_DIR="${WHIPPLETREE_SKILL_DIST_DIR:-$ROOT/dist}"
PACKAGE="$DIST_DIR/whippletree-author-skill.tar.gz"
CHECKSUM="$PACKAGE.sha256"

if [[ ! -f "$SOURCE" ]]; then
  echo "missing source skill: $SOURCE" >&2
  exit 1
fi

mkdir -p "$DIST_DIR"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

mkdir -p "$tmpdir/whippletree-author"
cp "$SOURCE" "$tmpdir/whippletree-author/SKILL.md"

tar \
  --sort=name \
  --mtime='UTC 2026-01-01' \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -czf "$PACKAGE" \
  -C "$tmpdir" \
  whippletree-author

sha256sum "$PACKAGE" > "$CHECKSUM"
echo "Packaged whippletree-author skill:"
echo "  $PACKAGE"
echo "  $CHECKSUM"
