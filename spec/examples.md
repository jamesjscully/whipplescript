# Examples

Status: sketchpad

For canonical terminology and the current language reference, start with
[`../docs/language-reference.md`](../docs/language-reference.md). This file
remains a design sketchpad for example shapes.

These examples are intentionally small. The goal is to feel the authoring model
before committing to syntax.

## Simple Queue Worker

This is the best current v0 candidate for the smallest useful real workflow:

```whipplescript
workflow SimpleQueueWorker

queue backlog {
  tracker builtin
}

agent worker {
  profile "repo-writer"
  capacity 1
}

rule start_ready_issue
  when {
    backlog has ready item as item
    worker is available
  }
=> {
  claim item as work

  after work succeeds {
    tell worker """markdown
    Implement this work item:

    {{ item.title }}

    {{ item.body }}

    Finish with a concise summary.
    """
  }

  after work fails as failure {
    askHuman """markdown
    WhippleScript could not claim this work item.

    Reason:
    {{ failure.reason }}

    Please inspect the queue state or retry later.
    """
  }
}
```

Why this script is important:

- `claim` and `tell` are not sequenced by source order.
- `after work succeeds` creates a durable dependency edge.
- The worker prompt can use claim output only inside the success scope.
- Claim failure is handled separately from worker failure.

## Queue Worker With Typed Review

This example adds typed schema coercion. It uses backend-compatible `enum`,
`class`, and `coerce` declarations, but keeps ordinary data operations small and
pure. coerce is a current backend, not the language concept.

```whipplescript
workflow QueueWorkerWithReview

queue backlog {
  tracker builtin
}

enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class WorkReview {
  status ReviewStatus
  reason string
  followups string[]
  confidence float
}

class ReviewedWork {
  itemId string
  review WorkReview
}

coerce reviewWork(itemTitle string, agentSummary string) -> WorkReview {
  prompt """markdown
  Review this completed agent turn for the work item.

  Item:
  {{ itemTitle }}

  Agent summary:
  {{ agentSummary }}

  Return a structured review.

  {{ ctx.output_format }}
  """
}

agent worker {
  profile "repo-writer"
  capacity 1
}

rule start_ready_issue
  when {
    backlog has ready item as item
    worker is available
  }
=> {
  claim item as work

  after work succeeds {
    tell worker """markdown
    Implement this work item:

    {{ item.title }}

    {{ item.body }}

    Finish with a concise summary.
    """
  }

  after turn succeeds as outcome {
    coerce reviewWork(item.title, outcome.summary) as review
  }

  after review succeeds as verdict {
    record ReviewedWork {
      itemId item.id
      review verdict
    }
  }

  after review fails {
    askHuman """markdown
    Typed review failed for this completed turn.

    Please inspect the turn artifacts and decide whether to accept, revise, or
    block the item.
    """
  }
}

rule accept_reviewed_work
  when ReviewedWork as reviewed
  when reviewed.review.status == Accept
=> {
  record AcceptedWork {
    itemId reviewed.itemId
    reason reviewed.review.reason
  }
}

rule request_revision
  when {
    ReviewedWork as reviewed
    reviewed.review.status == Revise
    worker is available
  }
=> {
  tell worker """markdown
  Revise this work item:

  Review reason:
  {{ reviewed.review.reason }}

  Follow-ups:
  {{ reviewed.review.followups }}
  """
}

rule escalate_blocked_review
  when ReviewedWork as reviewed
  when reviewed.review.status == Blocked
=> {
  askHuman """markdown
  The model review says this work item is blocked:

  Reason:
  {{ reviewed.review.reason }}
  """
}
```

What transfers from the earlier expression design:

- Schema-coercion-compatible scalar/container/schema types are still valid at
  boundaries.
- Field access, equality, ordering, boolean logic, membership, object/list
  construction, and string interpolation are still the right small expression
  set.
- Arrays and floats can be stored, compared, passed to `coerce`, and displayed.

What changes:

- `coerce` is no longer a synchronous `let`-style value operation.
- `coerce` enqueues a durable `schema.coerce` effect. Current implementations
  may still report this as `coerce`.
- `after review succeeds` narrows `review` to the typed success payload.
- `record ReviewedWork` creates a durable typed workflow fact.
- Nontrivial data reasoning should happen in a schema-coercion backend or a
  capability, not inside WhippleScript.

## Ralph Loop

An infinite loop that waits for the agent to finish before asking for another
small step:

```whipplescript
workflow Ralph

agent ralph {
  profile "repo-writer"
  capacity 1
}

rule begin
  when {
    started
    ralph is available
  }
=> {
  tell ralph "Do one small useful thing and update the todo list."
}

rule again
  when {
    ralph completed turn
    ralph is available
  }
=> {
  tell ralph "Do one small useful thing and update the todo list."
}
```

This is recursive, but the cycle crosses an external event:

```text
tell -> harness turn -> completion event -> tell
```

It cannot spin internally.

Operationally, this runs as:

```text
program Ralph deployed once
instance created
rule begin enqueues agent.tell
harness completes the turn
completion event enters the instance log
rule again enqueues the next agent.tell
```

## Bounded Implementation Workers

```whipplescript
workflow ImplementSpec

agent worker {
  profile "repo-writer"
  capacity 5
}

agent reviewer {
  profile "repo-reader"
  capacity 1
}

rule implement_ready_work
  when {
    ready work as item
    worker is available
  }
=> {
  tell worker """markdown
  Claim and implement this work item:

  {{ item.goal }}
  """
}

rule review_successful_work
  when {
    worker completed work as item
    reviewer is available
  }
=> {
  tell reviewer """markdown
  Review this work item:

  {{ item.goal }}
  """
}

rule accept
  when reviewer accepted work as item
=> {
  done item -> record AcceptedWork {
    id item.id
    status "accepted"
  }
}

rule retry
  when reviewer rejected work as item
=> {
  ready item
}
```

Capacity is visible on the agent declaration, but not because an agent secretly
is a pool. It is a runtime invariant over turn effects for that role.

## Research Fan-Out

```whipplescript
workflow Research

agent researcher {
  profile "internet-research"
  capacity 8
}

agent synthesizer {
  profile "repo-reader"
  capacity 1
}

rule investigate
  when {
    open question as q
    researcher is available
  }
=> {
  tell researcher """markdown
  Research this question and return cited findings:

  {{ q.text }}
  """
}

rule collect
  when researcher completed question as q
=> {
  finding q.result
}

rule synthesize
  when {
    findings are sufficient for dossier as d
    synthesizer is available
  }
=> {
  tell synthesizer """markdown
  Synthesize these findings into a concise answer:

  {{ d.findings }}
  """
}
```

The phrase `findings are sufficient` will need a precise lowering, probably via
a typed coerce/classification rule. This is a good example of syntax that feels
right but needs semantic discipline.

## OpenClaw-Lite Composition

OpenClaw-lite is an example composition, not a special mode. It combines core
registries with package/provider capabilities:

```whipplescript
workflow OpenClawLite

use std.memory

class OpenClawCronTick {
  job string
  interval string
  firedAt string
  status "ready"
}

class OpenClawHeartbeat {
  job string
  interval string
  firedAt string
  status "observed"
}

class WorkItem {
  title string
  body string
  status "ready"
}

class PlannedWork {
  title string
  body string
  context MemoryContext
  turn AgentTurn
  status "planned"
}

class MemoryContext {
  summary string
  target string
}

agent planner {
  profile "repo-reader"
  capacity 1
  skills ["whipplescript-author", "human-review-user"]
}

agent worker {
  profile "repo-writer"
  capacity 2
  skills ["whipplescript-author", "loft-user"]
}

rule seed_fixture_tick
  when started
=> {
  record OpenClawCronTick {
    job "openclaw-heartbeat"
    interval "15m"
    firedAt "fixture-now"
    status "ready"
  }

  record WorkItem {
    title "Try the OpenClaw-lite composition"
    body "Use a heartbeat, explicit memory recall, skill-guided planning, and human-visible escalation without an OpenClaw gateway."
    status "ready"
  }
}

rule emit_heartbeat
  when OpenClawCronTick as tick where tick.status == "ready"
=> {
  emit openclaw.heartbeat as heartbeat

  after heartbeat succeeds {
    done tick -> record OpenClawHeartbeat from tick {
      job
      interval
      firedAt
      status "observed"
    }
  }
}

rule plan_ready_work
  when {
    OpenClawHeartbeat as beat where beat.status == "observed"
    WorkItem as item where item.status == "ready"
    planner is available
  }
=> {
  call memory.query for item as context

  after context succeeds as memory {
    tell planner as plan """markdown
    Use the attached skills and recalled memory to produce a short plan.

    Work:
    {{ item.title }}

    {{ item.body }}

    Memory:
    {{ memory.summary }}
    """
  }

  after plan succeeds {
    done item -> record PlannedWork from item {
      title
      body
      context context
      turn plan
      status "planned"
    }
  }
}

rule request_human_trace_review
  when OpenClawHeartbeat as beat where beat.status == "observed"
=> {
  askHuman """markdown
  OpenClaw-lite heartbeat {{ beat.job }} fired at {{ beat.firedAt }}.
  Review the trace if the plan or memory context needs human attention.
  """
}
```

The important part is not the name. The important part is that skills,
heartbeat scheduling, agent turns, Loft work claims, memory access, human
review, and evidence tracing are composed through the same small rule/effect
kernel.

In production, cron belongs at the control-plane boundary: the scheduler appends
a durable tick/event, and the workflow reacts to that observation. The checked
fixture seeds one `OpenClawCronTick` from `started` so validation runs are
deterministic; it does not add an OpenClaw gateway or a hidden scheduler loop.

## Current Checked Fixtures

The curated checked examples in `examples/` are documented in
[`../docs/examples.md`](../docs/examples.md). The current learning path is:

- `minimal-noop.whip`: smallest complete workflow.
- `human-review.whip`: manual approval with typed answer facts.
- `triage-flow.whip`: sequential flow with branching and terminal outcomes.
- `coerce-branch.whip`: typed schema coercion and explicit routing.
- `terminal-output-union.whip`: exhaustive effect-terminal handling.
- `incident-router.whip`: rich guards, collection access, and `AgentRef`.
- `scheduled-escalation.whip`: timeout, timer, cancellation, and escalation.
- `exec-json-ingest.whip`: typed command output and JSON ingestion.
- `event-bridge.whip`: external events and directed notifications.
- `reusable-review-pattern.whip`: compile-time pattern reuse.
- `queue-worker-with-review.whip`: queue claim, agent turn, typed review, and
  finish/release policy.
- `multi-agent-bounded-concurrency.whip`: capacity-bounded agent handoff.
- `circuit-breaker.whip`: counter-driven resilience policy.
- `ralph.whip`: minimal recurring service loop.
- `openclaw-lite.whip`: heartbeat, planning, queue filing, and human review.
- `autoresearch-lite.whip`: experiment budget and typed metric decision.
- `gastown-lite.whip`: coding-agent queue/lease/review ledger.

Runtime test fixtures such as `provider-language-e2e.whip`,
`package-memory.whip`, and `queue-gated-smoke.whip` remain checked, but they are
not the curated authoring path.

Each checked source has a matching `.ir` snapshot consumed by parser tests.

## Validation Notes

First authoring pass guesses:

- Equality guards such as `when reviewed.status == Accept` feel natural, but
  the implemented grammar does not support guard expressions yet. For now this
  is a hard diagnostic; authors should route through typed facts or a coerce
  `coerce` result.
- Binding an effect after a multi-line string, for example closing with
  `""" as plan`, also feels natural. The current parser only recognizes
  `as binding` on the effect line. This should become either a supported alias
  or a targeted diagnostic before v0.
- Package-specific shorthand such as `memory.query item as context` is
  appealing, but the current durable surface is the generic `call` capability
  effect. Keep shorthand out of the grammar until package effect registration
  and docs can make the lowering auditable.
