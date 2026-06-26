# DR-0025: Workflows as agent tools — the convergence invariant

Status: accepted 2026-06-25 (design); v1 implemented 2026-06-25. Builds on the
owned brokered harness ([DR-0024](0024-owned-brokered-agent-harness.md)): it
extends the slice-4 capability-facade mechanism to `workflow.invoke`, and it
**amends DR-0024 slice 2** (the workspace lease becomes re-entrant within an
invoke subtree). The formal convergence model
(`models/maude/subworkflow-convergence.maude`) and the re-entrant-lease model
(`models/maude/subworkflow-lease.maude`) gate the implementation.

## v1 implementation status

Landed (model-gated): the `@tool` tag + per-workflow convergence check
(terminates, no `@service`, no external-signal/`@external`/inbound-message
readiness, no `human.ask`, and — conservatively for v1 — must be a leaf with no
nested `invoke`); synchronous `workflow.invoke` from inside a brokered turn
(`drive_subworkflow_tool`), exposing each `@tool` workflow as a typed agent tool
whose schema is its `input` contract; and the re-entrant workspace lease keyed on
the work-unit root. Curation is the per-agent **`tools [Foo, Bar]` grant** (the
in-program surface, checked at `whip check`), with `WHIPPLESCRIPT_HARNESS_TOOLS`
as an operator override; granted names resolve against the same program bundle or
a `use`d package (cross-package attestation below). Deferred: nested `@tool`
composition with the acyclic invoke-graph check (still under design).

## Problem

The owned harness lets an agent compose *primitive* tools (read/write/edit/bash,
DR-0024) and, via slice 4, *governed capabilities* (the work tracker). The natural
next rung is composing whole *sub-orchestrations*: letting the model, mid-turn,
run a predetermined whip workflow as a typed tool.

There are two ways to give an agent that power:

```text
skill via bash            owned workflow-as-tool
(model runs `whip run X`) (brokered workflow.invoke)
------------------------  -----------------------------------------
a side process; result    one durable workflow.invoke effect:
parsed from stdout        replayable, idempotency-keyed
no parent/child lineage   first-class parent<->child instance lineage
CLI args + text parsing   typed tool schema from the workflow contract
widens the bash allow-    per-workflow capability gate
list (coarse, bash risk)
separate store/runtime    shares the runtime: leases/counters/tracker compose
opaque                    events/facts/effects traceable + crash-recoverable
```

Shelling out **discards every property whip exists to provide** — it is the
delegating mindset. Brokering the invocation through the runtime keeps them all,
and turns the agent's tool surface into a **library of typed, durable, governed,
liveness-checked operations** it composes with judgment. That is the same
brokering-not-delegating choice as the harness itself, applied to sub-orchestration,
and no standalone agent harness can offer tools that are themselves verified
durable workflows.

But letting a *loop* invoke *workflows that may contain loops* raises the real
risk: **non-termination**. A single agent loop terminates (model-driven, step-
bounded). If each loop can spawn more loops without bound — by depth, or by
invoking a non-terminating sub-workflow — the system never converges.

## Decision

Expose **curated, predetermined** workflows as typed agent tools, as the
`workflow.invoke` capability projected through the same facade mechanism slice 4
uses for the tracker (mark a workflow agent-callable -> a tool whose schema is its
declared input/output contract, gated per-capability). Invocation is
**synchronous**: the tool call blocks the turn until the sub-workflow reaches its
terminal.

Synchronous blocking is safe because the invokable set is held to a **convergence
invariant** that makes every block *bounded*. The axis that matters is not
short-vs-long or sync-vs-async; it is convergence. A turn may block for a long
time, but it provably will not block forever.

### The convergence invariant (two static checks)

```text
1. Acyclic invoke-tool graph. The set of invokable workflows is finite and
   predetermined; the workflow-invoke-tool graph must be acyclic. A finite
   acyclic graph has bounded depth (its longest path) -- so this IS the recursion
   bound; no numeric depth limit is needed. Checked by reachability over the
   invoke graph, exactly like the existing graph.unbounded_pattern_recursion
   rejection (pattern-recursion.maude).

2. Every invokable sub-workflow is provably convergent. It must pass workflow/flow
   liveness (reaches complete/fail) WITHOUT the @service escape; its only permitted
   waits are ones the convergence model can prove terminate (a bounded timer with a
   terminating on-timeout path); it may not consume external input that may never
   arrive (no `when message from` / injected signal/event readiness); and it may
   not use `human.ask` (a human gate blocks the synchronous parent unboundedly, so
   human gates stay at the root). The admissible class is DEFINED BY the
   convergence model, not by enumeration -- signal-freedom and the human.ask ban
   are consequences of "the model cannot prove termination otherwise," not
   hand-picked rules (see "Declaration and checking"). Reuses the existing liveness
   analysis (flow-liveness.maude) plus a model-derived readiness check.
```

Depth-bound alone is insufficient (one `@service` or signal-blocked node hangs at
finite depth); per-node convergence alone is insufficient (cycles). Together they
guarantee: **the whole invoke tree below the root provably converges.**

### Signals are a root-only privilege

The principle the convergence invariant rests on:

> Intentional non-termination — waiting on external signals/events — is a
> privilege of the **root** workflow only. The root is the `@service` loop that
> idles on signals and dispatches work; everything it spawns is a convergent
> function of its inputs.

So an invokable sub-workflow cannot receive arbitrary signals. The whole running
system is one tree rooted at a single workflow: the root may loop forever by
design (top-level signal handling); every node below it terminates. Runaway
recursion / unbounded spawn becomes *structurally impossible*, not merely
discouraged — a system-wide progress property no standalone harness can state.

### Re-entrant workspace lease (amends DR-0024 slice 2)

DR-0024 slice 2 holds the workspace lease **per turn, exclusively, keyed by the
workspace path**. Synchronous workflow-tools break that: if a parent turn holds
the lease and blocks on a sub-workflow whose own turn needs the same workspace ->
**self-deadlock** (the child waits for a lease the blocked parent holds).

The lease therefore becomes **re-entrant within an invoke subtree**: the holder is
the **root of the unit of work** (the top-level invocation), and descendants share
it. The lease still excludes *other* root-level work from the workspace (the
coordination it exists for), but a parent and its own sub-workflows/sub-turns
never contend. This:

- makes synchronous blocking deadlock-free as well as bounded;
- keys the lease on the work-unit, not the turn — which also removes the
  parallel-contention sharp edge in the slice-2 implementation (every owned turn
  currently keys on cwd);
- composes with the deferred atomic-turn-isolation hook: a sub-workflow that must
  not see the parent's uncommitted workspace runs in a branched worktree instead
  of sharing — same subtree boundary, different isolation policy.

This supersedes the slice-2 per-turn lease keying; slice 2's implementation is
updated when this lands.

## Declaration and checking

The convergence invariant is only useful if there is a clear way to *declare* a
workflow as a tool and *enforce* the invariant. Declaration is two-sided:

- **The workflow opts in** with a `@tool` tag (parallel to `@service`) on
  `workflow Foo`. The tag means "eligible to be an agent tool, and subject to the
  convergence check"; Foo's declared `input`/`output` contracts become the tool's
  JSON schema (the slice-4 facade projection).
- **The agent is granted specific tool-workflows** — a per-agent allow-list (a
  dedicated `tools [RunTests, OpenPr]` field on the agent, or the existing
  `capabilities` gate). This is the curation: eligibility is the workflow's claim,
  access is the agent's grant.

Enforcement runs at `whip check` / compile in layers, each mirroring an analysis
that already exists:

```text
per-workflow (local)   A @tool workflow must be in the model-convergent class
                       (clause 2 above): liveness without @service, only
                       model-provably-terminating waits, no external-signal/event
                       readiness, no human.ask. Reuses flow-liveness + a readiness
                       check shaped like flow-namespace.
program (graph)        Build the invoke-tool graph (edge W -> V when an agent in W
                       is granted invoke V); reject cycles by reachability, exactly
                       like graph.unbounded_pattern_recursion.
transitive eligibility Acyclic ⇒ a well-founded bottom-up pass: a @tool workflow is
                       convergence-eligible iff it is locally convergent AND every
                       workflow it invokes-as-tool is eligible.
package boundary       A package exposing a @tool workflow attests in its contract:
                       convergence-eligible + its outgoing invoke edges, so a
                       consumer checks acyclicity/eligibility from the contract
                       without the source (see "Attestation format", implemented).
```

Failures are ordinary diagnostics: *"workflow X is `@tool` but is `@service` /
reads signal Y / uses `human.ask`"*, *"invoke-tool cycle X -> Y -> X"*, *"agent A
is granted Z, which is not `@tool`"*.

### Attestation format (implemented)

A package exports a `@tool` workflow by **shipping its source** and declaring it
in the manifest:

```json
"workflow_tools": [ { "name": "EchoText", "source": "tools/echo-text.whip" } ]
```

When the manifest loads — on the producer (`whip package`) **and** the consumer
(`use`) side — each entry's source is compiled (with `root = name`) and
convergence-checked, so a non-`@tool`/non-convergent export fails manifest
validation. From that the `package_contract_v0` artifact derives a `workflow_tools`
**attestation** per exported tool: `{ name, package_id, convergence_eligible:
true, input_schema, output_schema, invokes: [] }`. `input`/`output` become the
typed tool schema; `invokes` is reserved (empty for v1 leaf tools) for the
deferred nested-composition acyclicity check. A consumer's `tools [...]` grant
resolves against the same bundle first, then a `use`d package's exported tools;
at runtime the granted tool is driven from the package's shipped source through
the same `drive_subworkflow_tool` facade, with full parent↔child lineage. Modeled
in `subworkflow-attestation.maude` (the trust boundary: an accepted cross-package
grant is always backed by a real convergence attestation).

### The admissible class is model-defined, not enumerated

What counts as "convergent" is **defined by the convergence model, and the static
check is that model's decision procedure** — not a hand-curated list of banned
constructs. A workflow is `@tool`-eligible iff the model proves its turn-tree
converges. Signal-freedom and the human.ask ban fall out of this: a workflow that
waits on an external signal or a human cannot be shown to terminate, so the model
rejects it — not because someone declared signals forbidden, but because
convergence is unprovable with them. This keeps the check **sound by construction**
(it admits only model-convergent workflows) and lets the admissible class *grow*
as the model strengthens (e.g. if the model later proves a particular bounded
wait converges), always model-justified rather than opinion-driven. v1 is
deliberately conservative: admit only what is straightforwardly provable.

## Refined I3 line

Calling a **named workflow from a curated allow-list with typed inputs** is a
governed capability call (allowed — a `capability.call`-shaped effect, shared-state
participation per the DR-0024 refined I3). Letting the model **author or freely
compose orchestration** — pass arbitrary workflow source, choose topology, invoke
anything — is the model writing control flow (banned). The invokable set is a
per-agent allow-list of *declared* workflows; never "invoke anything / here is
some source."

## Why this is the emergent-composition complement to rule-driven invoke

A *rule* invokes a sub-workflow when the orchestration statically knows it should
(known coordination). The *model* invokes a workflow-tool when it discovers
mid-task that it needs one (emergent) — the same complementarity as agent-filed vs
rule-filed to-dos (DR-0024 slice 4). Both legitimate; different timing.

## Cost (accepted)

Invokable-as-tool workflows are a **restricted subset**: convergent, `@service`-
free, signal-free, bounded-wait. Not every workflow qualifies as a tool. That is
the right price, and it draws a sharp line: **convergent computation is what
sub-workflow-tools do; external-event-driven coordination stays at the root** —
whip's existing leaf/orchestration layering, enforced one level down.

## What this record does not decide (deferred hooks)

```text
async / handle variant   A tool that returns a child handle and lets the
                         ORCHESTRATION (not the model) await a long/human-gated
                         child. Reopens the I3 line and lease/blocking semantics;
                         deferred until a concrete need. Sync + convergent is v1.
numeric-bounded recursion  If self/mutual recursion is ever wanted, an explicit
                         numeric depth bound is the escape from strict acyclicity.
                         Deferred; acyclic-only for v1.
worktree sub-isolation   Running a sub-workflow in a branched worktree rather than
                         sharing the parent workspace (ties to the DR-0024 atomic-
                         turn-isolation hook).
attestation format       IMPLEMENTED (see "Attestation format" above): the
                         package_contract_v0 `workflow_tools` section attests
                         eligibility + tool schema + (reserved) outgoing invoke
                         edges; the manifest ships the tool source. The `invokes`
                         edges stay empty until nested @tool composition lands.
```

## Formal model plan (gate before code)

A Maude reachability model over the invoke-tool graph, same family as
`pattern-recursion.maude` / `flow-namespace.maude`:

- coverage: an acyclic graph of convergent sub-workflows is accepted;
- bite: a cycle in the invoke graph is rejected (`No solution` for a converged
  tree once a back-edge exists); a signal-consuming or `@service` node reachable as
  an invokable sub-workflow is rejected. Each bite carries the usual residual soup
  variable.

Plus the re-entrant-lease change is reflected in the brokered-turn lifecycle
(TLA/Maude) so a parent blocking on a same-workspace child cannot deadlock.

## Consequences

- The agent's tool surface can include curated, typed, durable, governed,
  liveness-checked sub-workflows — whip's value proposition extended into the loop.
- The whole agent system is a provably-convergent tree with exactly one intentional
  non-termination point (the root `@service` signal loop).
- Signals / unbounded external waits are root-only; sub-workflow-tools are
  convergent functions of their inputs.
- The workspace lease is scoped to the invoke subtree (work-unit), not the turn;
  DR-0024 slice 2 is amended accordingly.
- Skill-via-bash remains the explicit long-tail escape hatch for genuinely
  external, throwaway invocations where lineage does not matter.
