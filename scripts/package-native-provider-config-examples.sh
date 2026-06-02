#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/target/distrib/native-provider-config-examples.tar.gz}"
SOURCE_DIR="$ROOT/examples/provider-configs/native"

if [[ ! -f "$SOURCE_DIR/native.example.json" ]]; then
  echo "missing native provider config example: $SOURCE_DIR/native.example.json" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"
tar -czf "$OUT" -C "$ROOT" examples/provider-configs/native
echo "Wrote native provider config examples: $OUT"
