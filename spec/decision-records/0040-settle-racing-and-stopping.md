# DR-0040 — `whip settle`: racing + stopping (the system chooses N, v1)

Status: accepted + built (2026-07-14; the "settle" step of the
experimentation build spine — research note §4.3/§11, the next spine step
recorded in DR-0038). Witness model:
`models/maude/settle-stopping.maude` (the sound certifier certifies
exactly at the evidence-threshold crossing and returns an honest
`undetermined` on exhaustion below it; the naive run-until-N certifier is
the modeled hazard — a lucky early pass mints a certificate no evidence
supports). Tests: engine unit tests (`settle_walk_*` mirror the model's
bite and coverage searches), e2e
(`settle_races_pinned_scenarios_and_stops_at_the_crossing`,
`settle_exhausts_to_an_honest_undetermined`,
`settle_refuses_a_gauge_without_a_bar`). Cross-refs: DR-0037 (gauges and
bars — the decision being settled), DR-0038 (the paired regeneration
driver this races).

## Problem

`suppose` answers one what-if; `improve` verdicts candidates. Neither
answers the operator's third question — *are we sure enough?* — without
the operator inventing a sample size. The research note's contract:
name the decision and the system stops itself — at the gauge's bar, or
with an honest `undetermined` when the evidence is exhausted, **never at
an operator-chosen N** (run-until-N is the ritual the whole design
defines itself against).

## Decision

### 1. The decision is the gauge's bar

`whip settle <gauge>` requires the gauge to declare an `expect` bar; a
gauge without one has no decision shape and is refused with the reason
(declare the bar, don't invent one). Builtin resource gauges carry no
bars, so they are refused naturally.

### 2. The sound certifier (the model, executably)

One evidence walk per settle: a strong observation (the regenerated
reading passes the bar) raises the evidence level by one; a contrary one
lowers it, floored at zero; the decision closes exactly when the level
crosses the threshold K (default 3, `--threshold` to override). The
crossing is the log-domain abstraction of an e-process crossing 1/alpha:
by Ville's inequality the first crossing is valid at ANY stopping time,
which is why crossing-once suffices and no exhaustion is needed.
Exhaustion below K is an honest `undetermined`, never a certificate —
and the two outcomes are structurally exclusive (the model's `open` cell
is consumed by exactly one verdict rule).

### 3. Racing = the pinned pool, round-robin, until dry

The evidence stream is regenerations of the non-retired pinned scenarios
(prefix replay for mark pins, input replay otherwise — DR-0038's driver,
unchanged). Rounds are round-robin over the pool; **exhaustion is the
system's call, not an N**: a full pass that fails to raise the all-time
evidence high-water mark cannot be expected to add net evidence (with a
deterministic provider it provably cannot), so settle stops with
`undetermined` and says what to do about it (pin more scenarios, or
revisit the bar). A pass with no informative readings (judge unscored
everywhere) stops the same way with its own reason.

### 4. `--certify` mints the crossing as a certificate

Same walk, same threshold: `--certify` records the crossing observation's
evidence row with a `certificate` tag and reports a content-derived
certificate id (`ct-<hash8>` over gauge, program hash, N, level). Without
`--certify` the verdict is reported as `bar-cleared` with no durable
certificate. Uninformative observations don't move the walk but are
reported in the trail with their skip reason.

### 5. Every settle observation is evidence

Each regeneration lands in the ledger (`regen`, tagged `settle`) whether
or not it moved the walk — settling is measurement, and measurement is
never discarded.

## Honest v1 gaps

- No belief-update estimator: the walk reports level/threshold, not
  `P(better)=93%` — the estimator is the evidence-machinery growth step
  recorded since DR-0038.
- No identification-first pass and no EVSI allocation (`est. to settle`,
  instrumentation-edit suggestions): v1 races the pinned pool uniformly;
  the §12 identification layer is the recorded upgrade.
- `--compare <dim>` (rankability, "no total order") and `--anchor` are
  not built; settle v1 answers bar-shaped decisions only.
- `--spend-cap` is deliberately absent: regeneration cost is unpriced in
  v1 (the improve-loop residual), and accepting a cap that cannot bind
  would be dishonest surface. *(Landed 2026-07-14 once priced spend
  landed: the cap binds on the priced regeneration cost (`std.spend`'s
  observable) and stops the race as an honest `undetermined` with reason
  `spend-cap-reached`; unpriced usage still cannot bind it.)*
- The standing-contradiction reopener (evidence flags that respectfully
  reopen an overruled call) lands with the `evidence` verb surface, not
  here.
