#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v maude >/dev/null 2>&1; then
  echo "maude not found; skipping Maude checks" >&2
  exit 1
fi

declare -A EXPECTED_NO_SOLUTION=(
  ["coerce-branches.maude"]=1
  ["loft-claim-turn.maude"]=2
  ["effect-dependencies.maude"]=2
  ["policy-capacity-retry.maude"]=2
  ["ralph-loop.maude"]=1
)

declare -A EXPECTED_SOLUTION=(
  ["coerce-branches.maude"]=3
  ["loft-claim-turn.maude"]=2
  ["effect-dependencies.maude"]=4
  ["policy-capacity-retry.maude"]=4
  ["ralph-loop.maude"]=2
)

for test_file in "$ROOT"/models/maude/tests/*.maude; do
  echo "== maude ${test_file#"$ROOT"/}"
  output="$(maude "$test_file")"
  printf '%s\n' "$output"

  test_name="$(basename "$test_file")"
  actual_no_solution="$(grep -c 'No solution\.' <<<"$output" || true)"
  actual_solution="$(grep -c '^Solution 1' <<<"$output" || true)"

  if [[ "${EXPECTED_NO_SOLUTION[$test_name]:-}" != "$actual_no_solution" ]]; then
    echo "unexpected No solution count for $test_name: got $actual_no_solution, expected ${EXPECTED_NO_SOLUTION[$test_name]:-unset}" >&2
    exit 1
  fi

  if [[ "${EXPECTED_SOLUTION[$test_name]:-}" != "$actual_solution" ]]; then
    echo "unexpected Solution 1 count for $test_name: got $actual_solution, expected ${EXPECTED_SOLUTION[$test_name]:-unset}" >&2
    exit 1
  fi
done
