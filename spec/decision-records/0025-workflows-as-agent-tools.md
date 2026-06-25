# DR-0025: Workflows as agent tools — the convergence invariant

Status: accepted 2026-06-25 (design). Builds on the owned brokered harness
([DR-0024](0024-owned-brokered-agent-harness.md)): it extends the slice-4
capability-facade mechanism to `workflow.invoke`, and it **amends DR-0024 slice 2**
(the workspace lease becomes re-entrant within an invoke subtree). A follow-on
implementation slice; the formal convergence model is the gate before code.

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

2. Every invokable sub-workflow is self-terminating. It must pass workflow/flow
   liveness (reaches complete/fail) WITHOUT the @service escape, AND without
   dependence on external input that may never arrive: no signal consumption
   (`when message from` / injected signals), and any `human.ask` / `timer` wait
   must carry a terminating timeout/fallback. Reuses the existing liveness
   analysis (flow-liveness.maude) plus a signal-freedom check.
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
