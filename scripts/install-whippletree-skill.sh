#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/skills/whippletree-author/SKILL.md"
TARGET_DIR="${WHIPPLETREE_SKILL_DIR:-$HOME/.codex/skills/whippletree-author}"

if [[ ! -f "$SOURCE" ]]; then
  echo "missing source skill: $SOURCE" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"
cp "$SOURCE" "$TARGET_DIR/SKILL.md"

echo "Installed whippletree-author skill to $TARGET_DIR/SKILL.md"
