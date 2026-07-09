# Language ergonomics tracker (v2 surface)

Source: ergonomics/design evaluation of 2026-06-09 (follow-up to
[`review-change-plan.md`](../review-change-plan.md), all items of which shipped).

Philosophy anchor: WhippleScript is a simple scripting language for AI to
orchestrate AI. Safe to run by default; explicit, gated escape hatches for
external scripts and agents. LLM-driven control flow goes through coerce-typed
decisions. Data primitives are sufficient for workflow execution and nothing
more. The durable rule/effect kernel is the semantics; the surface should be
scriptable.

Priority order (1 and 4 remove lies; 2, 3, 5, 6 add promised primitives):

1. Harden the soft middle (parser/evaluator unification)
2. `flow` — a sequential surface that lowers to rules
3. Time — effect timeouts and timer effects
4. Regular event matching; work queues
5. Inline `decide` + `case` over choices
6. Capability-gated `exec` effect

---

## Status board

The single source of truth for where every workstream stands. Stages:
**design → spec → model → implement + test → review.** `—` means the stage
does not apply. Update this table as stages complete; details live in the
linked sections below.

Rows 1–9 are complete through review (2026-06-10). Full workspace tests
(12 suites), clippy, Maude + TLA+ models, rule-coverage CI, smoke scripts,
and both acceptance fixtures pass. Row 10 is intentionally deferred (one
deprecation release before removal). Rows 11–17 (Part C, decided 2026-06-10) are complete through review
(2026-06-11): full workspace tests, clippy, rule-coverage CI, and the
live-collector OTel check pass. Noted follow-ups: `acquire ... wait` FIFO
form, ledger `has entry for` projection sugar, TLA+ formalization of the
store-tested coordination invariants, exec `with` stdin, time arithmetic.
Row 18 (C9, decided 2026-06-11) hardens `exec` for hosted deployments:
design and spec are done, build pending.

| # | Workstream | Design | Spec | Model | Impl + test | Review |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Soft middle: body AST + unified evaluator ([B1](#b1-harden-the-soft-middle-priority-1)) | — | — | **done** (`tests/soft_middle.rs`) | **done** | **done** |
| 2 | `flow` ([A1](#a1-flow-surface-shape--decided-2026-06-09)) | **done** | **done** ([flow.md](../flow.md)) | **done** (lowering-equivalence tests in `flow_expand.rs`; approve/reject/fan-out e2e) | **done** | **done** (review found+fixed 3 bugs: trigger rename in branch conditions, multi-flow ask-binding isolation, string-literal rename; regression tests added) |
| 3 | Time: timeout, timer, cancel ([A2](#a2-time--decided-2026-06-09)) | **done** | **done** ([time.md](../time.md)) | **done** (exactly-once race/expiry tests in `soft_middle.rs`) | **done** | **done** |
| 4 | Event matching: `when fact` + sugar ([A3a–c](#a3-regular-event-matching-work-queues--decided-2026-06-09)) | **done** | **done** (`spec/language.md` surface revisions) | — | **done** | **done** |
| 5 | Work queues ([A3d–e](#a3-regular-event-matching-work-queues--decided-2026-06-09)) | **done** | **done** ([work-queues.md](../work-queues.md)) | **done** (claim-atomicity property tests in `items.rs`; e2e chain tests) | **done** | **done** |
| 6 | Inline `decide` + `case` over choices ([A4](#a4-inline-decide--case-over-choices--decided-2026-06-09)) | **done** | **done** (`spec/language.md` surface revisions) | **done** (`inline_decide_completes`) | **done** | **done** |
| 7 | Gated `exec` effect ([A5](#a5-capability-gated-exec-effect--decided-2026-06-09)) | **done** | **done** (`spec/language.md` surface revisions) | **done** (`exec_is_gated_by_operator_grants`) | **done** | **done** |
| 8 | Small cuts: `emit`, `harness`, `pattern` ([A6](#a6-small-cuts--decided-2026-06-09)) | **done** | — | — | **done** (`emit` rejected; `consume` warns) | **done** |
| 9 | Dynamic rule-coverage CI ([B3](#b3-carry-overs-from-the-previous-plan)) | — | — | — | **done** (`scripts/check-rule-coverage.sh`) | **done** (surfaced + fixed 2 latent example dev failures) |
| 10 | Remove `consume` after deprecation window ([B3](#b3-carry-overs-from-the-previous-plan)) | — | — | — | deferred (one release) | — |
| 11 | Sum types: data-carrying variants ([C1](#c1-sum-types-data-carrying-variants--decided-2026-06-10)) | **done** | **done** ([sum-types.md](../sum-types.md)) | **done** (`sum_type_*` e2e in `soft_middle.rs`: tagged dispatch + payload binding per variant, fixture `--variant` knob, reserved-field/bare-binding/exhaustiveness checks) | **done** (brace-body variants lower to `<Enum>.<Variant>` classes with literal `variant` field, visible in `check`; coerce effects embed per-variant fixtures) | **done** (review fixed: case-branch `as` binding in the body parser, runtime variant dispatch, nested-payload splitter capture; full suite + clippy green) |
| 12 | JSON/JSONL ingestion ([C3](#c3-json-and-jsonl-ingestion--decided-2026-06-10)) | **done** | **done** ([json-ingestion.md](../json-ingestion.md)) | **done** (`exec_parse_*` e2e in `soft_middle.rs`: typed bind, fan-out, all-or-nothing stream, fail-branch routing) | **done** (effects carry a self-contained `parse` contract so workers validate without the IR; escape-aware command extraction fixed en route) | **done** (review fixed: escape-aware command extraction — `\"` inside exec commands previously truncated the command) |
| 13 | Scheduled time: absolute deadlines ([C4](#c4-scheduled-time-absolute-deadlines--decided-2026-06-10)) | **done** | **done** ([scheduled-time.md](../scheduled-time.md)) | **done** (`timer_until_*` e2e in `soft_middle.rs`: past fires exactly-once, future pends, path resolves at record time) | **done** (fixed 3 bugs en route: body-AST statement gate, lexicographic `strftime` compare, NULL `timeout_seconds` row mapping) | **done** (review fixed: lexicographic strftime compare, NULL timeout row mapping; e2e re-verified after both) |
| 14 | External signal ingress ([C5](#c5-external-signal-ingress--decided-2026-06-10)) | **done** | **done** ([event-ingress.md](../event-ingress.md)) | **done** (signal payload boundary, typed fact reaction, malformed/undeclared rejection, typed-field checks) | **done** (`signal` decl, bare `when <signal>` typed reaction, `whip signal` CLI target; hosted source providers deferred per spec) | **done** (review verified boundary rejection cannot land ill-typed facts; liveness lint exempts declared signals naturally) |
| 15 | Coordination: lease, ledger, counter ([C6](#c6-coordination-resources-lease-ledger-counter--decided-2026-06-10)) | **done** | **done** ([coordination.md](../coordination.md)) | **done** (protocol invariants pinned by store property tests in `coordination.rs` — mutual exclusion, TTL expiry, N-slot, holder release, append order/partitions, cap + lazy reset — and `lease_*`/`counter_*` e2e in `soft_middle.rs`; TLA+ formalization of the same invariants is the noted follow-up) | **done** (try-acquire/release/`until ttl`, append+retain, consume+lazy reset; workspace-scoped `coordination.sqlite`; `whip leases/ledger/counters`; safety checks: one-lease default, exhaustive outcomes, must-release prototype, terminal auto-release; `acquire wait` FIFO form is the noted follow-up) | **done** (review fixed: policy gate blocked coordination kinds as provider-less, `after` line-scan rejected outcome predicates, acquire bindings missing from seen-bindings; discipline checks exercised against leak/missing-arm/multi-lease programs) |
| 16 | Messaging: durable tuple-space ([C7](#c7-messaging-a-durable-tuple-space--decided-2026-06-10)) | **done** | **done** ([coordination.md](../coordination.md), [event-ingress.md](../event-ingress.md)) | **done** (cross-instance signal injection validates payload against the declared signal shape and lands the typed fact in the peer) | **done** (`emit signal <name> to <instance> { payload }` effect; ledger pub-sub via C6; `has entry for` projection sugar is the noted follow-up) | **done** (review verified payload validation failure fails the signal effect without touching the peer) |
| 17 | Observability: OpenTelemetry export ([C8](#c8-observability-opentelemetry-export--decided-2026-06-10)) | **done** | **done** ([observability.md](../observability.md)) | **done** (emit-once verified against a live collector: second pass exports nothing; `otel_export_dry_run_emits_structural_spans` pins span naming + structural-only attributes) | **done** (`whip otel-export`: cursor-tracked log tailer → OTLP/HTTP JSON traces; standard OTel env vars; spans named after source constructs; `gen_ai.system` on model spans; plain-HTTP to a local Collector which owns TLS/fan-out; metrics + allowlist are the noted fast-follows) | **done** (review verified emit-once against a live collector and failure isolation: a failed POST marks nothing exported) |
| 18 | Script capabilities: content-pinned `exec` ([C9](#c9-script-capabilities-content-pinned-exec--decided-2026-06-11)) | **done** | **done** ([script-capabilities.md](../script-capabilities.md)) | todo | todo | todo |

Sequencing (as executed): row 1 (body AST + unified evaluator) landed first;
rows 2–9 built on it. Flow lowering depends on row 1's AST as predicted.
Row 11 builds on the existing `case`/exhaustiveness machinery (rows 1, 6).
For the Part C build pass: 12 (JSON ingestion) lands first — it unblocks
usable `exec` stdout and is the shared parser behind 14's signal payloads;
then 13 (absolute time, extends the A2 worker pass); then 14a (signal
injection + `when` reaction); 14b (hosted webhooks) is a separate later,
opt-in track. 11 (sum types) slots alongside — parsed/injected payloads can
be tagged variants, and coordination outcomes (15) are sum types. 15
(coordination) generalizes the work-queue store and reuses 11/4/13; its
signal injection + mailbox pattern (16) compose with 14. 16 needs no
standalone build beyond 15's ledger and 14's injection.

---

## Part A — Design decisions

Process: each decided feature is driven through the full development cycle —
**spec -> modeling -> implementation + testing -> review** (pipeline table in
B2). Status: all of Part A decided 2026-06-09.

### A1. `flow`: surface shape — DECIDED 2026-06-09

Core framing: **a flow is a rule whose body is a multi-step sequence.**
`rule` = match → one atomic commit; `flow` = match → a chain of commits
with compiler-managed state between steps. A generalization of rules, not a
construct beside them.

```whip
flow triage
  when Ticket as ticket where ticket.status == "open"
{
  tell triager as turn """markdown
  Suggest an owner and a fix plan for {{ ticket.title }}.
  """
  on fails {
    fail error { reason "triage failed" }
  }

  askHuman as signoff """markdown
  Plan: {{ turn.summary }} — approve or reject.
  """

  when signoff.choice == "approve" {
    complete result {
      decision signoff.choice
      decidedBy signoff.answered_by
    }
  } else {
    fail error { reason "rejected" }
  }
}
```

- [x] **A1a.** Named flows with `when` triggers; any number per workflow;
      rules and flows are peers (a one-step flow is a rule). Rejected:
      "workflow IS a flow" (demotes the reactive core, opens a
      source-vs-runtime explanation gap, breaks free per-fact fan-out,
      invites general-purpose computation) and "one anonymous flow"
      (arbitrary singleton, no principled home for a second sequential
      concern).
- [x] **A1b.** v1 scope: sequential steps with implicit after-succeeds
      chaining; `when/else` branching on step outputs; per-step `on fails`
      / `on timeout` handlers. Handlers are mandatory scope — without them
      any off-path exit strands authors against compiler-generated state.
      `retry N` is designed-for (state fact carries an attempts field) but
      ships v1.5; requires a mandatory bound to satisfy the liveness lint.
      No collection loops, no parallel blocks: fan-out is the `when`
      trigger's job.
- [x] **A1c.** Lowering is visible everywhere, following the existing table
      precedent: generated rules named under the flow
      (`flow.<name>.step<N>`), state facts in a reserved `flow.` namespace
      with `provenance_class: "flow"` and `construct: "flow_step"` spans
      pointing at the step in source. Checker error if user rules read or
      consume reserved flow state. `check` snapshot groups generated rules
      under their flow. First-class "step 3 of 5" presentation may layer on
      top of provenance metadata later; never instead of it.
- [x] **A1d.** Per-fact progressions by design: the `when` clause
      determines fan-out (`when started` = once per instance; fact match =
      one progression per matched fact, throttled by agent capacity).
      Compiler generates the correlation guards routing effect completions
      and human answers to the right progression; state fact identity
      carries a per-row salt to survive byte-identical triggering facts.
      v1 may stage as `when started`-only if implementation demands —
      per-fact arrives later with no syntax change. Documented interim
      fan-out idiom: a rule that `invoke`s a child workflow per item.

### A2. Time — DECIDED 2026-06-09

- [x] **A2a.** `timeout <duration>` clause on any effect. The clock starts
      at **effect creation** (deadline semantics — predictable, anchored to
      a visible log event, unified with timer semantics; run-start clocks
      hide capacity stalls, the failure mode timeouts exist to catch).
      Expiry marks the effect `timed_out` (terminal: fires
      `after ... fails`/`completes` and flow `on timeout` handlers) and
      *requests* provider cancellation — a request, not a result, same
      discipline as `revise --cancel running`. Units: `s/m/h/d`.
- [x] **A2b.** `timer <duration> as x` creates a `timer.wait` effect
      completed by the runtime when due. Rule determinism untouched (timers
      are effects). **Source-level `cancel <binding>` ships in v1** (Jack's
      call, upgrading my defer recommendation): the ask+timer escalation
      race can explicitly cancel the losing effect, keeping inboxes clean
      and not leaving zombie provider work. Cancel of an already-terminal
      effect is a no-op with evidence. Guard-mismatch remains the fallback
      for un-cancelled losers.
- [x] **A2c.** No daemon: timers fire when `dev`/`worker` next runs after
      the due time; external schedulers own wall-clock wakeups.
      **`dev --until idle` treats pending timers as idle** (idle = no
      immediately runnable work) so dev runs never hang on long timers;
      pending timers are visible in `status`.
- [x] **A2d.** Dissolved: `timeout` is uniform across effect kinds, so
      `askHuman as signoff timeout 24h` works with zero special-casing.

### A3. Regular event matching; work queues — DECIDED 2026-06-09

- [x] **A3a.** General form: `when fact <dotted.name> as x [where ...]`.
      Runtime events are stored and matched as facts, so `fact` is the
      truthful keyword; dotted lowercase names cannot collide with
      capitalized user classes.
- [x] **A3b.** Sugar that survives, each documented as a lowering to the
      general form: `started`, `<agent> is available`,
      `human answered ... as x`, `<agent> completed turn ... as x`, plus
      `<queue> has ready item as x` (new, from the queue design). Dropped:
      `manual review requested` (undocumented, unused).
- [x] **A3c.** The `human answered <label>` label is documentary. Flows
      auto-correlate ask -> answer (A1); hand-written rules discriminate
      with guards on the answer payload. No false promise of reference
      semantics.
- [x] **A3d.** Superseded by the **work-queue design**.
      Full spec: [`work-queues.md`](../work-queues.md).
      Summary: `queue <name> { tracker <kind> }` declarations mirroring
      agent/provider; status categories `open | in_progress | done |
      cancelled` (the layer GitHub/Linear/Jira share); `ready` is a derived
      predicate the binding answers, `blocked` does not exist in core;
      verbs `file`/`claim`/`release`/`finish` as standard `queue.*` effects
      (`cancel`/`comment` v1.5); schema `id, title, body, status, labels,
      metadata`; claim = tracker-arbitrated lease, "already claimed" is a
      branchable outcome; **builtin tracker** backed by a workspace-scoped
      SQLite file (`.whipplescript/items.sqlite`, not the run store) as
      default and reference implementation; tracker-native opaque identity
      (builtin issues sequential `WS-n`); polling projection on worker
      passes; agents file mid-turn via `whip items add` with run-identity
      provenance injected into turn environments.
- [x] **A3e.** Builtin `AgentTurn` drops the never-populated `issue` /
      `changedFiles` fields; affected examples rewritten honestly; turn
      enrichment becomes a documented capability a tracker binding may
      provide later. The type system must not promise data that cannot
      exist.

### A4. Inline `decide` + `case` over choices — DECIDED 2026-06-09

- [x] **A4a.** Inline anonymous coercion:
      `decide "<prompt>" -> { fixed bool, reason string } as verdict` —
      lowers to a generated coerce function + class with stable generated
      names (same visibility discipline as flow lowering). Same schema-coercion
      effect family, same `after` branching. Current implementations may still
      expose the legacy `coerce` kind. Rationale: every point of
      friction on the typed-coercion path is an incentive to parse prose
      out of turn summaries — the failure mode the philosophy exists to
      prevent. Reused shapes promote mechanically to named `coerce`.
      Ships alongside `flow` (one-shot decisions cluster in flows).
- [x] **A4b.** `case` over string-literal-union types
      (`"approve" | "reject"`) with exhaustiveness checking; plain
      `string` scrutinees remain rejected.
- [x] **A4c.** `askHuman as signoff choices ["approve", "reject"] "..."`
      declares the choice set in source: types `signoff.choice` as the
      literal union (case-able, exhaustive), and drives the inbox UI
      choices, replacing the JSON-prompt convention.

### A5. Capability-gated `exec` effect — DECIDED 2026-06-09

- [x] **A5a.** `exec "<command>" as x` creates an `exec.command` effect.
      Command string in source; args/env/cwd via config in v1 (keep the
      source surface minimal).
- [x] **A5b.** Grants are **operator-config-only**: an `exec:<glob>`
      allowlist in runtime config. Source declares, config grants, no
      self-granting — the moment source can grant itself, the gate is
      decorative. Ungranted execs land `blocked_by_capability`, visible in
      `effects`.
- [x] **A5c.** Evidence: exit code, truncated stdout/stderr, duration.
      Binding exposes `x.exit_code`, `x.stdout` (truncated); full output as
      evidence artifacts. Non-zero exit = effect `failed` (branchable).
- [x] **A5d.** v1 sandboxing posture: none — a grant is a documented trust
      decision. Sandboxing arrives later as binding/config territory, not
      language.

### A6. Small cuts — DECIDED 2026-06-09

- [x] **A6a.** `emit event.name` is **removed** from the surface: no
      documented semantics, no example usage. Events are the runtime's to
      append. (Can return specified if a real need appears.)
- [x] **A6b.** `harness` demoted to an advanced appendix (docs-only).
- [x] **A6c.** `pattern`/`apply` kept untouched until `flow` ships; then
      re-evaluate remaining genuine use with evidence.

---

## Part B — Approved engineering work (no design input needed)

> **RECONCILED 2026-07-01.** The [status board](#status-board) (rows 1–18) and
> the codebase are authoritative — the Part B features below **shipped** (body
> AST + unified evaluator in `parser/src/body.rs`+`flow_expand.rs`; single-line
> record fix; `{{ }}` interpolation; timers/`cancel`; general `when fact` matcher
> + `manual review requested` removed; work queues + `whip items`; `exec` +
> `WHIPPLESCRIPT_EXEC_ALLOW`; rule-coverage lint; `--exec-profile`/C9). The
> checklist below is retained as historical decomposition; its `[ ]` marks are
> **not** live work except the two genuinely-open carry-overs, which are the
> only remaining items in this whole tracker:
> - **B1a (partial):** the body AST shipped, but *moving lowering out of the CLI
>   crate* is still open (`main.rs` ~50k lines) — folds in review-change-plan §4.11.
>   *Ordering note 2026-07-01:* the durable-object sans-IO refactor
>   (`../durable-object-runtime-tracker.md` Phases 1–4) will restructure the
>   executor half of the same file; do the lowering-move first or jointly, not
>   after — avoid refactoring the same ~50k-line file twice.
> - **B3:** remove `consume` after its deprecation window (it still parses); and
>   the dynamic per-run committed-rule reporting half of rule-coverage CI.
>
> **B1g closed 2026-07-02:** the pull-forward was kept: the no-silent-no-op
> sweep now has a deterministic accepted-body matrix in
> `accepted_rule_body_matrix_has_no_silent_noops`, plus runtime scanner coverage
> in `parse_effect_statements_covers_accepted_body_surface`. The CLI scanner also
> no longer double-advances adjacent single-line effects, which was the concrete
> silent-skip risk the sweep exposed.

### B1. Harden the soft middle (priority 1)

The root cause is shared: rule bodies are raw strings re-scanned line by
line at lowering time, and guards/assertions use two different evaluators.

- [ ] **B1a.** Parse rule bodies into a real AST in the parser crate
      (absorbs the deferred "move lowering out of the CLI" item from the
      previous plan). Every statement form is grammar; unknown tokens are
      parse errors with spans.
- [ ] **B1b.** One expression evaluator shared by guards, assertions, and
      payload lowering.
- [ ] **B1c.** Fix: filtered queries in guards evaluate wrong at runtime —
      `count(Item where status == "done")` / `exists(Item where ...)` are
      false despite matching facts, while the identical expression passes
      as an `assert`. Regression tests pairing guard and assert evaluation
      of the same expressions.
- [ ] **B1d.** Fix: single-line record/payload blocks
      (`complete result { total 2 }`) fail with a misleading
      "missing required field" error. Either support the form or reject it
      with a real diagnostic.
- [ ] **B1e.** Fix: `{{ ... }}` interpolation in record/payload fields
      produces corrupt values (mangled quoting, unsubstituted templates).
      Support it properly or reject at check time.
- [ ] **B1f.** Reject unknown effect modifiers with a spanned diagnostic
      (`tell w as turn timeout 10m "..."` must say "unknown token
      `timeout`", not mis-parse downstream).
- [x] **B1g.** Sweep: fuzz/property tests that any body accepted by `check`
      either executes faithfully or produces a runtime diagnostic — no
      silent no-ops. Closed 2026-07-02 with deterministic parser/CLI accepted-body
      matrices and the adjacent single-line effect scanner fix.

### B2. Implementation work derived from Part A decisions

Stage status lives in the [status board](#status-board) — do not duplicate
it here. This section holds the per-feature work breakdown.

Time (A2):
- [ ] `timeout <dur>` clause parsed on all effect statements; stored on the
      effect; enforced on worker passes (creation-anchored deadline).
- [ ] Expiry path: mark `timed_out`, fire dependency branches, record
      cancellation request + evidence.
- [ ] `timer <dur> as x` -> `timer.wait` effect kind; completed by
      worker/dev passes when due.
- [ ] `cancel <binding>` rule-body operation (v1): terminal-cancels a
      pending effect, requests cancellation of a running one; no-op with
      evidence on already-terminal effects.
- [ ] `dev --until idle`: pending timers do not count as runnable work;
      `status` lists pending timers with due times.
- [ ] Duration literals (`s/m/h/d`) in the expression layer.

Event matching (A3a–c):
- [ ] `when fact <dotted.name> as x [where ...]` general matcher.
- [ ] Re-implement surviving sugar as documented lowerings to the general
      form; delete the magic-prefix table in `normalize_pattern_name` /
      `binding_from_when`.
- [ ] Remove `manual review requested`.

Work queues (A3d–e), per [`work-queues.md`](../work-queues.md):
- [ ] `queue { tracker ... }` declaration; binding config resolution.
- [ ] `queue.file/claim/release/finish` effect kinds + verbs; claim lease
      semantics; branchable already-claimed outcome.
- [ ] Builtin tracker: workspace-scoped store, sequential `WS-n` ids,
      transactional claim.
- [ ] Projection on worker passes; `(queue, id)`-keyed item facts;
      `<queue> has ready item` sugar.
- [ ] CLI: `whip items [add|show|list]`; run-identity env injection into
      turns; provenance stamping.
- [ ] Drop `AgentTurn.issue`/`changedFiles`; keep the curated queue
      examples as the authoring surface.

### B3. Carry-overs from the previous plan

- [ ] Dynamic rule-coverage CI: every rule in every shipped example commits
      at least once in some fixture run.
- [ ] Remove `consume` after its deprecation window.

---

## Part C — Follow-on design (evaluated 2026-06-10)

Two follow-on passes after rows 1–9 shipped, against the same philosophy
anchor. The governing filter: does a feature serve **structuring and routing
typed workflow state**, or does it pull **general computation** into a language
that deliberately routes computation to coerce (semantic) and `exec` (arbitrary)?

- **Pass 1 — a Gleam-inspired batch.** Most candidates declined (C2); one
  taken: sum types (C1).
- **Pass 2 — the workflow boundaries** (data in, time, events in). Three taken
  (C3–C5), unified by one discipline: *impurity lives at the effect/worker
  boundary and is recorded as a durable fact; rules stay pure and replayable.*
- **Pass 3 — distributed-systems coordination.** Two taken (C6–C7), unified by
  one observation: the work-queue is already cross-instance mutable state, so
  the move is to generalize it into a closed family of coordination resources
  (durable tuple-space), made safe by static restrictions + TLA-proven store
  protocols rather than runtime hope. A circuit-breaker primitive was declined
  (it would hide the visible state machine the language exists to expose; it is
  a pattern, shipped as an example).
- **Pass 4 — operational observability (the ambassador).** One taken (C8): the
  event log is already the observability substrate, so export it to
  OpenTelemetry from a log-tailing sidecar — the read-side mirror of C5,
  config-not-language, one OTLP target fanned out at the Collector to every
  enterprise platform.

### C1. Sum types: data-carrying variants — DECIDED 2026-06-10

Driven through the cycle like Part A; spec: [`sum-types.md`](../sum-types.md).

- [x] **C1a.** Enum variants may carry typed payloads via a brace body that
      reuses the class grammar (`Approved { score float }`). Bare variants
      are unchanged; the feature is purely additive.
- [x] **C1b.** The discriminant is **synthesized from the variant name** into
      a reserved literal field `variant "<Variant>"`, never written in
      source, visible in the lowering. One source of truth (the variant
      name); no author-side typo or WS↔coerce desync possible. Not `$variant`
      (reservation risk); a plain `variant` identifier.
- [x] **C1c.** `case x { Variant as b => { ... b.field ... } }` binds the
      payload by `as` (consistent with all other binding) and is exhaustive
      over the variant set (reuses `validate_case_coverage`). Payload access
      outside a matched branch is a check error — the prohibition that forces
      exhaustive handling.
- [x] **C1d.** Runtime is internally-tagged JSON (`{"variant":"Approved",
      ...}`); WS `case` compares the tag exactly because coerce's parser has
      already canonicalized it.
- [x] **C1e.** coerce mapping settled by SAP source evidence, not assumption: a
      sum type is a coerce union of classes; SAP scores each variant and the
      literal discriminant is decisive (matching arm scores 0, wrong arm
      rejected or +100); `match_string` normalizes the tag; no-match yields a
      coerce failure routed to `after <coerce> fails`. WS generates no coerce
      source — it references the user-written coerce type and `check`
      cross-validates via schema hashes. No live-model spike needed.
- [x] **C1f.** v1 scope: flat payloads (scalars, class refs, arrays); named
      via `enum`/`coerce` (inline `decide` stays flat). Held out: generics,
      recursion, nested sum-type payloads, methods.

### C2. Considered and declined

Recorded so the boundary is durable, not re-litigated.

- **Result types / railway error handling** — declined. Redundant with the
  effect lifecycle (four terminal states, durable, retryable — a richer
  Result than `ok|err`); the legitimate kernel (typed failure outcomes from
  decisions) is served better by C1 sum types. Railway `try`/`use` chaining
  would pull control flow into the pure expression layer.
- **General/structural pattern matching** — declined beyond C1. Targeted
  `case` over enums, literal unions, terminal unions, optionals, and now sum
  types covers the workflow-relevant question ("which variant"). Nested/list
  destructuring and arbitrary-shape exhaustiveness are general-language
  features.
- **Regex / string manipulation** — declined. Text→structure is coerce's job
  (`coerce`); regex invites a text-processing language, is a ReDoS/determinism
  hazard, and competes with `coerce`. A bounded set of **total** string
  predicates (`starts_with`/`ends_with`/`contains`, glob `matches`) for guard
  routing is the only part worth a future, separate workstream — regex itself
  is refused.
- **Type assertions / casts** — declined as general casts (they undermine the
  schema-validation boundary `coerce` already enforces). A narrow type
  **ascription** on an untyped `when fact <dotted>` binding is the only
  legitimate slice and is a candidate for a future bounded workstream.
- **Nil / null type** — declined; a regression. Absence is already modeled by
  optionals (`T?`) with `some`/`none` `case` matching; a pervasive nil would
  erase the present-unless-optional guarantee that keeps facts auditable.
- **Block comments** — accepted as a trivial, separable paper-cut (`//` line
  comments already work); not a tracked workstream, fold into any convenient
  parser change.

### C3. JSON and JSONL ingestion — DECIDED 2026-06-10

Pass 2. Spec: [`json-ingestion.md`](../json-ingestion.md). One primitive:
validate JSON against a declared schema, deterministically, no model — the
non-LLM sibling of `coerce`, shared by `--input`, effect output, and event
payloads (C5).

- [x] **C3a.** A text-returning effect declares a parse target with `->`
      (read "typed as", consistent with `coerce ... -> Type`). v1: `exec`.
- [x] **C3b.** Single: `exec "cmd" -> Report as x` binds the typed value in
      `after x succeeds as r`. Effect success = exit 0 AND stdout parses;
      a non-zero exit or a parse failure both route to `after x fails`
      (one branchable outcome, reusing the effect lifecycle).
- [x] **C3c.** Stream: `exec "cmd" -> each Report` records one fact per
      JSONL line / JSON-array element, reacted to by ordinary per-fact rule
      fan-out (no loops). All-or-nothing: a malformed line fails the effect;
      no partial-stream commit.
- [x] **C3d.** A schema is required — no untyped/jq access; want raw, keep
      the string `x.stdout`. Outbound JSON production stays covered by
      `record`/payload construction. Targets may be sum types (C1).

### C4. Scheduled time: absolute deadlines — DECIDED 2026-06-10

Pass 2. Spec: [`scheduled-time.md`](../scheduled-time.md). Extends the A2 time
model. Load-bearing rule: **the clock is read only at the worker boundary;
`now` never exists in a guard.**

- [x] **C4a.** New scalar `time` (ISO 8601 instants), as a schema field type
      and as a quoted-string literal in `time`-typed positions. `time` values
      are comparable in guards (pure — both operands recorded); `now` is never
      available in a guard.
- [x] **C4b.** `timer until <time-expr> as x` — absolute deadline; the
      operand is a `time` literal or `time`-typed path. Distinguished from the
      relative `timer <duration>` by `until`; both lower to `timer.wait`,
      differing only in deadline basis. Fires on the next worker pass at or
      after the target; recorded as a `timer.fired` fact (replay-safe, no
      daemon).
- [x] **C4c.** Out of scope v1: recurrence/cron (exact cron needs the rejected
      daemon; recurring needs use an external scheduler + `whip signal`, C5);
      absolute `timeout` on effects (`timeout` stays relative-only).

### C5. External signal ingress — DECIDED 2026-06-10

Pass 2. Spec: [`event-ingress.md`](../event-ingress.md). An authenticated
external signal becoming a durable typed fact is the event-sourced core
generalized (the inbox, generalized). Split at the language/operations seam.

- [x] **C5a (3a, core).** `signal <dotted.name> { schema }` declares a typed
      external signal (class-body grammar). It is the typed ingress manifest and
      subsumes the `@external` liveness escape for declared signals.
- [x] **C5b (3a, core).** Reaction reuses `when` — a declared signal gets the
      typed bare form `when deploy.finished as d`; undeclared dotted facts keep
      the untyped `when fact <name>`.
- [x] **C5c (3a, core).** Injection: `whip signal <instance> --name <name>
      --data <json>`; `--data` is parsed against the signal schema via the C3
      primitive. A payload failing validation is rejected at the CLI boundary
      (no ill-typed fact). No server required — an operator gateway verifies a
      webhook and shells out to `whip signal`. Replay-safe.
- [x] **C5d (3b, opt-in ops).** Hosted webhooks are a runtime mode
      (`whip serve --webhooks`) configured, not coded: a config maps a signal
      to an endpoint + auth strategy (`hmac`/`bearer`/`shared-secret`) with
      secret *references* (`credentials_ref` model). No source syntax.
      Separable, later track. Deferred to 3b's design pass: event-starts-vs-
      advances-instance, payload→instance correlation, TLS/DoS/retry posture.
- [x] **C5e.** Does this remove extensions? It right-sizes them. Events-in +
      `exec`-out + JSON typing make extensions **optional for integration
      glue**; providers (agent backends) and synchronous in-process
      capabilities remain irreducibly theirs. A sharpening of the philosophy,
      not a removed subsystem.

### C6. Coordination resources: lease, ledger, counter — DECIDED 2026-06-10

Pass 3. Spec: [`coordination.md`](../coordination.md). The work-queue is already
cross-instance mutable state; generalize it into a closed family.

- [x] **C6a.** A closed family of shared coordination resources —
      `queue` (have it), `lease`, `ledger`, `counter` — each declared like a
      queue (workspace-scoped builtin), mutated by atomic branchable effects,
      projected to facts, attributable by instance/run provenance. Closed, not
      an open "shared cell": no `get`/`set`/query surface, so it cannot slide
      into a database.
- [x] **C6b.** Three architectural principles: (i) typed domains keyed on
      entities you already model (no string namespaces); (ii)
      atomic-attempt-and-branch, never read-then-act (no op reads shared state
      into a guard); (iii) held resources bounded by holder-lifetime + TTL.
      These kill the stringly-typed, TOCTOU, and leak classes respectively.
- [x] **C6c.** Reversible/irreversible split: a semaphore folds into `lease`
      as an N-slot lease (leak-safe; what agent `capacity` already is);
      `counter` is consumable-only (decrement against a cap, scheduled reset,
      cannot leak-hold). Three non-overlapping shapes.
- [x] **C6d.** Coordination outcomes **are sum types** (`Held(slot) |
      Contended`, `Ok | Over`) with compiler-enforced exhaustive handling (C1)
      — assume-success-under-contention is unrepresentable.
- [x] **C6e.** Safety model, local and predictable (no environment-dependent
      compilation): **at-most-one-held-lease per progression is the hard
      default** (breaks Coffman hold-and-wait); multi-lease requires an
      explicit workspace-level `lease order` checked locally per acquire path
      (breaks circular-wait); linear must-release on every terminal path or
      explicit `until ttl` (forgotten release is a compile error); exhaustive
      outcomes (C6d). TTL is the crash net, not the forgot-to-write net.
- [x] **C6f.** Formal modeling: one TLA+ spec per resource in the existing
      rig — lease (`MutualExclusion`, `NoDeadlock`, `BoundedWait`), counter
      (`CapInvariant`, `NoLostConsume`), ledger (`AppendLinearizable`,
      `NoLostEntry`, `PartitionIsolation`); the Rust store refines the model
      via trace conformance. Tractable because all concurrency funnels through
      a handful of atomic store ops.
- [x] **C6g.** Liveness (undecidable statically) is a store guarantee:
      FIFO-fair granting + mandatory TTL gives bounded waiting; instance-terminal
      release gives crash safety. Single-host coordination in v1, like the
      queue (multi-host is a larger, separate story).
- [x] **C6h.** Declined: a circuit-breaker primitive — it would hide the
      visible state machine the language exists to expose. A consecutive-failure
      breaker is expressible as facts + rules + `timer until` (C4); ship it as a
      documented example, not a feature.

### C7. Messaging: a durable tuple-space — DECIDED 2026-06-10

Pass 3. Specs: [`coordination.md`](../coordination.md#messaging-a-durable-tuple-space),
[`event-ingress.md`](../event-ingress.md). No actor/channel primitive.

- [x] **C7a.** WhippleScript coordinates through shared durable state, not
      direct messaging — a durable tuple-space, decoupled in identity and time,
      which preserves inspectability and replayability. Documented as the
      guiding model so authors reach for the family, not channels.
- [x] **C7b.** Messaging patterns map onto existing resources: broadcast/
      pub-sub -> `ledger`; competing consumers -> `queue`; spawn/delegate ->
      `invoke`; point-to-point mailbox -> a `ledger` partitioned by recipient
      identity (`when mailbox has entry for me`). No new primitive for any.
- [x] **C7c.** The one thin addition: an in-workflow `emit signal <name> to
      <instance> { payload }` effect — C5's injection turned inward, landing a
      typed durable signal fact in a target instance. Still "inject a durable
      signal," not "open a channel"; no liveness coupling, replay-safe.
- [x] **C7d.** Synchronous peer request/reply is deliberately unsupported —
      it is usually a sign you want an effect (`coerce`/`call`/`exec`), not a
      peer round-trip; scatter-gather is `invoke` + a ledger barrier.

### C8. Observability: OpenTelemetry export — DECIDED 2026-06-10

Pass 4. Spec: [`observability.md`](../observability.md) (resolves that doc's prior
open question). The event log is already the observability substrate; export
it, do not instrument.

- [x] **C8a.** One target: **OpenTelemetry (OTLP)**, fanned out at the OTel
      Collector to any backend. No per-platform exporters — one OTLP emitter
      covers Datadog/Honeycomb/Splunk/Grafana/clouds and platforms not yet
      built.
- [x] **C8b.** The ambassador is a **cursor-tracked log-tailing sidecar**
      (`whip otel-export`), event log as buffer, reusing the existing
      `TraceRecord` projection. Gives zero hot-path overhead, failure isolation
      (a down collector never breaks execution), and emit-once/replay-safe
      (no double-counting). In-process export rejected.
- [x] **C8c.** Ergonomics: honor the standard OTel env vars
      (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`,
      `OTEL_EXPORTER_OTLP_HEADERS` for auth as credential references, ...).
      Authors write nothing; observability is never in source; off by default
      with zero overhead.
- [x] **C8d.** Three pillars: traces first (spans named after source
      constructs — traces read like the workflow; retroactive long-lived
      traces; agent turns aligned to OTel **GenAI** `gen_ai.*` conventions for
      LLM-observability tools); metrics as log projections incl. coordination
      metrics (lease contention, counter-vs-cap, ledger rate) — OTLP push v1,
      **Prometheus pull fast-follow**; logs (correlated) later.
- [x] **C8e.** Clarity: a documented, versioned `whipplescript.*` semantic
      convention reusing the existing correlation fields; `gen_ai.*` alignment
      version-pinned (tracks a stabilizing spec).
- [x] **C8f.** Content policy: **structural by default** (ids/kinds/statuses/
      timings/counts), **never content** (prompt bodies, fact values); richer
      attributes opt-in via an **operator-config allowlist** (the operator owns
      cardinality/compliance), which may only name declared schema fields.

### C9. Script capabilities: content-pinned `exec` — DECIDED 2026-06-11

Pass 5 (hosted-deployment hardening). Spec:
[`script-capabilities.md`](../script-capabilities.md). Motivating deployment: an
LLM authoring WhippleScript for a non-technical user, with no bash tool and no
ability to write or run scripts; raw `exec` + `WHIPPLESCRIPT_EXEC_ALLOW` is a
dev convenience gate, not a security boundary (prefix-match in front of
`sh -c` admits metacharacter injection; path whitelists don't survive
file-writing workers).

- [x] **C9a.** **Identity by content, not name.** The operator manifest pins
      each script's sha256; a worker editing the file unpins it and the
      capability fails closed, loudly. "Agents can't edit the whitelist"
      enforced by arithmetic, not permissions; works in fully shared
      workspaces; updates are explicit audited operator acts.
- [x] **C9b.** **No shell; arguments are data.** Hosted source is
      `exec <name> with <record> -> <Type>`: argv from the manifest, typed
      record on stdin (the C3 mirror form), output typed by `->`. The
      injection surface is absent by construction.
- [x] **C9c.** **Reuse the policy gate.** A manifest entry registers as
      capability `script.<name>`, bound per program; unbound blocks as
      `blocked_by_capability`; profiles express per-orchestrator tiering.
- [x] **C9d.** **Three planes.** Harness (trusted, user authority) curates
      the manifest and drives time (worker passes, heartbeats); the
      orchestrator `.whip` reaches the world only through names (no verb
      writes files); workers may write any bytes but cannot make them
      executable as a capability. Execution-time manifest resolution; the
      run evidence records the executing hash; replay re-reads the record.
- [x] **C9e.** **Exclusions and harness obligations.** The `whip` binary is
      never whitelistable (control-plane minting); worker sandboxes must
      exclude the manifest path; manifest changes require explicit human
      confirmation with the diff shown — the curator is an LLM, and the
      human gate closes the social half of the attack.
- [x] **C9f.** **Tiers.** Dev keeps raw `exec` behind the env allowlist
      (documented as a convenience gate); hosted rejects raw `exec` at check
      and at the worker. Deferred: argv schemas beyond stdin, signed
      manifests (multi-host), network script fetch.

## Proposed — v1 surface-hardening design pass (2026-07-01)

- [x] **P1. `prompt` — bare model prompt → text.** DECIDED (Jack). The most
      basic model pattern is missing: `coerce`/`decide` are structured and `tell`
      needs an `agent` decl + full turn lifecycle, so there is no lightweight
      `prompt "…" → text`. Add a `prompt "<text>" [using <provider>] as r`
      effect — a **free-text-output sibling of `decide`**, reusing the same
      provider backend (no `agent` decl, no turn lifecycle). IFC treatment is
      identical to `coerce` (the prompt is an egress; the result is a low-integrity
      model output). Result type is a plain string reachable via `after r
      succeeds`. Small: parse + lower (reuse the Coerce backend path) + IFC
      (reuse coerce's) + test. No new model needed (coerce-shaped invariant).
      Shipped 2026-07-02: parser/IR/runtime scanner/native/fixture coerce path
      support `prompt`, including provider override, string payload binding, and
      matrix/input-shape coverage. Course-correction same day: the new `prompt`
      keyword made the runtime effect scanner misparse record-field lines named
      `prompt` (fixture-table rows) as effects, colliding effect idempotency
      keys; fixed by skipping record blocks in `parse_effect_statements` and
      requiring the `as` binding on the prompt branch
      (`parse_effect_statements_skips_record_block_fields` +
      `dev_provider_language_*` regressions).
