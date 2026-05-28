# Examples

Status: sketchpad

These examples are intentionally small. The goal is to feel the authoring model
before committing to syntax.

## Simple Docket Worker

This is the best current v0 candidate for the smallest useful real workflow:

```armature
workflow SimpleDocketWorker

use skill "docket-user"
use skill "human-review-user"

agent worker {
  profile "repo-writer"
  capacity 1
  skills ["docket-user"]
}

rule start_ready_issue
  when docket has ready issue as issue
  when worker is available
=> {
  claim issue with docket as claim

  after claim succeeds {
    tell worker """
    Implement this Docket issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}

    When finished, update the Docket issue with a concise note and evidence.
    """
  }

  after claim fails {
    askHuman """
    Armature could not claim this Docket issue:

    {{ claim.issue_id }}

    Reason:
    {{ claim.reason }}

    Please inspect the issue state or retry later.
    """
  }
}

rule recover_idle
  when no active agent turns
  when docket has unfinished issues
  when no claimable effects
=> {
  askHuman """
  There is unfinished Docket work, but Armature has no active turns or
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

## Docket Worker With BAML Review

This example adds a typed model decision. It uses BAML-shaped `enum`, `class`,
and `coerce` declarations, but keeps ordinary data operations small and pure.

```armature
workflow DocketWorkerWithReview

use skill "docket-user"
use skill "baml-coerce-user"
use skill "human-review-user"

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
  Review this completed agent turn for the Docket issue.

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
  skills ["docket-user"]
}

rule start_ready_issue
  when docket has ready issue as issue
  when worker is available
=> {
  claim issue with docket as claim

  after claim succeeds {
    tell worker """
    Implement this Docket issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}

    Finish with a concise summary and list of changed files.
    """
  }
}

rule review_finished_turn
  when worker completed turn for docket issue as turn
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
  close reviewed.turn.issue with docket
}

rule request_revision
  when ReviewedWork as reviewed
  when reviewed.review.status == Revise
  when worker is available
=> {
  tell worker """
  Revise this Docket issue:

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
  Armature.

## Ralph Loop

An infinite loop that waits for the agent to finish before asking for another
small step:

```armature
workflow Ralph

agent ralph {
  profile "repo-writer"
  capacity 1
}

rule begin
  when started
  when ralph is available
=> {
  tell ralph "Do one small useful thing and update the todo list."
}

rule again
  when ralph completed turn
  when ralph is available
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

```armature
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
  when ready work as item
  when worker is available
=> {
  tell worker """
  Claim and implement this work item:

  {{ item.goal }}
  """
}

rule review_successful_work
  when worker completed work as item
  when reviewer is available
=> {
  tell reviewer """
  Review this work item:

  {{ item.goal }}
  """
}

rule accept
  when reviewer accepted work as item
=> {
  complete item
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

```armature
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
  when open question as q
  when researcher is available
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
  when findings are sufficient for dossier as d
  when synthesizer is available
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

```armature
workflow OpenClawLite

use skill "armature-author"
use skill "docket-user"
use skill "human-review-user"
use plugin "memory"

agent worker {
  profile "repo-writer"
  capacity 3
  skills ["docket-user", "memory-user"]
}

rule heartbeat
  when every 15m
=> {
  emit heartbeat
}

rule start_ready_issue
  when heartbeat
  when docket has ready issue as issue
  when worker is available
=> {
  claim issue with docket as claim

  after claim succeeds {
    memory.query claim.issue.summary as memory

    after memory succeeds {
      tell worker """
      Implement this Docket issue:

      {{ claim.issue.title }}

      Relevant memory:
      {{ memory.results }}

      Update the issue with evidence when done.
      """
    }
  }
}

rule ask_when_idle
  when heartbeat
  when docket has unfinished issues
  when no active agent turns
=> {
  askHuman """
  Armature is idle but Docket still has unfinished work.
  Inspect the trace and decide whether to resume, split, or block the work.
  """
}
```

The important part is not the name. The important part is that skills,
heartbeat scheduling, agent turns, Docket work claims, memory access, human
review, and evidence tracing are composed through the same small rule/effect
kernel.
