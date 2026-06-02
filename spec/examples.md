# Examples

Status: sketchpad

For canonical terminology and the current language reference, start with
[`../docs/language-reference.md`](../docs/language-reference.md). This file
remains a design sketchpad for example shapes.

These examples are intentionally small. The goal is to feel the authoring model
before committing to syntax.

## Simple Loft Worker

This is the best current v0 candidate for the smallest useful real workflow:

```whipplescript
workflow SimpleLoftWorker


agent worker {
  profile "repo-writer"
  capacity 1
  skills ["loft-user"]
}

rule start_ready_issue
  when {
    loft has ready issue as issue
    worker is available
  }
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """
    Implement this Loft issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}

    When finished, update the Loft issue with a concise note and evidence.
    """
  }

  after claim fails {
    askHuman """
    WhippleScript could not claim this Loft issue:

    {{ claim.issue_id }}

    Reason:
    {{ claim.reason }}

    Please inspect the issue state or retry later.
    """
  }
}

rule recover_idle
  when no active agent turns
  when loft has unfinished issues
  when no claimable effects
=> {
  askHuman """
  There is unfinished Loft work, but WhippleScript has no active turns or
  claimable effects. Inspect status and decide whether to resume, split, or
  block the work.
  """
}
```

Why this script is important:

- `claim` and `tell` are not sequenced by source order.
- `after claim succeeds` creates a durable dependency edge.
- The worker prompt can use `claim.issue` only inside the success scope.
- Claim failure is handled separately from worker failure.
- The idle recovery rule reacts to facts; it does not run a hidden supervisor
  loop.

## Loft Worker With BAML Review

This example adds a typed model decision. It uses BAML-shaped `enum`, `class`,
and `coerce` declarations, but keeps ordinary data operations small and pure.

```whipplescript
workflow LoftWorkerWithReview


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
  turn AgentTurn
  review WorkReview
}

coerce reviewWork(issueTitle string, agentSummary string, changedFiles string[]) -> WorkReview {
  prompt """
  Review this completed agent turn for the Loft issue.

  Issue:
  {{ issueTitle }}

  Agent summary:
  {{ agentSummary }}

  Changed files:
  {{ changedFiles }}

  Return a structured review.

  {{ ctx.output_format }}
  """
}

agent worker {
  profile "repo-writer"
  capacity 1
  skills ["loft-user"]
}

rule start_ready_issue
  when {
    loft has ready issue as issue
    worker is available
  }
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """
    Implement this Loft issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}

    Finish with a concise summary and list of changed files.
    """
  }
}

rule review_finished_turn
  when worker completed turn for loft issue as turn
=> {
  coerce reviewWork(turn.issue.title, turn.summary, turn.changedFiles) as review

  after review succeeds {
    record ReviewedWork {
      turn turn
      review review
    }
  }

  after review fails {
    askHuman """
    BAML review failed for this completed turn:

    {{ turn.id }}

    Please inspect the turn artifacts and decide whether to accept, revise, or
    block the issue.
    """
  }
}

rule accept_reviewed_work
  when ReviewedWork as reviewed
  when reviewed.review.status == Accept
=> {
  close reviewed.turn.issue with loft
}

rule request_revision
  when {
    ReviewedWork as reviewed
    reviewed.review.status == Revise
    worker is available
  }
=> {
  tell worker """
  Revise this Loft issue:

  {{ reviewed.turn.issue.title }}

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
  askHuman """
  The model review says this issue is blocked:

  {{ reviewed.turn.issue.title }}

  Reason:
  {{ reviewed.review.reason }}
  """
}
```

What transfers from the legacy expression design:

- BAML-compatible scalar/container/schema types are still valid at boundaries.
- Field access, equality, ordering, boolean logic, membership, object/list
  construction, and string interpolation are still the right small expression
  set.
- Arrays and floats can be stored, compared, passed to `coerce`, and displayed.

What changes:

- `coerce` is no longer a synchronous `let`-style value operation.
- `coerce` enqueues a durable `baml.coerce` effect.
- `after review succeeds` narrows `review` to the typed success payload.
- `record ReviewedWork` creates a durable typed workflow fact.
- Nontrivial data reasoning should happen in BAML or a capability, not inside
  WhippleScript.

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
  tell worker """
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
  tell reviewer """
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
  tell researcher """
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
  tell synthesizer """
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
registries with a few plugins:

```whipplescript
workflow OpenClawLite

use memory

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

  after context succeeds as memory => {
    tell planner as plan """
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
  askHuman """
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

The checked example fixtures in `examples/` are:

- `minimal-noop.whip`: smallest rule/fact shape with no external effect.
- `ralph.whip`: recursive external-turn loop.
- `loft-worker-with-review.whip`: Loft claim before agent turn, BAML
  review, and human fallback.
- `coerce-branch.whip`: typed model classification followed by explicit
  routing.
- Codex French poem fixture: Codex writes a French poem artifact, then a second
  Codex turn judges whether it is a real French poem.
- `codex-poem-coerce-review.whip`: Codex writes a French poem artifact, then a
  typed BAML `coerce` reviews the completed turn.
- `human-review.whip`: manual review request and answer recording.
- `implementation-plan-phase-review.whip`: fan out implementation-plan phase
  reviews to Codex/repo-reader turns and a visible tracker.
- `multi-agent-bounded-concurrency.whip`: two agents with explicit capacity
  bounds.
- `openclaw-lite.whip`: planner, implementer, verifier, and human approval
  composition.
- `plugin-memory.whip`: memory plugin capability call before an agent turn.
- `provider-language-e2e.whip`: Codex, Claude, and Pi language-generation
  turns across six languages, each reviewed by typed BAML coercion.

Each checked source has a matching `.ir` snapshot consumed by parser tests.

## Validation Notes

First authoring pass guesses:

- Equality guards such as `when reviewed.status == Accept` feel natural, but
  the implemented grammar does not support guard expressions yet. For now this
  is a hard diagnostic; authors should route through typed facts or a BAML
  `coerce` result.
- Binding an effect after a multi-line string, for example closing with
  `""" as plan`, also feels natural. The current parser only recognizes
  `as binding` on the effect line. This should become either a supported alias
  or a targeted diagnostic before v0.
- Plugin-specific shorthand such as `memory.query item as context` is
  appealing, but the current durable surface is the generic `call` capability
  effect. Keep shorthand out of the grammar until plugin effect registration
  and docs can make the lowering auditable.
