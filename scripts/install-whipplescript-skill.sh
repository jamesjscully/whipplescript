#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/skills/whipplescript-author/SKILL.md"
TARGET_DIR="${WHIPPLESCRIPT_SKILL_DIR:-$HOME/.codex/skills/whipplescript-author}"

if [[ ! -f "$SOURCE" ]]; then
  echo "missing source skill: $SOURCE" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"
cp "$SOURCE" "$TARGET_DIR/SKILL.md"

echo "Installed whipplescript-author skill to $TARGET_DIR/SKILL.md"
