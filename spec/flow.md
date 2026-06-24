# Flow: sequential surface lowering to rules

Status: spec drafted 2026-06-09 from decided design
([`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md) A1).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**A flow is a rule whose body is a multi-step sequence.**

- `rule` = match -> one atomic commit.
- `flow` = match -> a chain of commits with compiler-managed state between
  steps.

A generalization of rules, not a construct beside them. Flows introduce no
runtime semantics: a flow lowers entirely to ordinary rules, facts, and
effects, all visible to inspection.

## Surface

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
  on timeout {
    fail error { reason "triage timed out" }
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

### v1 scope

- Any number of named flows per workflow; flows and rules are peers.
- A flow header takes the same `when` clauses as a rule (including grouped
  form and guards).
- Body statements, executed as a chain:
  - **Effect steps**: `tell`, `askHuman`, `coerce`, `decide`, `call`,
    `invoke`, `exec`, queue verbs, `timer`. Each step implicitly waits for
    the previous step's success before its effect is created (implicit
    `after <prev> succeeds`). Step bindings (`as turn`) are in scope for
    all subsequent steps.
  - **Non-effect statements**: `record`, `done`, `cancel`, `complete`,
    `fail` — commit with the step they follow.
  - **Branching**: `when <expr> { ... } else { ... }` over previously bound
    step outputs; deterministic, same expression language as guards.
  - **Handlers**: `on fails { ... }` and `on timeout { ... }` attached to
    the preceding effect step. Handler bodies are full statement lists
    (commonly `release`, `record`, `fail`). An unhandled failure stalls the
    progression visibly (facts/effects show the terminal effect; no silent
    drop). A stalled progression is a liveness hazard, not an acceptable
    terminal: when the flow is its workflow's only terminal path, every
    failure/timeout branch must be handled in a way that reaches a workflow
    terminal, or the liveness lint fails (see [Static checks](#static-checks)).
    Step terminals use the canonical union
    ([expression-kernel.md](expression-kernel.md)): `on fails` matches
    `Failed<E>` and `on timeout` matches `TimedOut` (status `timed_out`).
- Excluded from v1: `retry N` (designed-for: the progression state fact
  reserves an `attempts` field; ships v1.5 with a mandatory bound),
  collection loops, parallel blocks (fan-out is the `when` trigger's job).

## Lowering

Each flow lowers to one rule per step plus a progression-state fact class.

- Generated rule names: `flow.<name>.step<N>` (and `.step<N>.on_fails`,
  `.step<N>.branch` variants).
- Progression state: a reserved class `flow.<name>.state` (namespace
  `flow.` is reserved; user classes cannot start with it) holding: the
  triggering fact's identity and bound fields, completed step bindings'
  payloads, the current step index, a per-progression salt, and a reserved
  `attempts` field for v1.5 retry.
- Step rule shape: `when fact flow.<name>.state as s where s.step == N`
  plus the step's effect-completion match, correlated to the progression
  (see below); body consumes the old state fact and records the advanced
  one (`done s -> record ...`) in the same commit as the step's effect.
- Terminal statements (`complete`/`fail`) lower into the step rule that
  reaches them; the state fact is consumed.
- Provenance: generated rules and state facts carry
  `provenance_class: "flow"` and `source_span.construct: "flow_step"`
  spans pointing at the step in source. `check` groups generated rules
  under their flow in the snapshot.

### Correlation

The compiler generates the guards humans get wrong:

- Effect completions are routed to the right progression by matching the
  completion fact's `effect_id` against the effect id recorded in the
  progression state when the step's effect was created.
- Human answers route by `answer.effect_id` against the recorded ask
  effect id — the auto-correlation that makes the `human answered` label
  unnecessary inside flows.

### Fan-out (per-fact progressions)

The `when` clause determines multiplicity: `when started` = one progression
per instance; a fact match = one progression per matched fact, throttled by
agent capacity as usual. State identity includes a per-progression salt so
byte-identical triggering facts cannot collapse into one progression.
Staging: v1 may ship `when started`-only if implementation demands;
per-fact arrives later with no syntax change.

## Static checks

These are owned by [static-analysis.md](static-analysis.md) (which treats the
generated step rules and `flow.`-namespaced facts as first-class for
read/write-set, effect-safety, and cycle analysis). All severities use the
canonical `error | warning | info | hint` enum.

- User (non-generated) rules that read, match, consume, or `record`
  `flow.`-namespaced facts are checker errors. A user class name may not begin
  with `flow.`. Only the owning flow's generated rules may touch its progression
  state.
- Liveness: a flow counts as a workflow's terminal path only when **every**
  branch reaches a workflow terminal (`complete`/`fail`). This includes each
  handled `on fails` and `on timeout` handler and both arms of every internal
  `when ... { } else { }`. A reachable step whose failure/timeout is unhandled
  stalls the progression and therefore leaves that branch with no terminal — so
  the no-terminal-path liveness lint **fails** for a flow that is the workflow's
  only terminal path. A flow does not satisfy liveness merely by having *some*
  path to `complete`/`fail`. Flow steps' generated reads are exempt from the
  dead-rule lint (their producers are generated alongside).
- `on fails` and `on timeout` handlers are checked like rule bodies (terminals
  validated against contracts, bindings in scope).

## Dependencies

Requires B1 (real body AST): flow bodies nest statements and handlers that
the line-oriented scanner cannot host, and trustworthy flow guards require
the unified evaluator.

## Modeling notes

- Lowering equivalence: a flow and its hand-written rule expansion produce
  identical event/fact/effect traces on the same inputs (golden tests; the
  generated rules ARE the semantics, so this is a structural property).
- Progression isolation: two progressions of the same flow never read each
  other's state (correlation guard property tests; byte-identical trigger
  facts produce distinct progressions).
- Handler coverage: a failing step with a handler runs exactly the handler;
  without a handler, the progression stalls with the state fact and
  terminal effect inspectable (no silent drop). The stall is observable
  behavior, not a sanctioned terminal: when the flow is the workflow's only
  terminal path, an unhandled failure/timeout branch is a liveness-lint
  failure at compile time (see [Static checks](#static-checks)), so a shipped
  workflow cannot rely on a silent forever-stall.
