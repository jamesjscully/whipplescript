# Spec Implementation Workflow Example

Status: implemented example fixture

This file mirrors the live example at
`examples/workflows/spec-implementation.armature`. The workflow replaces ad hoc
director scripts with a constrained hierarchical statechart, BAML-shaped typed
coercions, and explicit capability boundaries.

```armature
machine specImplementation
initial running

data {
  seenRuns string[] = []
  lastIdleNudgeAt time? = nil
}

agent director = thread("director")
agent worker = codingAgent() {
  maxActive 2
}
agent quality = codingAgent() {
  maxActive 1
}

capability plan = adapter("implementationPlan")

event finished {
  id string
  name string
  status string
  stdoutTail string
  stderrTail string
  exitCode int?
}

event idle {
  activeRuns int
  unfinishedItems int
}

enum RunKind {
  WorkerComplete
  WorkerFailed
  QualityPassed
  QualityFailed
  Irrelevant
}

class RunSummary {
  id string
  name string
  status string
  stdoutTail string
  stderrTail string
  exitCode int?
}

class RunClassification {
  kind RunKind
  workItemId string?
  reason string
}

enum NextAction {
  StartWorker
  StartQuality
  AskHuman
  Wait
  Done
}

class NextStep {
  action NextAction
  workItemId string?
  reason string
  message string?
}

coerce classifyRun(run RunSummary) -> RunClassification {
  model "gpt-4o-mini"

  prompt """
  Classify this finished agent run for the spec implementation workflow.

  {{ run }}

  Use WorkerComplete only when a worker appears to have completed an
  implementation item. Use QualityPassed only when review accepted a specific
  item. Use Irrelevant when the run should not affect the plan.

  {{ ctx.output_format }}
  """
}

coerce chooseNextStep(planText string) -> NextStep {
  model "gpt-4o-mini"

  prompt """
  Select the next safe workflow action.

  {{ planText }}

  Prefer unblocked implementation items. Do not start duplicate work. Start
  quality only for items marked ready for quality. Use AskHuman when the plan is
  contradictory or cannot safely advance.

  {{ ctx.output_format }}
  """
}

state running {
  initial watching

  on finished as run
    guard !(run.id in data.seenRuns)
  {
    case run.name {
      matches "worker-*" -> {
        assign data.seenRuns = data.seenRuns.append(run.id)

        let classification = coerce classifyRun({
          id run.id
          name run.name
          status run.status
          stdoutTail run.stdoutTail
          stderrTail run.stderrTail
          exitCode run.exitCode
        })

        case classification.kind {
          WorkerComplete -> {
            plan.markReadyForQuality(classification.workItemId)

            start quality {
              task classification.workItemId
              message "Review completed worker task."
            }

            stay
          }

          WorkerFailed -> {
            plan.markBlocked(classification.workItemId, classification.reason)

            send director """
            Worker failed: {{ classification.reason }}
            """

            stay
          }

          _ -> {
            stay
          }
        }
      }

      matches "quality-*" -> {
        assign data.seenRuns = data.seenRuns.append(run.id)

        let classification = classifyRun({
          id run.id
          name run.name
          status run.status
          stdoutTail run.stdoutTail
          stderrTail run.stderrTail
          exitCode run.exitCode
        })

        case classification.kind {
          QualityPassed -> {
            plan.markDone(classification.workItemId)
            goto choosing
          }

          QualityFailed -> {
            plan.markBlocked(classification.workItemId, classification.reason)
            askHuman(classification.reason)
            stay
          }

          _ -> {
            stay
          }
        }
      }

      _ -> {
        stay
      }
    }
  }

  state watching {
    on idle as observation
      guard observation.activeRuns == 0
      guard observation.unfinishedItems > 0
      guard elapsedSince(data.lastIdleNudgeAt) >= 2m
    {
      assign data.lastIdleNudgeAt = now()
      goto choosing
    }
  }

  state choosing {
    entry {
      let planText = plan.snapshot()
      let next = coerce chooseNextStep(planText)

      case next.action {
        StartWorker -> {
          start worker {
            task next.workItemId
            message next.message
          }

          goto watching
        }

        StartQuality -> {
          start quality {
            task next.workItemId
            message next.message
          }

          goto watching
        }

        AskHuman -> {
          askHuman(next.reason)
          goto watching
        }

        Wait -> {
          send director next.reason
          goto watching
        }

        Done -> {
          goto done
        }
      }
    }
  }

  state done {
    final
  }
}

invariant declaredAgentsOnly
invariant declaredEffectsOnly
invariant agentCapabilitiesRespected
invariant maxActiveRespected
invariant terminalInvocationsObserved
invariant failedEffectsAreDurable
invariant blockedWorkIsVisible
```

## What This Replaces

The equivalent shell supervisor should only need to emit coarse runtime events:

```text
finished
idle
```

It should not decide what to do next. The statechart owns that logic and the
runtime owns durable event handling.

## Why This Is Safer Than TypeScript

The workflow can express semantic orchestration:

```text
classify completed run
update item state through a declared plan capability
start bounded worker
start quality gate
escalate for human review
finish when done
```

But it cannot:

```text
open arbitrary files
run shell commands
install packages
create hidden sockets
ignore concurrency limits
call undeclared agents
invent capabilities at runtime
```

That is the intended security and reliability boundary.
