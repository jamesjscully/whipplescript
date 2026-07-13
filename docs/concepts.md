# Concepts

WhippleScript separates orchestration policy from execution. This page
defines the handful of terms the rest of the documentation builds on.

## The execution model

```text
facts/events + rules  ->  durable facts and effects
effects + workers     ->  provider runs
provider results      ->  events and facts
workflow terminals    ->  completed or failed instances
```

Rules are deterministic: given the same facts, they commit the same changes.
Everything external — an agent turn, a model decision, a human approval — is
an effect, recorded durably before it runs and resolved by events after.
That split is what makes a workflow steppable, resumable, and auditable.

The same rules-and-effects kernel runs locally and, unchanged, inside a
Cloudflare Durable Object on the edge: `whip deploy` ships a workflow to a
Worker plus Durable Object, and `whip checkpoint` / `whip restore` rewind an
instance's file state, agent transcript, and event-log position together as
one coherence-checked cut. See [Runtime & operations](runtime-operations.md)
for the cloud runtime and the operator commands.

## Workflow

The durable boundary for a unit of work. Starting a workflow creates an
*instance* with its own event log, facts, effects, provider runs, evidence,
and lifecycle state (`running`, `paused`, `completed`, `failed`,
`cancelled`).

## Fact

Typed workflow state. Rules match facts, create facts, and consume facts as
work moves forward. Queued tasks, reviewed results, approvals, and failure
records are all facts.

## Event

An append-only record of something that happened: `external.started`,
`rule.committed`, `effect.terminal`, `human.answer.received`,
`workflow.completed`. The event log is the source of truth; facts, effects,
status, and traces are projections over it.

## Rule

Deterministic policy. A rule names the facts and events it is waiting for,
optionally filters them with a pure guard, and commits a rewrite — new facts,
consumed facts, new effects, or a workflow terminal — atomically:

```whip
rule dispatch
  when WorkItem as item where item.status == "queued"
  when worker is available
=> {
  tell worker as turn "Do {{ item.title }}"
}
```

Rules never perform I/O. If a decision needs a model or external data, the
rule creates an effect and a later rule reacts to its completion.

## Flow

A sequential surface over rules. A flow reads top to bottom — do this step,
then that one, then branch — and lowers to ordinary rules plus a generated
await state. It adds no new runtime concept: the kernel still sees rules,
facts, and effects, so a flow stays steppable and auditable like everything
else. Use it when work is genuinely a fixed sequence; use plain rules when
reactions are independent.

## Effect

A durable request for external work. `tell` (agent turn), `coerce`/`decide`
(typed model decision), `askHuman` (review request), `call` (package
capability), `exec` (dev raw command or hosted pinned script capability),
`timer` (a delay), the tracker verbs (`file`/`claim`/`release`/`finish`), and
`invoke` (child workflow) all create effects. So do the event verbs `emit` (an
event) and `emit signal ... to` (a signal to another instance), `send via`
(channel messaging), and the coordination verbs `acquire`/`release` (leases),
`consume` (a counter budget), and `append` (a ledger entry). An effect records
what was requested, which provider ran it, whether it finished, and what
evidence was captured.

## Work queue

A durable backlog of work items, declared in source and vendor-neutral. Where
a fact is workflow state, a queue item is a unit of pending work to be
claimed, worked, and finished — the `builtin` tracker persists it outside the
event log so a backlog survives across instances. Rules pull from a queue with
`claim`; a lost claim is an ordinary branchable failure, so contention needs no
locks in source.

## Agent

A logical target declared in source:

```whip
agent triager {
  provider fixture
  profile "repo-reader"
  capacity 1
}
```

Source names the agent, its provider family, authority profile, and
concurrency capacity. Runtime configuration supplies credentials and
execution details — never the source file.

## Provider

The thing that executes effects. The *fixture provider* is deterministic and
local: it completes effects with synthetic results, which makes it the right
default for development, tutorials, and tests. The *owned harness* runs the
agent tool-use loop inside whip itself — whip is the executor. The delegating
native providers (Codex and Claude) instead hand the whole
turn to the provider's own harness. See [providers & packages](providers.md).

## Worker

The loop that claims ready effects, runs them through a provider under a
lease, and records completions. Workers execute what rules already decided;
they hold no policy of their own.

## The four runtime commands

| Command | Does | Does not |
| --- | --- | --- |
| `run` | Start an instance and record the start event. | Evaluate rules or providers. |
| `step` | Evaluate rules and commit facts/effects. | Execute providers. |
| `worker` | Execute ready effects through a provider. | Decide policy. |
| `dev` | Compose all three in a loop, then evaluate assertions. | — |

Use `dev` day to day. Use the separate commands when you want to observe one
boundary at a time.

## Skills and packages

A *skill* is a context bundle attached to an agent or a turn — it shapes what
the agent knows, not what the language means. A *package* can expose library
surface through `use` and register capabilities, providers, schemas, and
resources through its manifest; its capabilities are called as explicit effects,
never as hidden control flow.
