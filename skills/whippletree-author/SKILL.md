# Whippletree Author

Use this skill when authoring, reviewing, or operating `.whip` workflows.

## Mental Model

Whippletree is a restricted event-sourced rule machine.

- Rules match durable facts and write new facts or durable effects.
- Effects do not run inline. Providers claim and complete them later.
- Source order does not sequence effects. Use `after <effect> succeeds`,
  `after <effect> fails`, or `after <effect> completes`.
- Every effect must be auditable through status, events, evidence, and trace.
- Prefer Loft for project work, BAML `coerce` for typed model decisions, and
  plugins for memory or external systems.

## Valid Minimal Workflow

```whippletree
workflow MinimalNoop

class StartupSeen {
  source string
  state "observed"
}

rule observe_start
  when started
=> {
  record StartupSeen {
    source "external.started"
    state "observed"
  }
}
```

Run:

```sh
whip check examples/minimal-noop.whip
whip run examples/minimal-noop.whip --json
```

## Common Patterns

Loft claim before an agent turn:

```whippletree
rule start_ready_issue
  when loft has ready issue as issue
  when worker is available
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker "Implement {{ claim.issue.title }}"
  }
}
```

BAML classification with human fallback:

```whippletree
rule classify_request
  when WorkItem as request
=> {
  coerce classifyMessage(request.title, request.body) as classification

  after classification succeeds {
    record ClassifiedMessage {
      request request
      classification classification
    }
  }

  after classification fails {
    askHuman "Classify this request manually: {{ request.title }}"
  }
}
```

Plugin capability with explicit context:

```whippletree
rule recall_before_work
  when WorkItem as item
  when worker is available
=> {
  call memory.query for item as context

  after context succeeds {
    tell worker "Use this context: {{ context.summary }}"
  }
}
```

Fan-out with a visible tracker:

```whippletree
class ReviewRequest {
  number int
  title string
  trackerPath string
  status "queued"
}

class ReviewDispatch {
  request ReviewRequest
  turn AgentTurn
  status "dispatched"
}

agent reviewer {
  profile "repo-reader"
  capacity 4
}

rule seed_reviews
  when started
=> {
  record ReviewRequest {
    number 0
    title "Repository reset"
    trackerPath "spec/review-tracker.md"
    status "queued"
  }
}

rule dispatch_review
  when ReviewRequest as request
  when reviewer is available
=> {
  tell reviewer as turn """
  Review phase {{ request.number }}: {{ request.title }}.

  Update {{ request.trackerPath }} with status, findings, and evidence.
  """

  after turn succeeds {
    record ReviewDispatch {
      request request
      turn turn
      status "dispatched"
    }
  }
}
```

Use this when the operator needs to see progress outside the runtime store.
The workflow records durable dispatch facts, while the agent prompt tells each
thread exactly which tracker row or file to update. Keep the tracker path in a
fact so the prompt, evidence, and later review are tied to the same input.

## Profiles

Choose profiles by authority intent:

- `repo-reader`: inspect files and status, no writes.
- `repo-writer`: make code changes.
- `internet-research`: browse and cite external sources.
- `human-review`: request operator decisions.
- plugin profiles such as `memory-user`: use registered plugin capabilities.

Do not assign one powerful profile to unrelated work. Split agents by authority
and capacity.

## Current Grammar Limits

Do not write equality guards yet:

```whippletree
when review.status == Accept
```

Use typed facts or BAML classification branches until guard expressions are
implemented.

Put `as binding` on the effect line:

```whippletree
tell worker "Do it" as turn
```

Do not put `as turn` after a closing multiline string delimiter.

Use generic plugin calls for now:

```whippletree
call memory.query for item as context
```

Do not invent plugin-specific control-flow syntax.

## Debug Workflow

Use these commands first:

```sh
whip doctor
whip check workflow.whip
whip check --model-search workflow.whip
whip run workflow.whip --json
whip status <instance>
whip effects <instance>
whip runs <instance>
whip evidence <instance> --json
whip trace <instance> --check --json
```

If an effect does not run, check its status and policy block reason before
changing prompts.

## Safety

- Never hide orchestration in shell scripts or prompt text.
- Never rely on source order for effect sequencing.
- Do not silently inject memory. Query it as an effect and preserve evidence.
- Keep provider credentials outside workflow source.
- Use human review for destructive or ambiguous steps.
