#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run_tlc() {
  tlc -deadlock \
    -config models/statechart-workflows/SpecImplementation.cfg \
    models/statechart-workflows/SpecImplementation.tla
}

run_maude() {
  maude models/statechart-workflows/SpecImplementation.maude
}

run_generated_checks() {
  cargo run -q -p whippletree-cli -- check \
    examples/workflows/spec-implementation.whip \
    --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
    --policy examples/policies/spec-implementation.enterprise-policy.json \
    --target tla \
    --json >/dev/null
  cargo run -q -p whippletree-cli -- check \
    examples/workflows/spec-implementation.whip \
    --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
    --policy examples/policies/spec-implementation.enterprise-policy.json \
    --target maude \
    --json >/dev/null
  cargo run -q -p whippletree-cli -- prove \
    examples/workflows/spec-implementation.whip \
    --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
    --policy examples/policies/spec-implementation.enterprise-policy.json \
    --json >/dev/null
}

if command -v tlc >/dev/null 2>&1 && command -v maude >/dev/null 2>&1; then
  run_tlc
  run_maude
  run_generated_checks
elif command -v nix >/dev/null 2>&1; then
  nix --extra-experimental-features 'nix-command flakes' develop -c bash -c '
    tlc -deadlock \
      -config models/statechart-workflows/SpecImplementation.cfg \
      models/statechart-workflows/SpecImplementation.tla
    maude models/statechart-workflows/SpecImplementation.maude
    cargo run -q -p whippletree-cli -- check \
      examples/workflows/spec-implementation.whip \
      --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
      --policy examples/policies/spec-implementation.enterprise-policy.json \
      --target tla \
      --json >/dev/null
    cargo run -q -p whippletree-cli -- check \
      examples/workflows/spec-implementation.whip \
      --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
      --policy examples/policies/spec-implementation.enterprise-policy.json \
      --target maude \
      --json >/dev/null
    cargo run -q -p whippletree-cli -- prove \
      examples/workflows/spec-implementation.whip \
      --adapter-manifest examples/adapters/spec-implementation.fake-adapter.json \
      --policy examples/policies/spec-implementation.enterprise-policy.json \
      --json >/dev/null
  '
else
  echo "error: tlc and maude are not available, and nix is unavailable" >&2
  exit 127
fi
