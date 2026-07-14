# DR-0041 — The belief-update estimator and the standing-contradiction reopener

Status: accepted + built (2026-07-14; design pass with Jack the same day:
two model families; Jeffreys cold priors; reopener trigger = posterior ≥
0.8 AND non-decreasing over the last three informative observations,
advisory-only; surfaces = `suppose`, `settle`, `gauges`). Witness model:
`models/maude/contradiction-reopener.maude` (the sustained trigger never
flags a spiking-but-receding posterior; the naive spike trigger is the
modeled hazard; and the module deliberately contains NO rule producing
`revoked` — the reopener carries no revocation authority). Tests:
numerics unit tests against independently computed references
(`incomplete_beta_and_t_cdf_match_reference_values` + siblings), e2e
(`suppose_reads_out_p_better_from_the_paired_sign_test`,
`settle_reads_out_p_bar_met_alongside_the_walk`,
`sustained_live_contradiction_reopens_an_answered_call`). Cross-refs:
DR-0040 (the certification walk this reads out alongside), DR-0037
(cards and precedents), research note §4.3/§18.1.

## Problem

`suppose` printed raw paired readings, `settle` counted an evidence
level, and an overruled call stayed overruled forever — the vignettes'
`P(better)=71%`, `P(bar met)=93%`, and "⚠ contradicts the 07-03 call:
P(worse)=81% and tightening" had no estimator behind them. §18.1's
recorded floor is Beta-Bernoulli-per-partition; the design pass extended
it with a second family for continuous evidence.

## Decision

### 1. Two families, Jeffreys priors, pure functions

**Family A (pass/fail evidence)** — the Bayesian sign test over paired
bar verdicts: wins = discordant pairs favoring the treatment, losses =
the reverse, concordant pairs uninformative (exactly what pairing wants);
`P(better) = P(θ > ½)` for `θ ~ Beta(½+wins, ½+losses)`. Defined from a
single pair (`N=1 → ≈ 0.82` for one win), which is what `suppose` needs.

**Family B (continuous evidence)** — the Student-t posterior on paired
deltas under the Jeffreys prior on (μ, σ²): `P(mean delta favors the
gauge's better direction)`. Needs ≥ 2 deltas (one delta has no scale —
the honest refusal); zero variance is the deterministic case and reads
as certainty.

Numerics are self-contained (`ln_gamma`/`betainc`/`student_t_cdf`, the
standard continued fraction) — no dependency added; unit-tested against
independently computed reference values.

### 2. Where the readouts surface

- **`suppose`**: `p_better` per gauge line — family A over the one
  (recorded, regenerated) pair when both carry verdicts; family B stays
  silent at N=1 by design.
- **`settle`**: `p_bar_met` alongside the walk — θ = per-regeneration
  bar-pass probability, referenced at the chance bar's own rate (or
  majority for stat-shaped bars). The CERTIFICATION rule remains the
  anytime-valid walk (DR-0040); the posterior is the readout, never the
  stopping rule.
- **Evidence cards** (`improve`): `p_better` per gauge line over the
  comparable pairs — family A when every pair carries verdicts, family B
  on paired score deltas otherwise. This is where the second family
  earns its keep (resource gauges across ≥ 2 scenarios).

### 3. The standing-contradiction reopener

For every ACCEPTED precedent, the reopener folds the **ambient stream
only** — live rows recorded under the accepted candidate's own program
hash, after the answer (the campaign's regen rows were the evidence the
answer already weighed; they reopen nothing; and a program that has
since moved on quiesces the flag naturally, because its rows carry a
different hash). Per gauge, the contradiction posterior runs against the
answer-time operating point: family A (`P(pass rate below the accepted
point)`, reference clamped off degenerate endpoints) when rows carry
verdicts and the point is rate-shaped; family B (one-sample t on the
worse side) otherwise.

**The trigger is sustained** (`contradiction-reopener.maude`): flag only
when the posterior trajectory's last three informative points are
non-decreasing AND the final one is ≥ 0.8 — a single noisy day never
nags, a recession resets the streak, and a re-tightening streak still
flags. **The flag is advisory only**: it cites the precedent
(`campaign:candidate (accepted <when>)`); revocation stays
`whip answer --revoke`. Surfaced on `whip gauges` (JSON
`contradictions` array + a ⚠ line per flagged gauge).

Ambient dev rows now stamp their program hash (previously `None`), which
is what lets the reopener — and any future same-hash warm-start — fold
them.

## Honest v1 gaps

- The warm prior is structural, not parametric: posteriors accumulate
  within one program hash because rows are hash-keyed; carrying belief
  ACROSS hashes is the transfer layer (slice-hash carry), not a discount
  factor invented here.
- `suppose`'s vignette shows a warm-started `P(better)` at N=1; v1
  reports the pure paired posterior — folding prior same-hash pairs in
  is recorded for the transfer step.
- The reopener reads accepted precedents only: a REJECTED candidate was
  never adopted, so no ambient stream exists to vindicate it.
- Rate-shaped detection for family A uses the answer-time operating
  point's range; an exotic gauge scoring in [0,1] without being a rate
  would be read as one.
