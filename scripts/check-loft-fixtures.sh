#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/loft-fixtures-lib.sh"

SUBMODULE_FIXTURE_DIR="$ROOT/vendor/loft/$LOFT_FIXTURE_DIR"
COMPAT_FIXTURE_DIR="$ROOT/examples/loft-fixtures/v0.1"

if [[ -n "${WHIPPLETREE_LOFT_FIXTURE_DIR:-}" ]]; then
  FIXTURE_DIR="$WHIPPLETREE_LOFT_FIXTURE_DIR"
elif [[ -d "$SUBMODULE_FIXTURE_DIR" ]]; then
  FIXTURE_DIR="$SUBMODULE_FIXTURE_DIR"
elif [[ "${WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES:-}" == "1" ]]; then
  FIXTURE_DIR="$SUBMODULE_FIXTURE_DIR"
elif [[ -d "$COMPAT_FIXTURE_DIR" ]]; then
  FIXTURE_DIR="$COMPAT_FIXTURE_DIR"
else
  FIXTURE_DIR="$SUBMODULE_FIXTURE_DIR"
fi

if [[ ! -d "$FIXTURE_DIR" ]]; then
  if [[ "${WHIPPLETREE_REQUIRE_LOFT_FIXTURES:-}" == "1" || "${WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES:-}" == "1" ]]; then
    echo "missing required Loft fixture directory: $FIXTURE_DIR" >&2
    exit 2
  fi

  echo "Skipping Loft fixture conformance checks."
  echo "Set WHIPPLETREE_LOFT_FIXTURE_DIR, add vendor/loft/fixtures/whippletree/v0.1,"
  echo "or keep examples/loft-fixtures/v0.1 available."
  exit 0
fi

FIXTURE_DIR="$(cd "$FIXTURE_DIR" && pwd)"

if [[ "${WHIPPLETREE_REQUIRE_LOFT_SUBMODULE_FIXTURES:-}" == "1" && "$FIXTURE_DIR" != "$SUBMODULE_FIXTURE_DIR" ]]; then
  echo "Loft submodule fixtures are required; refusing fallback fixture source: $FIXTURE_DIR" >&2
  exit 2
fi

mapfile -t fixture_files < <(loft_fixture_files "$FIXTURE_DIR")

if [[ "${#fixture_files[@]}" -eq 0 ]]; then
  echo "Loft fixture manifest has no fixtures: $(loft_manifest_path "$FIXTURE_DIR")" >&2
  exit 2
fi

missing=0
for file in "$LOFT_FIXTURE_MANIFEST" "${fixture_files[@]}"; do
  if [[ ! -f "$FIXTURE_DIR/$file" ]]; then
    echo "missing Loft fixture: $FIXTURE_DIR/$file" >&2
    missing=1
  fi
done

if [[ "$missing" -ne 0 ]]; then
  exit 2
fi

WHIPPLETREE_LOFT_FIXTURE_DIR="$FIXTURE_DIR" \
  cargo test --quiet --manifest-path "$ROOT/Cargo.toml" -p whippletree-kernel \
    loft_submodule_fixture_shapes_are_compatible -- --nocapture
