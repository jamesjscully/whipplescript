# DR-0024: Owned brokered agent harness — founding premise and invariants

Status: accepted 2026-06-24 (founding premise + invariants). Opens a new harness
*mode* alongside the existing delegating one
([`agent-harness.md`](../agent-harness.md), DR-0015..0018). This record locks
**only** the premise and the three invariants every downstream decision must
preserve. The durability granularity, governance-capability map, sandbox
mechanism, and lifecycle/replay formalization are deliberately **left open** and
tracked as later steps (see "What this record does not decide").

## Problem

Today an `agent.tell` effect is executed by *delegating* the entire agent turn to
a provider's own native harness — Codex's App Server, the Claude Agent SDK
sidecar, Pi RPC (DR-0015..0018, [`agent-harness.md`](../agent-harness.md)). That
model is correct for what it is: a way to run a heavyweight external coding agent
and capture a normalized terminal summary plus opaque evidence.

But the turn-access grant in that model is **authority-narrowing metadata, not an
enforced boundary**. The provider's native process is the thing that actually
opens files and runs shells. WhippleScript computes an "effective intersection"
of store policy, source grant, provider capability, and profile and hands it to
the adapter — and then trusts the external process to honor it. Three
consequences follow, and all three are structural, not incidental:

```text
1. Boundaries are advisory. A `lease` over a workspace, a `counter` budget, a
   `file store` path policy, a `capability` gate — these live OUTSIDE the
   provider process. whip cannot enforce "this turn may write only under src/,
   may run only these commands, and dies at 50k tokens." The provider's own
   sandbox is the real boundary, and it is the provider's, not ours.

2. Redaction is a necessity, not a policy. In-turn tool calls are recorded as
   shape-redacted evidence because they cross a process boundary into a system
   we do not control. We redact because we cannot see in cleanly, not because we
   chose to.

3. The event stream is a summary. Per-tool actions are reachable only through
   `evidence_refs`/`artifact_refs` as provider-incompatible stream concepts, not
   as native whip events we can trace, replay at fidelity, or count.
```

None of these is a bug in the delegating harness — they are the *price of
delegation*. The capability we cannot reach by delegating, at any price, is
**an enforced, coordinated boundary on agent execution**: the same lease /
counter / file-store / capability machinery that governs orchestration *between*
turns, governing the inside of a turn *for real*. That capability is the entire
reason to build anything new here. If we were not willing to make the boundary
enforceable, the delegating harness already exists and we should just use it.

## Decision

Introduce the **owned (brokered) harness** as a first-class harness mode,
intended to become the default agent-turn executor. The provider-native
harnesses (Codex, Claude, Pi) are **not removed**: they are reframed as the
*external harness* mode — "bring a bigger external agent when the task needs
one" — and demoted from default. Both modes satisfy invariants I2 and I3 below;
only the owned harness satisfies I1, and only I1 makes boundaries enforceable.

The owned harness is `coerce`'s direct-model-call plumbing generalized from a
single structured decision to a multi-turn tool-use loop. The model gets the
raw, trained-on tools it is good at (read / write / edit / bash, designs
borrowed from the open-source Codex/Pi harnesses — see step 2); whip's
structured primitives are **not** in the model's tool list. They are the
*enforced envelope around the loop*.

This record locks three invariants. Everything downstream must preserve them.

### I1 — Brokered execution

> whip executes every tool the model requests. The model's process never
> directly touches the filesystem, the shell, or the network. The model emits a
> tool *request*; whip runs it under policy and returns the result.

This is the founding premise and the source of every benefit over delegation.
Because whip is the executor:

- The boundary has teeth. A `lease` decides which workspace the loop may touch;
  a `file store` policy is the path sandbox for `write`/`edit`; a `counter` caps
  the budget and the loop dies when it is spent; a `capability` gate decides
  whether the turn may run `bash` at all — all enforced at the moment of
  execution, not advised before it. Confining a command whip itself runs reuses
  the existing `exec` allow-list / confinement
  ([`script-capabilities.md`](../script-capabilities.md),
  `WHIPPLESCRIPT_EXEC_ALLOW`) rather than wrapping a foreign process from
  outside.
- Redaction becomes a policy choice, per tool, not a necessity. whip sees every
  tool request and result in cleartext before deciding what is recorded as
  governed data and what is evidence.
- Every tool action is a native whip event, traceable and replayable at
  fidelity, not an opaque provider stream reference.

The corollary is the cost we accept knowingly: whip takes on being the executor
of `bash` and `edit`. The sandbox is now whip's responsibility (step 4), which
is exactly why brokering makes the sandbox *feasible to enforce* in the first
place — see the sandbox feasibility note below.

### I2 — A turn is a leaf

> A turn is a single node in the effect graph. Its internal loop is model-driven,
> and that is expected and correct. Nothing inside the loop becomes a
> rule-matchable fact or chooses which rule fires.

This reaffirms and extends the existing rule that in-turn tool invocations are
evidence and "never become rule-matchable child facts"
([`agent-harness.md`](../agent-harness.md) §Provider Adapter Contract). It holds
identically for the owned harness: owning the loop's internals does not change
the turn's node-ness. The turn was always a model-driven black box; we are only
moving who runs its tools, not promoting its insides into the orchestration
graph.

This is the clean split between WhippleScript's two trust postures: **distrust
the LLM for control flow *between* steps; trust it for the work *within* a
step.** A raw tool-use loop that depends on the model to terminate is fully
consistent with that — termination of a leaf was always the model's job.

### I3 — No upward control-flow leak

> The only things that cross the turn boundary upward are the terminal outcome,
> the declared structured result, and evidence. The model never selects an
> orchestration step, emits an orchestration fact, or escalates its own
> authority. Budgets, leases, and the sandbox bound the loop from outside and may
> terminate it; the loop cannot widen them.

I3 is what keeps I1's power from corrupting the invariants whip works to protect.
The loop is unstructured by nature; I3 forbids that unstructured nature from
leaking up into the structured layer. The static checks the language enforces —
liveness, producibility, namespace discipline, deterministic replay — are
properties of the graph *between* leaves; they neither reach into a leaf nor may
be subverted from inside one.

### Refining I3 — shared-state participation is not control-flow leak

I3 bans the model from *directing* the orchestration. It does **not** ban the
model from *participating in durable shared state* that the orchestration
independently consults. The two are mechanically distinct:

```text
banned (control-flow direction)      allowed (shared-state participation)
---------------------------------    -----------------------------------
record <fact>  — injects into the     add_todo  — writes an item to the durable
fact-base, the substrate rules        work tracker, an external store rules
match on (direct control flow)        choose to observe via `when <tracker> ...`
select which rule fires               read tracker / ledger / memory
spawn / advance an instance           (the workflow's rules decide what, if
emit signal to a chosen instance      anything, to do about the new state)
  (pointed; gray-zone, deferred)
```

The discriminator: **facts are the control-flow substrate**, so writing them is
direction; **the tracker is a durable store the workflow polls**, so writing to it
is participation. This is what lets an agent file an *emergent* to-do mid-turn —
a follow-up the workflow could not have known to create — without the model ever
choosing the next step. The agent-callable surface is therefore exposed as
**tools that are facades over governed whip capabilities** (a `file store` facade
and a tracker `add_todo` facade are the same projection mechanism), each gated
per-capability — never the raw coordination primitives, and never `record`. The
v0 tool surface is specified in
[`owned-harness-tool-surface.md`](../owned-harness-tool-surface.md).

### Corollary — guarantees live at the boundary; the interior is invariant-exempt

I1–I3 imply a principle worth stating outright, because it resolves the step-3
durability question:

> Guarantees live at the turn boundary. The interior is model-driven and
> **explicitly exempt from workflow invariants** — no idempotency, no
> exactly-once, no producibility. Tool calls are *appended to a stream*; they are
> not committed effects.

The interior's non-idempotence is a **feature**, not a hazard: an `edit`/
`apply_patch` consumes its anchor on success, so a retry that finds the anchor
gone tells the model "this already applied" — which is exactly how the model
self-corrects a misaligned edit. Imposing exactly-once on a tool call would fight
the mechanism that makes the loop work. (The taxonomy note's earlier "commit
before execute + dedup per tool" framing was an over-application of the workflow
keystone and is retracted there.) The term **sub-effect** is therefore avoided
for tool calls: a tool call is a stream event, not an effect.

A turn touches three kinds of state, and only the third carries guarantees:

```text
1. external world state   filesystem / workspace. Source of truth = the world.
   (layer 1)              The model mutates, re-observes, reconciles. No
                          exactly-once: the model is the reconciler (same reason
                          anti-idempotent edits are fine).
2. the narrative          message / tool-call / tool-result history. Source of
   (layer 2)              truth = the append-only event stream. Evidence-grade.
3. the boundary           terminal outcome + structured result crossing UP into
   (layer 3)              orchestration. The ONLY place keystone invariants
                          (record-once, exactly-once, the turn-leaf rules) apply.
```

### Crash recovery is resume-from-projection

Decided: a crash mid-turn recovers by **resume-from-projection** — rebuild the
model's context by projecting the event stream up to the crash, then continue the
loop — *not* by re-running the turn.

Re-run would require the turn to be all-or-nothing, which means **undoing** the
side effects already on disk: a snapshot/rollback subsystem (worktree,
copy-on-write, staged overlay). Resume undoes nothing — already-applied edits
stay applied, the model continues against the world as it actually is — and it
reuses the *same projection* the harness already needs for compaction and
initial-prompt assembly. So resume adds no new machinery; re-run adds a rollback
engine.

Resume requires only: append the tool **call** before executing and the tool
**result** after, and a projection that **tolerates a dangling final call** (the
crash window). A re-presented dangling call is safe precisely because of the
layer-1 reconciliation above. This also reinforces the v0 deferral of persistent
sessions (below): a live PTY/long process cannot be resumed across a crash
without process-level checkpointing, so fresh-spawn-only shells keep resume clean.

## Why brokered, stated once and plainly

- Enforced boundaries are the **only** capability unreachable by delegation.
  Clean native events, fidelity replay, and removing the adapter seam are real
  but are *nice-to-have* — none of them alone would justify owning a loop. They
  ride along for free *if* we build for the boundary reason.
- The decision therefore hinges on one question, already answered yes:
  do we want enforced, coordinated boundaries on agent execution? If yes, own
  the loop. If "advisory budgets + trust the provider's sandbox" were enough, we
  would keep delegating and build nothing.

### Sandbox feasibility note

I1 dissolves what looked like the scariest open risk — "can we sandbox `bash`
better than codex already does?" Under delegation the sandbox question is "how do
we confine a foreign process from the outside" (OS-level, weak). Under brokering
it becomes "how do we confine a command *we run ourselves*" — which is the
`exec` capability we already ship, extended. The sandbox stops being an
existential feasibility question and becomes a bounded engineering one (step 4).

## Relationship to the existing delegating harness

```text
delegating / external harness   owned / brokered harness
(DR-0015..0018, agent-harness)  (this record)
------------------------------  ------------------------------
provider native process runs    whip runs the tools
the tools
boundary = advisory metadata    boundary = enforced at execution (I1)
redaction = necessity           redaction = per-tool policy
tool calls = opaque evidence    tool calls = native whip events
default today                   intended default; delegating becomes opt-in
                                "bring a bigger external agent"
satisfies I2, I3                satisfies I1, I2, I3
```

The exactly-once / `uncertain`-terminal and record-once replay rules in
[`admission-and-idempotency.md`](../admission-and-idempotency.md) continue to
govern the *turn* as a leaf effect (layer 3) under both modes. They do **not**
reach inside the loop: per the boundary corollary above, interior tool calls are
stream events, not committed effects, and carry no exactly-once guarantee.

## What this record does not decide

These are explicitly open and sequenced as the next design steps:

```text
step 2  Tool taxonomy. A scoped read of the open-source Codex/Pi tools:
        which mutate vs are pure reads, idempotency, per-turn cadence, and the
        compaction model. Taxonomy, not a reimplementation study. Feeds step 3.
        DONE 2026-06-24 -> spec/owned-harness-tool-taxonomy.md

step 3  Event-stream + projection contract (reshaped by the boundary corollary;
        was "durability granularity"). The interior is a stream, not committed
        sub-effects, so the spine is: the event types appended inside a turn; the
        projection function that derives model context from the stream (carrying
        the borrowable compaction designs in the taxonomy note); how the
        layer-3 terminal re-engages the keystone; and the per-tool redaction
        policy (which stream events persist in cleartext vs shape-redacted).
        Resume-from-projection (decided above) is the recovery instance of this
        same projection. Only decidable after steps 1-2.

step 4  Governance-capability map + sandbox mechanism (parallelizable once step 3
        lands): which primitive gates which brokered tool (lease = workspace,
        counter = budget, file store = path sandbox, capability = bash gate), and
        how whip confines the commands it now runs (extend `exec` confinement).
        Note: `bash` classification is now a step-4 sandbox/redaction concern, not
        a durability one (the boundary corollary removed durability as a reason to
        classify it).

step 5  Lifecycle + invariants, modeled first per the standing discipline:
        TLA+ for turn start / cancel / crash-recovery (resume-from-projection) /
        replay; Maude for any rule-surface or compiler additions.
```

## Deferred capabilities (recorded hooks, not v0)

Explicitly out of scope for v0, captured so the design leaves room for them:

```text
persistent sessions   Codex-style exec_command + write_stdin live PTY sessions
                      that survive across tool calls/turns. They are layer-1
                      external-world resource state and cannot be resumed across
                      a crash without process-level checkpointing. v0 uses
                      fresh-spawn-only shells (Pi-style); revisit as a session
                      resource modeled like leases/claims (with cancel cleanup).

atomic-turn isolation Roll a turn back cleanly (snapshot / git worktree /
                      copy-on-write). NOT needed for crash recovery (resume
                      handles that) — its value is speculative/abortable turns,
                      parallel turns on one workspace, and what-if/revision. When
                      added, it composes with step 4: an isolated turn is one
                      whose workspace `lease` binds a worktree and whose `file
                      store` sandbox is scoped to it, committed on the success
                      terminal and discarded on failure. "Leave room to evolve."
```

## Consequences

What downstream work must preserve, in one place:

- The model never holds raw OS authority; it requests, whip executes (I1).
- whip-primitive governance is the envelope, never the model's tool list.
- A turn stays one leaf node; in-turn actions never become rule-matchable facts
  or drive orchestration (I2, I3).
- The model may participate in durable shared state (the tracker) via
  capability-facade tools, but never write the fact-base or direct control flow
  (refined I3); v0 exposes the tracker only ->
  [`owned-harness-tool-surface.md`](../owned-harness-tool-surface.md).
- Guarantees live at the boundary; the interior is invariant-exempt. Tool calls
  are append-only stream events, never committed effects; their non-idempotence
  is a feature (the boundary corollary).
- Context is a projection over the event stream; crash recovery is the
  resume-from-projection instance of that projection.
- The model's raw tools (read/write/edit/bash) and compaction are borrowed from
  proven open-source harness designs, not invented; what whip adds is the
  event-sourcing wrapper and the enforced envelope, not a new coding agent.
- The delegating harness and its provider adapters remain supported as the
  external-harness mode; this is an addition, not a removal.
