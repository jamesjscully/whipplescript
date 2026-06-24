# 0002: Tracker Package

Status: proposed

## Decision

The record of work to be orchestrated is the issue tracker. `std.tracker`
provides that issue tracker as a standard package: a durable local provider for
agent/human coordination, plus a portable semantic surface that GitHub, Linear,
Jira, and other providers can approximate.

Ready queues are projections over issue records. The queue is not the source of
truth.

The tracker state model is append-only accepted changes plus projections:

```text
commands request changes
events record accepted changes
projections answer questions
```

Workflow source, CLI commands, and provider APIs should not mutate "the ready
queue" directly. They submit commands that append accepted issue, relation,
comment, evidence, or claim events. Current issue state, ready-work lists,
history, summaries, conflicts, sync state, and active-claim views are derived
projections.

## Construct Graph Contract

`std.tracker` should be specified first as a typed construct graph surface, then
as provider storage and CLI behavior. The package must contribute construct
instances and interfaces that the checker can normalize into
`whipplescript.construct_graph.v0`.

Minimum source-facing constructs:

```text
tracker <name> { ... }
when <tracker> has ready issue as <binding>
claim <issue> as <claim_handle>
renew <claim_handle>
release <claim_handle>
note <issue>
attach evidence to <issue>
set issue field
finish / close / cancel / reopen issue
```

Graph meaning:

```text
tracker declaration
  family: declaration_block
  provides: Resource<tracker-name, Tracker<Issue>>
  may provide: Projection<tracker-name, Issue> for ready issue
  lowering class: metadata today; future resource/event-source classes only if
    platform-owned and modeled

ready issue projection
  family: rule_projection
  requires: Resource<tracker-name, Tracker<Issue>>
  requires: Projection<tracker-name, Issue>
  provides: Value<Issue> in the rule condition
  lowering class: future projection_view, or checker-owned projection-read
    metadata until projection_view is implemented

tracker operations
  family: effect_operation or resource_operation
  requires: Resource<tracker-name, Tracker<Issue>> when a source tracker is named
  requires: Value<Issue> / Value<IssueRef> / Value<Lease> as appropriate
  requires: Capability<tracker.*>
  provides: EffectHandle<TrackerResult> and, after success, Value<TrackerResult>
  lowering class: future resource_effect or typed_effect_call; generic
    capability_call is acceptable only for operations whose behavior is plain
    request/response
```

Resource identity is part of the contract. Two tracker resources may both expose
`Projection<Issue>`, but a source use of `github_backlog` cannot be satisfied by
`linear_backlog`. Bare tracker/projection forms are valid only when import and
scope resolution leave exactly one compatible resource; otherwise the checker
must diagnose ambiguity instead of choosing.

Lowering must emit only ordinary core objects:

```text
effect graph templates for mutations and lifecycle operations
projection-read metadata or projection records for ready queries
event-source admission templates for external tracker sync, if added later
effect dependency templates for after-branch composition
```

It must not materialize leases, provider runs, terminal outcomes, conflicts, or
ready facts during parse/check/lowering. Those are runtime/provider state
derived through the ordinary event/effect lifecycle and validated outputs.

Capabilities should be narrow and namespaced:

```text
tracker.ready
tracker.file
tracker.claim
tracker.renew
tracker.release
tracker.note
tracker.evidence
tracker.field.write
tracker.relation.write
tracker.finish
tracker.cancel
tracker.reopen
tracker.sync
```

The implementation can start with a smaller subset, but hidden capability
requirements are forbidden. If `claim` also writes a durable status field via a
convenience command such as `claim --mark in_progress`, that combined operation
must expose both authority requirements and partial-failure behavior in the
effect contract.

## Loft Comparison

Loft has the right shape for the builtin provider:

```text
authoritative immutable transactions
disposable relational projection
command-mediated mutation
semantic conflicts
local execution leases
direct CLI and local API
schema/capability discovery
export/import and repair tools
```

`std.tracker` should adopt those product lessons without preserving Loft as a
separate language concept. Loft-specific names become provider details or
implementation inspiration, not compatibility obligations.

The most important Loft distinction is storage authority:

```text
durable issue history: committed transaction/event records
query state: disposable projection
execution claims: local runtime leases
ready queues: projection queries over the issue model
```

This gives agents a durable shared backlog, lets workflows consume ready work
deterministically, and keeps local execution claims out of portable project
history.

## Portable Issue Model

The portable issue shape should stay small but rich enough for real
orchestration:

```text
id
alias
external_id / url
title
body
status
type
labels
owner / assignee
priority
created_at / updated_at
closed_at / close_reason
defer_until / due_at
relations / blocked_by
comments / notes
evidence
resource_intent
metadata
heads
state_token / version
conflicted / field_conflicts
```

`id` is the canonical opaque issue identifier. The local provider may also
maintain a human-speakable `alias` such as `WS-7` for CLI and prompt ergonomics,
but aliases are not transaction identity and must not be used for causal
parents, hashes, or import/export identity. This preserves Loft-style
merge-friendly IDs without losing the ability to tell an agent "take WS-7".

`metadata` is where provider-specific richness lives. Fields should become
portable only when workflows need them frequently and external providers can map
them honestly.

The status category should be small:

```text
open
in_progress
closed
canceled
archived
```

There is no `blocked` status. Blocking is derived from relations, provider
readiness, deferral, unresolved conflicts, and active leases.

## Local Provider

The builtin provider should be an event-sourced local tracker, not a markdown
task list and not the workflow run store.

Its authoritative state should be committed project history:

```text
.whip/tracker/config.toml
.whip/tracker/tx/**/*.json
```

Its projection and runtime state should be rebuildable or local-only:

```text
.whip/tracker/index.sqlite      # disposable projection
.whip/tracker/runtime.sqlite    # local leases, heartbeats, sessions
.whip/tracker/locks/
.whip/tracker/tmp/
```

The exact path can change, but the boundary should not: transaction records are
portable project history; projections and runtime leases are not.

The provider should record mutations as immutable transactions containing one
or more events:

```text
issue.created
issue.field_set
issue.closed
issue.canceled
issue.reopened
issue.resource_intent_set
label.added
label.removed
relation.added
relation.removed
comment.added
evidence.added
claim.acquired
claim.renewed
claim.released
claim.expired
claim.failed
```

Current issue state, ready-work lists, history, summaries, conflicts, and change
feeds are projections.

The portable issue log and the local coordination log are related but distinct.
Issue events are portable project history. Claim events may be local-only when
they represent runtime coordination, but they should still be append-only within
the provider so readiness, conflict repair, and audit output can be rebuilt from
events rather than mutable rows.

External providers do not need to be event-sourced internally. GitHub, Linear,
Jira, and similar adapters should normalize observed mutable state into tracker
events with source tokens, sync cursors, and conflict metadata. The adapter must
not claim stronger semantics than the provider can support; weak or advisory
external claims should surface as such in readiness and conflict projections.

## Readiness

Readiness is the provider's promise. For the builtin provider, an issue is
ready when it is active, not deferred, not conflicted, not blocked by active
blockers, and not under an active local lease.

The language should keep the concise dispatch shape:

```whip
tracker backlog {
  provider builtin
}

rule implement_ready_issue
  when backlog has ready issue as issue
  when coder is available
{
  claim issue as active_claim

  after active_claim succeeds {
    tell coder with issue
  }
}
```

`when backlog has ready issue` lowers to a tracker query. It should not
duplicate readiness logic in the workflow runtime.

Readiness is a projection over accepted tracker events plus provider sync
observations. It is not a queue table that workflows can mutate.

There is no separate queue compatibility surface. The source language should
prefer `tracker`, `ready issue`, and issue verbs directly. Current queue/item
implementation details may inform the replacement, but they should not become
aliases that the package contract must preserve.

## Source Operations

The package should expose durable workflow effects for the lifecycle agents
need:

```text
file issue into tracker
claim issue as active_claim
renew active_claim
release active_claim
note issue
attach evidence to issue
set issue field
add / remove label
link / unlink relation
finish issue / close issue
cancel issue
reopen issue
```

The first implementation should prioritize:

```text
file
ready
claim
renew
release
finish
fail with note
note
evidence
resource_intent
dependency/blocker relations
```

`claim` is the hard semantic operation. It must be branchable: no ready issue,
already leased, stale state, actor mismatch, and lease mismatch are normal typed
failures, not crashes.

`claim` should create a local execution lease. It should not implicitly mutate
durable issue status unless the source or CLI asks for that behavior, such as
`claim --mark in_progress`. This differs from the current builtin queue store,
where claim also writes `status = in_progress`; the new provider should keep the
lease/status split.

The source-facing handle should be a tracker claim handle, not a generic lease
value. `std.coord` may expose generic leases as its own package, and the builtin
tracker provider may implement claims with the same lower-level lease machinery,
but importing `std.tracker` should not make arbitrary coordination leases part
of the issue API. External tracker providers must declare claim strength such as
strong, best-effort, advisory, or unsupported so readiness projections can expose
uncertainty instead of pretending every integration has local atomic leases.

## CLI Requirements

The CLI is part of the product surface. Humans and agents must be able to edit
issues directly; they should not need a workflow just to inspect, claim, note,
close, or repair tracker state.

The command names can settle during implementation, but the required shape is:

```bash
whip issue new "Fix retry policy"
whip issue list --json
whip issue show ISSUE --json
whip issue ready --json --limit 5
whip issue set ISSUE priority 1 --expect-state-token TOKEN
whip issue label add ISSUE infra
whip issue dep add BLOCKED depends-on BLOCKER --kind resource
whip issue note ISSUE "observed failure" --lease-id LEASE
whip issue evidence add ISSUE --lease-id LEASE --kind whip.trace --artifact REF
whip issue claim --ready --actor agent:a --ttl 30m --mark in_progress --json
whip issue renew LEASE --ttl 30m --json
whip issue release LEASE --json
whip issue complete LEASE --reason done --json
whip issue fail LEASE --note "tests failed" --release --json
whip issue close ISSUE --reason done --json
whip issue cancel ISSUE --reason obsolete --json
whip issue reopen ISSUE --reason "regression reproduced" --json
whip issue log ISSUE --json
whip issue conflicts --json
whip issue resolve ISSUE field --value VALUE --json
whip issue changes --since TOKEN --json
whip issue watch --json
whip issue export --format jsonl
whip issue import tracker-events.jsonl
whip issue rebuild
whip issue doctor --json
whip issue capabilities --json
whip issue schema --json
```

Every agent-facing JSON command that returns an issue should return the rich
issue shape. Agents should not need to immediately call `show` after `claim`,
`note`, `evidence`, `set`, or `complete`.

## Local API

The local provider should also expose a local API over the same command layer.
This is for harnesses, editor integrations, daemons, and long-running workers;
it is not a separate source of truth.

Minimum route shape:

```text
GET  /version
GET  /capabilities
GET  /schema
GET  /issues
GET  /issues/:id
GET  /ready
GET  /conflicts
GET  /changes
POST /commands
POST /leases/claim
POST /leases/:id/renew
POST /leases/:id/release
```

CLI JSON and API JSON should share success/error envelopes, issue schemas, and
typed recoverable errors.

## Leases And Lifecycle

Agent claims are local execution leases, not durable assignment history. A lease
coordinates who is currently working; durable ownership remains an issue field.

Lease rules:

- Expired or released leases do not block readiness.
- A plain lease does not have to change issue status.
- Agent lifecycle commands may combine a lease mutation with a durable issue
  mutation, but partial-failure responses must be explicit.
- Mutating commands may require `--lease-id` or `--require-lease`.
- Agents should pass state tokens when mutating issue state they previously
  observed.

The common lifecycle should be convenient:

```bash
claim --ready --mark in_progress
complete LEASE --reason done
fail LEASE --note "tests failed" --release
```

If a durable transaction commits but a runtime lease release fails, the response
must say so and include the committed issue state. If a durable mutation fails,
the command must not silently release the lease.

Combined commands such as `claim --mark in_progress` append multiple accepted
events in one command transaction, for example `claim.acquired` and
`issue.field_set(status=in_progress)`. Plain `claim` appends only the claim
event and changes readiness through projection overlay, not durable issue
status.

The current `queue.claim` implementation mutates issue status and claim holder
in the same row. The tracker replacement should split that state:

```text
durable transaction: issue.created / issue.field_set / issue.closed / ...
local runtime lease: lease_id, issue_id, actor, expires_at, released_at
projection: issue_status plus active lease overlay for readiness
```

New commands should not add durable claim fields. If developer-local data needs
rescue during the transition, that should be an explicit import/repair tool, not
a language or provider compatibility promise.

## Relations And Dependencies

Relations are directed edges. The builtin provider should support at least:

```text
blocks
parent-of
related
duplicates
supersedes
discovered-from
```

`blocks` affects readiness. Other relations are graph metadata unless provider
policy says otherwise.

Dependency sugar should compile to `blocks` relations:

```bash
whip issue dep add B depends-on A --kind hard
```

Dependency kinds should be small and operational:

```text
hard
soft
order
resource
review
contract
discovered
```

The tracker records dependency metadata. Domain-specific packages or providers
may decide whether resource intent implies new dependencies or write-governance
conflicts.

## Conflicts

The local provider should make semantic conflicts explicit during projection
rebuild. It should not silently pick one concurrent scalar write and pretend
there is no conflict.

Conflicted issues are not ready. `show`, `list`, `ready`, and every rich issue
JSON shape should expose:

```text
conflicted
field_conflicts
state_token
heads
```

Conflict resolution should be a command that appends a new resolving event whose
parents include the conflicting heads.

## Discovery And Guarantees

Providers should expose enough machine-readable discovery for harnesses and
agents:

```text
version
capabilities
schema
fixture family / contract version
supported commands
supported event types
supported features
```

The builtin provider should maintain conformance fixtures for claim, already
leased, note, field transition, evidence, resource intent, renew, release,
complete, fail, lease preconditions, stale state, and partial lifecycle failure.

External providers may not support every command atomically. The binding must
declare weaker guarantees rather than hiding them.

## Replacement Of Current Queue Store

The current builtin work store already has the right architectural boundary: a
workspace-scoped backlog store separate from disposable workflow run stores,
with ready items projected into instance facts. The new provider should keep
that boundary while replacing the row-as-source-of-truth store with
transaction-as-source-of-truth plus a disposable projection.

No backward-compatible migration plan is required. The implementation can move
directly to the tracker provider and `whip issue` CLI, remove old queue syntax
from examples, and stop projecting queue-shaped facts. An optional importer from
`.whipplescript/items.sqlite` is acceptable as an operator repair tool, but it
is not part of the tracker package contract.

## Fit With Existing Runtime

The refined tracker design fits the existing control plane because tracker
verbs are ordinary durable effects, ready work is projected into facts, and
agent turns can still be gated by `after claim succeeds`.

The runtime should not gain special tracker semantics. The provider owns
readiness, leases, conflicts, import/export, and repair. The workflow runtime
continues to own effect lifecycle, `after` branching, evidence links, and
instance-local derived facts.

## External Providers

GitHub, Linear, and Jira integrations should map to the same semantic surface:

```text
open / in_progress / closed / canceled
ready query
claim
comment
close / cancel / reopen
labels
relations or links where available
state/version token where available
```

Exact parity is impossible. The portable contract is "honest approximation with
declared guarantees," not forced sameness.

Examples:

- GitHub claim may be check-then-assign and therefore has a documented race
  window.
- Jira transition names are binding configuration, not language syntax.
- Linear states map through state categories, not through hardcoded state names.

## Rationale

A queue-only item model is too thin for serious orchestration. Agents need to
leave audit trails, attach structured evidence, decompose work, block on
dependencies, resume after failures, and coordinate over a shared durable
backlog.

Markdown task lists cannot provide deterministic readiness, lease preconditions,
conflict detection, or durable lifecycle. A SaaS-shaped tracker model is also
too heavy. The right kernel is small issue records plus command-mediated
mutation over durable history.

At the same time, WhippleScript should not become a full project-management
application. Boards, sprints, time tracking, dashboards, and rich UI workflows
are outside `std.tracker` unless they become necessary for orchestration.

## Consequences

- The existing builtin queue tracker should be replaced by the local
  `std.tracker` provider.
- `ready item` and queue verbs should not be retained as package-level aliases.
- Direct CLI operations must cover the lifecycle the language can express.
- The local provider should not store source-of-truth issues in the workflow run
  store.
- The local provider should not keep `.whipplescript/items.sqlite` as the
  authoritative tracker.
- Resource intent is issue metadata. Packages and providers may consume it, but
  the tracker owns issue state.
- A separate evidence package is not needed for now; issue evidence is part of
  tracker audit history, while artifacts remain a core substrate.

## Risks

- A richer issue model could make the first implementation too large.
- External trackers vary widely; exact parity is impossible.
- Event-sourced local work tracking adds migration and projection complexity.
- Atomic lifecycle commands cross durable issue state and local runtime lease
  state, so partial-failure contracts must be designed carefully.
- Semantic conflict handling is valuable but expensive to implement correctly.

## Open Questions

- Which fields deserve first-class syntax versus opaque metadata?
- Should v1 include full relation removal and conflict resolution, or only the
  subset needed to claim and complete ready work?
- Should `whip tracker` exist as an expert/admin alias, or is `whip issue`
  enough?
- Should the builtin provider use `.whip/tracker` or another path?
