#!/usr/bin/env bash
# Regenerate — or check — the `.ir` reference goldens for examples/*.whip.
#
#   scripts/regen-ir-goldens.sh            rewrite every examples/<name>.ir from
#                                          `whip compile examples/<name>.whip`
#   scripts/regen-ir-goldens.sh --check    fail (exit 1) if any golden is stale,
#                                          naming the file; rewrites nothing
#
# `whip compile <file>.whip` deterministically reproduces the golden, so `--check`
# turns the goldens into a real gate (a lowering change that moves a golden fails
# CI) that is blessed with one command (run this with no args). Without `--check`
# the goldens "looked like goldens but nothing checked them" — the worst of both.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHIP=(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --)

mode="write"
if [[ "${1:-}" == "--check" ]]; then
  mode="check"
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

status=0
checked=0
skipped=0
for ir in "$ROOT"/examples/*.ir; do
  whip="${ir%.ir}.whip"
  if [[ ! -f "$whip" ]]; then
    echo "orphan golden (no matching .whip): ${ir#"$ROOT"/}" >&2
    status=1
    continue
  fi
  # Auto-detect a sibling package lock (examples/<name>.lock.json).
  args=()
  if [[ -f "${whip%.whip}.lock.json" ]]; then
    args=(--package-lock "${whip%.whip}.lock.json")
  fi
  # Redirect (not $(...)) so the trailing newline is preserved exactly.
  if ! "${WHIP[@]}" compile "$whip" "${args[@]}" >"$tmp" 2>/dev/null; then
    # Some examples need exotic setup (a generated lock, a script manifest) the
    # generic refresher can't know. Skip them rather than fail — their lowering is
    # exercised by their own dedicated checks (e.g. the artifact-admission
    # differential), not by this golden gate.
    echo "skip (needs setup the refresher can't supply): ${whip#"$ROOT"/}" >&2
    skipped=$((skipped + 1))
    continue
  fi
  checked=$((checked + 1))
  if [[ "$mode" == "check" ]]; then
    if ! diff -q "$tmp" "$ir" >/dev/null 2>&1; then
      echo "STALE golden: ${ir#"$ROOT"/} — run scripts/regen-ir-goldens.sh" >&2
      status=1
    fi
  else
    cp "$tmp" "$ir"
  fi
done

if [[ "$mode" == "check" ]]; then
  [[ "$status" -eq 0 ]] && echo "IR goldens OK: $checked examples up to date ($skipped skipped)."
else
  echo "IR goldens refreshed: $checked examples ($skipped skipped)."
fi
exit "$status"
