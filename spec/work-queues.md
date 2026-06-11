# Work queues: vendor-neutral issue tracking

Status: spec drafted 2026-06-09 (decided in design discussion; see
[`language-ergonomics-tracker.md`](language-ergonomics-tracker.md) A3).
Stage: spec -> modeling -> implementation + testing -> review.

## Motivation

Agent work needs a durable system of record for *what is to be done*.
Harness-native planning artifacts are ephemeral and non-standard; markdown
backlogs decay into stale-note dumps because markdown has no semantics a
machine can act on; relying on LLM reasoning to operate a tracker wastes
intelligence on scheduling decisions that should be deterministic.

The dispatch loop — *take the next ready item the moment capacity frees* —
is WhippleScript's core competency. This spec gives that loop a standard,
vendor-neutral vocabulary, replacing the Loft tokens currently baked into
the parser. The design mirrors the agent/provider split the language already
has: source declares a logical queue; configuration binds it to a tracker.

```text
agent  : provider  =  queue : tracker
turn   : agent     =  item  : queue
```

## Source surface

### Declarations

```whip
queue backlog {
  tracker builtin        // or: loft, github, linear, jira (bindings)
}
```

`queue` declares a logical source of work items. `tracker` names the binding
kind; binding-specific configuration (repo, project, JQL filter, credential
references) lives in runtime config keyed by queue name, exactly like
provider configs — never in source.

### Readiness

```whip
flow work_backlog
  when backlog has ready item as item
  when worker is available
{ ... }
```

`when <queue> has ready item as x` is blessed sugar over the general fact
match (`when fact queue.item.ready as x where x.queue == "backlog"` —
exact lowering shape settled at implementation). **Readiness is the
tracker's promise**: the core never computes "ready" from dependency graphs,
sprints, or relations; it asks the binding. For the builtin tracker, ready =
`open` and unclaimed.

### Verbs (rule/flow body operations)

```whip
file item into backlog {
  title "Fix login crash"
  body "Repro: empty password on /login returns 500."
}

claim item as lease
release item
finish item {
  summary turn.summary
}
```

| Verb | Meaning | External mapping |
| --- | --- | --- |
| `file item into <queue> { ... }` | Create an item. | Create issue. |
| `claim item [as x]` | Acquire the item under a lease; sets `in_progress`. Atomic at the tracker. "Already claimed" is a normal, branchable failure. | Set assignee (+ transition where supported). |
| `release item` | Surrender the lease; item returns to `open`. | Unassign (+ transition back). |
| `finish item [{ summary ... }]` | Mark done. Optional payload posted as a closing comment — the agent-work audit trail lands in the tracker, where humans look. | Close/complete (+ comment). |

All four are durable effects (`queue.file`, `queue.claim`, `queue.release`,
`queue.finish`) with the standard effect lifecycle, evidence, and `after`
branching. Deferred to v1.5: `cancel item`, standalone `comment item` (both
map to all surveyed trackers; neither is needed to prove the loop).

## Core schema

```text
id        string     tracker-native, opaque to core, unique within queue
title     string
body      string
status    open | in_progress | done | cancelled
labels    string[]
metadata  map        opaque; tracker-specific richness lives here
```

The status set is the *category* layer every surveyed tracker shares.
`ready` is a derived predicate, not a status. `blocked` does not exist in
core — no surveyed tracker models it as a state; where a binding can derive
it, it surfaces as not-ready plus `metadata`.

Priority, estimates, sprints, relations: `metadata`, until real usage proves
a field universal.

## Tracker compatibility mapping

The interface must play "sort of nice, not perfect" with GitHub Issues,
Linear, and Jira:

| Core | GitHub | Linear | Jira |
| --- | --- | --- | --- |
| `open` | open | backlog / unstarted categories | To Do category |
| `in_progress` | open + assignee | started category | In Progress category |
| `done` | closed (completed) | completed category | Done category |
| `cancelled` | closed (not_planned) | canceled category | Done + won't-fix resolution |
| `claim` | check-then-assign (no CAS; documented race window) | set assignee | set assignee + transition |
| identity | `#123` per repo | `ABC-123` per team | `PROJ-123` per project |
| `ready` query | binding-configured (open, unassigned, label filter) | binding-configured | binding-configured JQL |
| `finish` summary | closing comment | comment | comment |

Bindings own the category -> native-transition mapping in their config
(Jira's arbitrary workflows are mapped there, never modeled in core).

## Claim and lease semantics

`claim` is the verb with real semantics and the hard kernel of the
interface:

- The **tracker is the arbiter**. The builtin enforces atomicity
  transactionally. External bindings do best-effort (check-then-assign);
  the residual race window is documented per binding.
- A successful claim creates a whip-side **lease** (expiry, renewal) using
  the existing lease machinery; lease expiry releases the item.
- `claim` on an already-claimed item completes on the failure branch with a
  typed reason — dispatch rules route around it without error handling
  ceremony.

## The builtin tracker

The reference implementation and default binding, backed by a
**workspace-scoped** SQLite file (default `.whipplescript/items.sqlite`),
deliberately *not* the run store:

- Run stores are disposable per experiment; the backlog is durable. Items in
  run stores would fragment the backlog across throwaway files.
- Symmetry: the builtin is *just another binding whose backend is a local
  file*. The run store never holds source-of-truth items for any tracker —
  only instance-local projections. One dataflow, no special case, and the
  projection/claim/round-trip paths are exercised by the default setup.

Builtin identity: sequential, human-speakable (`WS-1`, `WS-2`). Sequential
beats content hashes: "take WS-7" is speakable to an agent, and distinct IDs
for byte-identical items fall out for free.

A git-committable backend (items as JSONL/files) is a possible later binding
for teams who want the backlog in version control; the interface is
indifferent.

## Projection model

External tracker state reaches workflows by polling on worker passes
(consistent with timer firing — state advances when a worker runs):

```text
worker pass: query binding for ready items -> project as instance-local facts
verb effects: round-trip mutations through the binding (tracker stays truth)
```

Projected item facts are a cache keyed `(queue, id)`; the tracker is always
the source of truth. The builtin tracker uses the same path with a local
backend.

## Agent filing: two doors, one stamp

- **Workflows** file deterministically via the `file` effect.
- **Agents mid-turn** file through the CLI — `whip items add --queue
  backlog --title ... --body ...` — the one door every harness has (MCP
  surface later). `whip items`, `whip items show <id>` complete the
  inspection loop.
- The worker injects run identity into turn environments
  (`WHIPPLESCRIPT_RUN_ID`, instance id, effect id); the CLI stamps it as
  provenance on anything filed from inside a turn. An agent's decomposition
  lands in the tracker attributed to the exact turn that produced it.

Division of labor: agents decide *what* the work is; rules decide *when and
who*; the queue is where they meet. Items filed mid-turn become `ready`
during the run and are picked up by dispatch flows on the next worker pass —
the self-feeding loop (Ralph pattern) with a real backlog instead of a
prompt convention.

## Loft eviction

The parser loses every Loft token: the `loft has ready issue` pattern,
`claim issue with loft`, the `LoftIssue`/`LoftClaim` builtin schemas, and
loft effect kinds. Loft becomes a deferred tracker binding behind this
interface (not built now; the interface is shaped so it can bind, which is
sufficient). The builtin `AgentTurn` type drops the never-populated
`issue`/`changedFiles` fields; the examples referencing them are rewritten
honestly; turn enrichment becomes a documented capability a tracker binding
may provide later.

## Out of scope (v1)

- Loft/GitHub/Linear/Jira bindings (interface only; builtin is the proof).
- `cancel` / `comment` verbs (v1.5).
- Dependency/relation modeling, priorities, sprints (metadata).
- Webhook/push sync (polling on worker passes only).
- Cross-queue moves, bulk operations, item editing from source.

## Modeling notes (next stage)

- Claim atomicity and lease expiry belong in the TLA+ lifecycle model:
  two workers, one item, no double-claim; expiry releases exactly once.
- Projection staleness: a claimed-elsewhere item observed as ready locally
  must fail closed at claim time (the model should show the race resolves
  at the tracker, never in the projection).
- The `queue.*` effects reuse the standard effect lifecycle — existing
  Maude effect model should cover them with new kinds, not new semantics.
