# Hybrid Token Flow Example

Status: design sketch, not implemented syntax

This sketch tests a hybrid model:

- the machine statechart controls global mode
- places hold durable typed token multisets
- transitions consume and produce tokens
- agent turns are explicit effects emitted by transitions
- lifecycle events move tokens back through the net

The goal is to keep execution logic visible while avoiding fake "agent pool"
semantics. Capacity is represented as tokens, not hidden inside an agent.

```armature
machine SpecImplementation
initial active

class Task {
  id string
  goal string
  files string[]
  dependsOn string[]
}

class Slot {
  id string
}

class Work {
  task Task
  slot Slot
  turnId string
}

class Review {
  task Task
  slot Slot
  turnId string
}

class TurnCompleted {
  turnId string
  agent string
  status string
  summary string
}

agent worker = codingAgent {
  profile "repo-writer"
}

agent reviewer = codingAgent {
  profile "repo-reader"
}

place ready Task
place running Work
place needsReview Task
place reviewing Review
place done Task
place blocked Task

place workerSlots Slot = [
  { id "worker-1" },
  { id "worker-2" },
  { id "worker-3" }
]

place reviewerSlots Slot = [
  { id "reviewer-1" }
]

event begin {
  tasks Task[]
}

event turnCompleted TurnCompleted

state active {
  initial running

  state running {
    on begin as input {
      put input.tasks into ready
      stay
    }

    transition startImplementation
      take task from ready
      take slot from workerSlots
      guard task.dependsOn.all(id => done contains id)
    {
      let turn = tell worker slot.id """
      Implement this task:

      {{ task.goal }}

      Stay within these files unless you explain why more are needed:
      {{ task.files }}

      When finished, summarize the exact changes and tests.
      """

      put { task task, slot slot, turnId turn.id } into running
      stay
    }

    transition implementationSucceeded
      on turnCompleted as event
      take work from running where work.turnId == event.turnId
      guard event.agent == "worker"
      guard event.status == "succeeded"
    {
      put work.task into needsReview
      put work.slot into workerSlots
      stay
    }

    transition implementationFailed
      on turnCompleted as event
      take work from running where work.turnId == event.turnId
      guard event.agent == "worker"
      guard event.status != "succeeded"
    {
      put work.task into blocked
      put work.slot into workerSlots
      stay
    }

    transition startReview
      take task from needsReview
      take slot from reviewerSlots
    {
      let turn = tell reviewer slot.id """
      Review this completed task:

      {{ task.goal }}

      Check correctness, tests, and whether the implementation stayed within:
      {{ task.files }}

      Return accepted, rejected, or blocked with a concise reason.
      """

      put { task task, slot slot, turnId turn.id } into reviewing
      stay
    }

    transition reviewAccepted
      on turnCompleted as event
      take review from reviewing where review.turnId == event.turnId
      guard event.agent == "reviewer"
      guard event.status == "accepted"
    {
      put review.task into done
      put review.slot into reviewerSlots
      stay
    }

    transition reviewRejected
      on turnCompleted as event
      take review from reviewing where review.turnId == event.turnId
      guard event.agent == "reviewer"
      guard event.status == "rejected"
    {
      put review.task into ready
      put review.slot into reviewerSlots
      stay
    }

    transition reviewBlocked
      on turnCompleted as event
      take review from reviewing where review.turnId == event.turnId
      guard event.agent == "reviewer"
      guard event.status == "blocked"
    {
      put review.task into blocked
      put review.slot into reviewerSlots
      stay
    }

    always
      guard ready.empty
      guard running.empty
      guard needsReview.empty
      guard reviewing.empty
      guard blocked.empty
    {
      goto done
    }

    always
      guard blocked.notEmpty
      guard ready.empty
      guard running.empty
      guard needsReview.empty
      guard reviewing.empty
    {
      goto needsHuman
    }
  }

  state needsHuman {
    on humanResolved as resolution {
      put resolution.tasks into ready
      goto running
    }
  }

  state done final
}
```

## What This Is Testing

This example deliberately avoids `maxActive` on agents. The three concurrent
worker turns come from three `workerSlots` tokens. The reviewer has one slot, so
review is serialized.

The important runtime facts are explicit:

```text
ready          tasks not yet started
running        tasks currently owned by a worker turn
needsReview    completed implementation tasks waiting for review
reviewing      tasks currently owned by a reviewer turn
done           accepted tasks
blocked        tasks that need human or planning intervention
workerSlots    available implementation capacity
reviewerSlots  available review capacity
```

The machine's mode is also explicit:

```text
active.running
active.needsHuman
active.done
```

## Immediate Design Questions

1. Should `transition` blocks be first-class state members, or should this be
   spelled as `on` plus `take` statements?
2. Should places be globally declared, or scoped inside states?
3. Should `tell worker slot.id` create or resume a concrete worker session?
4. Should `put input.tasks into ready` mean bulk insertion, or require an
   explicit loop-like operation?
5. Should `take` reserve tokens before effects run, or only after effects are
   durably accepted?
6. Should a failed effect automatically roll consumed tokens back?
7. Is `done contains id` acceptable expression syntax, or do we need a more
   explicit indexed projection?
8. Does this model need event correlation by `turnId` only, or should the
   runtime bind completion events to consumed token claims automatically?
