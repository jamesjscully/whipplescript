#!/usr/bin/env bash

# shellcheck disable=SC2034 # Sourced by Loft fixture scripts as a shared default.
LOFT_FIXTURE_DIR="fixtures/whipplescript/v0.1"
LOFT_FIXTURE_MANIFEST="manifest.json"

loft_manifest_path() {
  local fixture_dir="$1"
  printf '%s/%s\n' "$fixture_dir" "$LOFT_FIXTURE_MANIFEST"
}

loft_fixture_files() {
  local fixture_dir="$1"
  local manifest
  manifest="$(loft_manifest_path "$fixture_dir")"

  if [[ ! -f "$manifest" ]]; then
    echo "missing Loft fixture manifest: $manifest" >&2
    return 2
  fi

  if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required to read Loft fixture manifests" >&2
    return 2
  fi

  jq -er '.fixtures[] | .file' "$manifest"
}
