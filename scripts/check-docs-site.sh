#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if python3 -m mkdocs --version >/dev/null 2>&1; then
  python3 -m mkdocs build --strict --config-file "$ROOT/mkdocs.yml"
  exit 0
fi

VENV="$ROOT/target/docs-mkdocs-venv"
if [[ ! -x "$VENV/bin/mkdocs" ]]; then
  if python3 -m venv "$VENV" >/dev/null 2>&1; then
    "$VENV/bin/python" -m pip install -r "$ROOT/docs/requirements.txt"
  else
    TARGET="$ROOT/target/docs-python"
    if [[ ! -d "$TARGET/mkdocs" ]]; then
      python3 -m pip install --target "$TARGET" -r "$ROOT/docs/requirements.txt"
    fi
    PYTHONPATH="$TARGET${PYTHONPATH:+:$PYTHONPATH}" \
      python3 -m mkdocs build --strict --config-file "$ROOT/mkdocs.yml"
    exit 0
  fi
fi

"$VENV/bin/mkdocs" build --strict --config-file "$ROOT/mkdocs.yml"
