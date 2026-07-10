#!/usr/bin/env bash
#
# Trace <-> model conformance bridge.
#
# Ties three artifacts to one transition relation so none can drift alone:
#   1. models/effect-lifecycle-transitions.tsv        (the single source of truth)
#   2. crates/whipplescript-kernel/src/trace.rs       (the Rust checker)
#   3. models/maude/tests/trace-lifecycle-conformance.maude (the executable model)
#
# This script enforces:
#   (a) the `model_pinned=yes` rows of the corpus == the `CORPUS:` tags in the Maude
#       file  (so the model proves exactly the pinned transitions),
#   (b) the Rust exhaustive-conformance test passes (check_trace accepts exactly the
#       corpus's legal transitions and rejects every other cell), and
#   (c) each RICHER check_trace invariant (dependency ordering, run/lease liveness,
#       terminal identity, pause/cancel gating, revision epochs) named in
#       models/trace-invariant-correspondence.tsv maps to a ControlPlaneLifecycle.tla
#       invariant that is present AND conjoined into SafetyInvariants (Apalache-checked),
#       and has a Rust bite scenario (trace.rs `richer_invariants_have_bite`).
# The Maude searches (that kernel.maude realizes each pinned transition and forbids
# each recovery guard) are executed by scripts/check-formal-models.sh; the TLA
# invariants themselves are checked by scripts/check-tla-models.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CORPUS="models/effect-lifecycle-transitions.tsv"
MAUDE="models/maude/tests/trace-lifecycle-conformance.maude"
CORRESPONDENCE="models/trace-invariant-correspondence.tsv"
TLA_SPEC="models/tla/ControlPlaneLifecycle.tla"

echo "== trace<->model corpus drift check"
python3 - "$CORPUS" "$MAUDE" <<'PY'
import sys

corpus_path, maude_path = sys.argv[1], sys.argv[2]

# Pinned transitions from the corpus (single source of truth).
pinned = set()
for line in open(corpus_path):
    line = line.rstrip("\n")
    if not line or line.startswith("#") or line.startswith("from_status"):
        continue
    cols = line.split("\t")
    if len(cols) < 4:
        raise SystemExit(f"malformed corpus row: {line!r}")
    frm, event, to, model_pinned = cols[0], cols[1], cols[2], cols[3]
    if model_pinned.strip() == "yes":
        pinned.add((frm, event, to))

# CORPUS: tags claimed by the Maude conformance test (ignore CORPUS-NEG guards).
tagged = set()
for line in open(maude_path):
    s = line.strip()
    marker = "--- CORPUS:"
    if s.startswith(marker):
        parts = s[len(marker):].split()
        if len(parts) != 3:
            raise SystemExit(f"malformed CORPUS tag: {s!r}")
        tagged.add((parts[0], parts[1], parts[2]))

missing_in_maude = pinned - tagged
extra_in_maude = tagged - pinned
ok = True
if missing_in_maude:
    ok = False
    print("pinned corpus rows with no CORPUS: tag in the Maude test:", file=sys.stderr)
    for t in sorted(missing_in_maude):
        print(f"  {t[0]} --{t[1]}--> {t[2]}", file=sys.stderr)
if extra_in_maude:
    ok = False
    print("CORPUS: tags with no matching model_pinned=yes corpus row:", file=sys.stderr)
    for t in sorted(extra_in_maude):
        print(f"  {t[0]} --{t[1]}--> {t[2]}", file=sys.stderr)
if not ok:
    raise SystemExit(1)
print(f"corpus <-> model tags agree ({len(pinned)} pinned transitions)")
PY

echo "== richer check_trace invariants <-> ControlPlaneLifecycle.tla"
python3 - "$CORRESPONDENCE" "$TLA_SPEC" <<'PY'
import re, sys

corr_path, tla_path = sys.argv[1], sys.argv[2]
tla = open(tla_path).read()

# The SafetyInvariants conjunction is what Apalache actually checks (--inv).
m = re.search(r"^SafetyInvariants ==\n(.*?)(?=\n[A-Za-z=]|\Z)", tla, re.S | re.M)
if not m:
    raise SystemExit("could not find SafetyInvariants in the TLA spec")
safety_block = m.group(1)

rows = 0
problems = []
for line in open(corr_path):
    line = line.rstrip("\n")
    if not line or line.startswith("#") or line.startswith("key\t"):
        continue
    cols = line.split("\t")
    if len(cols) < 3:
        raise SystemExit(f"malformed correspondence row: {line!r}")
    key, _substring, inv = cols[0], cols[1], cols[2]
    rows += 1
    # The invariant must be DEFINED in the spec ...
    if not re.search(rf"^{re.escape(inv)} ==", tla, re.M):
        problems.append(f"{key}: TLA invariant `{inv}` is not defined in the spec")
    # ... and conjoined into SafetyInvariants so Apalache checks it.
    if not re.search(rf"/\\ *{re.escape(inv)}\b", safety_block):
        problems.append(f"{key}: TLA invariant `{inv}` is not conjoined into SafetyInvariants")

if problems:
    for p in problems:
        print("  " + p, file=sys.stderr)
    raise SystemExit(1)
print(f"richer invariants <-> TLA agree ({rows} invariants present and Apalache-checked)")
PY

echo "== rust checker bite tests (transition corpus + richer invariants)"
cargo test --quiet -p whipplescript-kernel --lib -- \
  trace::tests::checker_matches_transition_corpus \
  trace::tests::richer_invariants_have_bite

echo "trace<->model conformance OK"
echo "note: the Maude searches for this bridge run under scripts/check-formal-models.sh"
echo "      (trace-lifecycle-conformance.maude: 10 Solution, 3 No solution); the TLA"
echo "      invariants run under scripts/check-tla-models.sh."
