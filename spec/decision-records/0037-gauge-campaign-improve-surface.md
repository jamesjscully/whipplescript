# DR-0037 — The gauge/campaign surface and the `whip improve` loop (v1)

Status: accepted + built (2026-07-11; formalizes the settled ground of
`spec/improve-design-note.md` §12 and
`spec/experimentation-subsystem-research-note.md` §4/§17, per the build
items in `spec/experimentation-improve-tracker.md`). Witness models:
`models/maude/improve-acceptance.maude` (never surface a dominated
candidate) and `models/maude/improve-holdout.maude` (seal-blindness,
wear-out, progressive rigor). Tests: parser
(`gauge_and_campaign_declarations_parse_and_lower` and siblings), store
(`whipplescript-store/src/improve.rs` unit tests), engine
(`crates/whipplescript-cli/src/improve.rs` unit tests), end-to-end
(`crates/whipplescript-cli/tests/improve_loop.rs`). Cross-refs: the two
design notes above (the design SSOTs — this DR records only what v1 builds
and the deliberate deviations), DR-0032 (typed failures — the exec judge
rides the det-validation pattern), versioned-workspace research note
(containment posture, certified-merge rebase upgrade path).

## Problem

The improve/evals half of the v0.4 banner existed as settled-in-principle
design notes with no ADR, no language surface, and no runtime. The four
Jack-held design questions (campaign scope, proposer leakage tiers, the
local utility model, the cross-workspace evidence door) are open — but the
settled ground is buildable without them if v1 stays conservative on each.

## Decision

### 1. Language surface: `gauge` and `campaign` as hand-parsed core decls

Both are core-grammar exceptions (like `agent`/`signal`/`coerce`), NOT
declaration-family constructs: `judge via` is a four-way tagged union,
bars are comparisons, `ascend` takes comma lists — all outside the
two-shape meta-grammar (DR-0011), deliberately.

```whip
gauge extract_quality on summarize.extract {
  judge via coerce DueDateJudge      # or: prompt "…" | exec "…" | labels "…"
  expect P(due_date_correct) at least 0.9
}

gauge fulfillment_cost {
  judge via exec "./cost_model.py"
  inputs extract_quality, std.spend  # derived gauge: judge gets the score vector
}

campaign release_tuning {
  ascend    extract_quality, reply_quality
  reach     std.latency at most 800ms
  guard     tone within 2 percent
  sacrifice verbosity
}
```

**Deviation from the design-note sketches, recorded:** the declaration
surface uses the word forms **`at least` / `at most`** (not `>=`/`<=`) and
**`within 2 percent`** (not `2%`). The declaration tokenizer deliberately
steps over comparison operators (expressions re-parse from raw source
slices) and has no `%` token; the shipped precedent is presence conditions
using `is` instead of `==`. A user who writes `>=` gets a targeted
diagnostic ("write `at least`"). The **CLI** keeps real operators
(`whip improve extract_quality>=0.9`) — args never pass through the
declaration tokenizer. The research note lists `expect` forms as an open
grammar detail; this closes it for v1.

Bars are chance-shaped (`P(<field>)` — the judge's boolean output field)
or stat-shaped (`mean`, `p10`, `p90`, … over the score distribution).
Thresholds keep exact source text end-to-end (AST/IR are `Eq`; consumers
parse). Validation: unique names, `judge via coerce` must resolve, derived
gauges must judge via exec and may not input themselves, campaign refs
must resolve to declared gauges or builtins, a campaign's partition is
disjoint, and a campaign must name something to improve. Neither
declaration is allowed in pattern bodies (objective intent is top-level).
`std.spend` / `std.latency` / `std.tokens` are built-in, namespaced,
present without declaration, and default to relative indifference bands.

### 2. Storage: a sibling improve store, append-only

`whipplescript-store/src/improve.rs`, own SQLite file
(`.whipplescript/improve.sqlite`, env `WHIPPLESCRIPT_IMPROVE_STORE`) —
the coordination-store pattern: run stores are disposable, the evidence
asset is durable. Tables: `evidence_rows` (append-only observations:
gauge, score, passed, execution_mode live|regen, scorer, scenario,
campaign/candidate refs, tags), `scenarios` (pinned runs; v1 identity =
the run's frozen input), `scenario_wear` (CUMULATIVE promotion-gate
exposure across campaigns — why it lives outside any campaign record),
`campaign_events` (the append-only campaign record, folded on read —
program archaeology's data). No DO twin in v1: improve is a native CLI
surface; the DO parity sweep picks this up when improve moves on-edge.

### 3. Verbs

- `whip pin <instance> --as <name>` — pin the scenario corpus.
- `whip improve [targets…] [then …] [--sacrifice g] [--within g=2%]
  [--spend-cap $n] [--proposer fixture|native] [--program p.whip]` —
  naming-is-the-partition; a single positional naming a declared
  `campaign` adopts its spec; bare = repair mode. Runs the campaign:
  seal → baseline regen → propose → static gate → evaluate → dominance
  verdict → sealed promotion gate → evidence card. Propose-don't-apply.
- `whip campaigns` / `whip campaign <id>` — the folded record + cards.
- `whip adopt <campaign>:<candidate> [--program p]` — reserved for
  candidates the campaign actually PROPOSED (a refused or tradeoff
  candidate has no adoption side door around the acceptance model), and
  writes the candidate source only if the program still matches the
  campaign's baseline hash; otherwise refuses honestly (certified-merge
  rebase is the upgrade).
- `whip gauges [<gauge>]` — the evidence view with the live/regen N
  decomposition. **Naming residual:** the research note's verb is
  `whip evidence <gauge>`, but `whip evidence` is already the runtime
  instance-evidence command; unification is a rename pass, recorded in
  the tracker.
- Ambient scoring: `whip dev` scores free deterministic judges (exec +
  builtins) after every settled run — measurement is ambient, no verb.

### 4. The loop's invariants (modeled first)

Acceptance = the Maude-modeled dominance rule: at least one ascend gauge
better, no ascend gauge worse, every guarded gauge within band (fail
closed: a guarded gauge that becomes unmeasurable on the candidate
refuses), declared bars hard regardless of partition, sacrifice releases
the guard only. Focus-up + guard-broken = a genuine tradeoff: recorded
and surfaced. *(Amended 2026-07-13, after the utility-model design pass
settled:)* a surfaced tradeoff is answerable via `whip answer` — the
answer is a precedent, and future tradeoffs auto-resolve by monotone
precedent dominance (default-on, locality-bounded, citing the precedent,
revocable; `models/maude/improve-precedent.maude`); everything without an
applicable dominant precedent still asks. Holdout: 20%/floor-2 sealed per campaign,
deterministic per campaign id (rotation across campaigns), engaged only at
≥ 4 scenarios — below that the campaign runs tagged `unheld-out`; the
proposer's reflection is holdout-blind (sealed aggregates only — tested);
each promotion gate bumps cumulative wear, k=3 retires. Bands: quality =
1.96·pooled-SE noise floor (min 0.02); resource gauges = 5% relative;
`--within`/`guard` override in percent of baseline.

### 5. Evaluation and containment

Scenario regeneration = re-run the workflow on the frozen input in a
DISPOSABLE temp store, with every workspace-scoped side store
(coordination leases/counters/ledgers, backlog items, harness content)
redirected into the eval scratch for the process — a counterfactual run's
writes land nowhere near the workspace stores (branch-grade containment
and egress-door diversion arrive with the versioned-workspace per-door
policy — the dependency the improve note records). Both arms run the same
scenarios (matched on input). Candidates compile through the full static
gate (under the campaign's `--root`) before any sample is spent; failures
die recorded. Judges see the run's COMPLETE fact record (consumed facts
included), receive a versioned input contract
(`whipplescript.judge_input.v0`), and run under a bounded wait so a wedged
judge can never hang a dev loop or campaign. Fixture-provider evaluations
are tagged `fixture-provider` on every evidence row and card — canned
outputs never pass silently for model behavior.

### 6. The proposer

`--proposer native` = one structured native-coerce turn (schema
`{rationale, source}`) fed program source + campaign spec + open-scenario
evidence + prior refusals; `--proposer fixture` = deterministic env-fed
candidates for tests/dev. Leakage posture (tiers settled 2026-07-11,
improve note §7): propose-don't-apply + the human adoption door as the
baseline; every candidate checked for verbatim scenario-payload fragments
newly present in its source (`leakage-overlap` card tag — flag, never
block); **campaign-attached stratified reflection** via the `proposer
redacted` declaration clause or `--redacted-view` (tighten-only), under
which the proposer sees aggregates alone — modeled as the
`redactedReflect` no-read invariant in `improve-holdout.maude` and tagged
`proposer:redacted-view` on the campaign's evidence. Reader-set-delta
tier keying engages when the evidence-plane IFC build lands.

## Honest v1 gaps (recorded, tagged at runtime where visible)

- `coerce` judges declared-but-unscoreable (parameter binding needs
  program context); `prompt` judges need a configured provider and are
  campaign-time only; both surface as `judge unscored — <reason>`.
  *(Coerce judges landed 2026-07-14, design pass with Jack: EXPLICIT
  argument binding — `judge via coerce Assess(input.ticket.title,
  facts.Assessment.priority)`, positional against the coerce's
  parameters, arity- and path-checked at compile time so a drifted
  signature is a check error, never a silently rebound judge; the single
  reserved `record` passes the whole judge-input record to a
  one-parameter coerce; scoring renders the SAME prompt/schema the
  runtime's `build_coerce_call_parts` would and reads the verdict off
  the coerce's own output. Bare `judge via coerce X` still parses and
  stays honestly unscoreable. Same day, the `evidence` naming residual
  settled as a subcommand split: bare `whip evidence [<gauge>]` is the
  gauge evidence view, `whip evidence instance <id>` the runtime
  evidence chain.)*
- `std.spend` has no priced observable yet (absent, never fabricated);
  the spend cap is enforced per proposal round against recorded cost and
  parks the campaign when reached, but token-only usage records cost 0
  (the `campaign.spend` events carry the tokens), so the cap cannot yet
  bind on unpriced usage. Provider price tables are the follow-on.
  *(Landed 2026-07-14, design pass with Jack: config-only price tables in
  the provider config's `prices` block — USD/Mtok per (provider, model),
  input/output separate, no shipped defaults, maintained example config;
  record-time pricing on spend events with an honest `priced: false` for
  unmatched usage; `std.spend` scores the priced run total in USD and
  skips with the reason when any usage-bearing run is unpriceable.)*
- `mark` (prefix cuts) and `suppose`/`settle` are not built — scenarios
  are whole-run input replays until the checkpoint-substrate integration;
  `then` stages beyond the first are recorded in the campaign record and
  run in later invocations (ratchet execution follow-on). *(All landed:
  marks/suppose 2026-07-13 (DR-0038), settle 2026-07-14 (DR-0040), and
  ratchet execution 2026-07-14 — a stage whose reach targets the baseline
  meets advances at invocation time, its achieved levels held as hard
  guard floors, recorded as `stage.advanced` events.)*
- No slice-hash transfer: evidence keys per program hash (whole-program
  fail-closed — per-version evidence islands, trivially sound per the
  research note §7).
- The v1 loop runs to its internal stopping rule (or the spend cap)
  within one invocation; resuming a parked campaign across invocations is
  a follow-on (`campaign.resumed` exists in the record vocabulary).
  *(Landed 2026-07-14 with priced spend, which made parked campaigns
  reachable: a park now ends the invocation on the `campaign.parked`
  event (no `campaign.closed` after it, so the record folds to `parked`);
  `whip improve --resume <id>` continues under a fresh per-invocation
  allowance — Jack's call — with the spec, program, proposer, and
  candidate numbering from the record, guarded by the baseline hash.)*
- Evaluation recompiles the program per scenario (correct, wasteful);
  hoisting a precompiled-IR path through `start_workflow_instance` /
  `run_worker_once` and parallelizing the per-scenario evaluations over
  the bounded thread pool are recorded efficiency follow-ons. *(Both
  landed 2026-07-14: the recompile hoist as a (root, source-hash)-keyed
  process-lifetime compile cache, and parallel evaluation once the
  env-var side-store containment was replaced by explicit
  `SideStorePaths` threaded through the drive — evaluations pool on a
  bounded `thread::scope`, results keep input order (the pairing), and
  child workflow drives inherit the parent's overrides. Judge turns also
  became recorded `campaign.spend` like proposer turns the same day.)*
