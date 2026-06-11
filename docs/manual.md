# Authoring guide

How to structure WhippleScript workflows well: modeling data, sequencing
effects, branching, composing source, and debugging runs. New users should
start with the [quickstart](quickstart.md) and [tutorial](tutorial.md); exact
syntax for every construct is in the
[language reference](language-reference.md).

The premise behind all of the guidance here: rules own policy and stay
deterministic; everything external is a durable effect; the runtime owns
delivery, retries, idempotency, and inspection.

## Model data with classes and enums

Facts are typed. Use enums for closed decision sets and literal fields for
small state machines:

```whip
enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class WorkItem {
  id string
  title string
  status "queued" | "reviewed"
}

class WorkReview {
  status ReviewStatus
  reason string
  confidence float
}
```

Literal-typed status fields keep guards deterministic and make illegal states
unrepresentable: a rule matching `where item.status == "queued"` cannot also
see a reviewed item.

Prefer consuming and re-recording facts over mutating status in place —
`done item -> record WorkItem { ... status "reviewed" }` makes the state
transition atomic with whatever else the rule commits.

## Request agent work

```whip
agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
}

rule implement
  when WorkItem as item where item.status == "queued"
  when worker is available
=> {
  tell worker requires ["agent.tell"] as turn """markdown
  Implement this item:

  {{ item.title }}
  """

  after turn succeeds as completed {
    done item -> record WorkItem {
      id item.id
      title item.title
      status "reviewed"
    }
  }
}
```

`tell` records an effect; the provider runs later. Three consequences worth
internalizing:

- Source order inside a rule body does not order effects. Use `after` blocks
  to create dependency edges.
- Effect output (`completed` above) is visible only inside the `after` block
  that proves the terminal status. This is what makes causality auditable.
- `when worker is available` is the capacity gate; without it the rule still
  fires but the effect may block on capacity.

## Make typed model decisions with `coerce`

When a judgment call should produce structured data rather than prose,
declare a coerce function and branch on its completion:

```whip
coerce reviewWork(title string, summary string) -> WorkReview {
  prompt """markdown
  Review the completed work.

  Title: {{ title }}
  Summary: {{ summary }}

  {{ ctx.output_format }}
  """
}

rule review
  when ...
=> {
  tell worker as turn "..."

  after turn succeeds as completed {
    coerce reviewWork(item.title, completed.summary) as review
  }

  after review succeeds as result {
    record ReviewedWork {
      item item
      review result
    }
  }

  after review fails as failure {
    askHuman "Review failed: {{ failure.reason }}"
  }
}
```

`coerce` is an effect, not a function call: it is durable, it can fail, and
its typed output is only available in the `after` branch.

## Branch deterministically

Guards handle filtering; `case` handles finite domains; `after ... completes`
handles exhaustive terminal-status handling:

```whip
rule accept
  when ReviewedWork as reviewed where reviewed.review.status == Accept
=> { ... }
```

```whip
case review.status {
  Accept => {
    record AcceptedWork { id item.id }
  }
  Revise => {
    tell worker as revision "Revise {{ item.title }}."
  }
  Blocked => {
    askHuman "Blocked: {{ review.reason }}"
  }
}
```

```whip
after turn completes {
  case turn.output {
    Completed result => { record TurnSucceeded { summary result.summary } }
    Failed failure   => { record TurnFailed { reason failure.reason } }
    TimedOut timeout => { record TurnTimedOut { reason timeout.reason } }
    Cancelled c      => { record TurnCancelled { reason c.reason } }
  }
}
```

Branch over typed values. Never parse prompt text to decide a route or a
status — if a decision needs model judgment, make it a `coerce` and branch on
the typed result.

## Sequential flows

Most orchestration is best expressed as independent rules: each reacts to the
facts it cares about, and the runtime sequences them through `after` edges.
But some work is genuinely a script — do this, then ask a human, then branch —
and threading it through nested `after` blocks obscures the sequence. A `flow`
writes that sequence top to bottom while lowering to the same rules:

```whip
flow triage
  when Ticket as ticket
{
  tell triager as turn "Plan {{ ticket.title }}."
  askHuman as signoff "Approve {{ turn.summary }}?"

  when signoff.choice == "approve" {
    complete result { decision signoff.choice }
  } else {
    fail error { reason "rejected" }
  }
}
```

Inside a flow, each effect step's output is in scope for the steps that follow
(`turn`, then `signoff`), so you do not nest `after` blocks for the common
case. Attach `on fails { ... }` / `on timeout { ... }` to a step to handle its
failure paths, and use `when <expr> { } else { }` to branch on a prior step's
output.

When to reach for a flow versus plain rules:

| Situation | Use |
| --- | --- |
| Steps that always run in a fixed order, with shared bindings | `flow` |
| Independent reactions that fan in from different facts | separate `rule`s |
| One linear path with a human gate in the middle | `flow` |
| Branchy policy where order is data-driven, not positional | `rule`s + guards |

A flow is not a new runtime mode. It compiles to ordinary rules
(`flow.<name>.seg0`, `seg1`, …) plus a generated await state, all visible in
`whip check`. Everything you know about rules — atomic commits, durable
effects, `after` semantics — still applies.

## Work queues

When work arrives as a backlog rather than as facts you seed up front, declare
a queue and let rules claim from it:

```whip
queue backlog {
  tracker builtin
}

rule pick_up
  when backlog has ready item as item
  when worker is available
=> {
  claim item as work
  tell worker as turn "Resolve {{ work.title }}."

  after work fails as taken {
    // another claimant won the race — just wait for the next ready item
  }
}
```

The verbs are `file item into <queue> { ... }`, `claim`, `release`, and
`finish`. A losing `claim` is a normal branchable failure, not an error, so a
contended queue stays correct without locks in source.

Operate the backlog from the CLI with `whip items add`, `whip items list`, and
`whip items show`. The builtin tracker is workspace-scoped and issues ids like
`WS-1`; items an agent files mid-turn carry run-identity provenance. Full
syntax is in the [language reference](language-reference.md#work-queues).

## Time and deadlines

Keep time out of guards — guards must stay pure. Express deadlines as effects:

- `timeout <dur>` on an effect bounds how long it may run; an
  `after ... times out` / `on timeout` branch reacts when it expires.
- `timer <dur> as deadline` is a standalone delay you branch on with
  `after deadline succeeds`.
- `cancel <binding>` stops a pending or running effect you bound earlier.

```whip
tell worker as turn timeout 10m "Do the work."

after turn times out as t {
  askHuman "Worker exceeded 10m — escalate?"
}
```

Durations are `<n><unit>` with units `s`/`m`/`h`/`d`. Timers and timeouts fire
on worker passes — there is no daemon — so `whip dev --until idle` treats
pending timers as idle, and `whip status` lists the time effects an instance is
waiting on.

## Express retries as facts

There is no built-in retry policy; retries are ordinary facts and rules,
which keeps them visible and auditable:

```whip
rule attempt_job
  when Job as job where job.status == "pending" and job.attempts < 3
  when worker is available
=> {
  tell worker as turn "Do job {{ job.id }}"

  after turn succeeds as ok {
    done job -> record JobDone { id job.id }
  }

  after turn fails as failed {
    done job -> record Job {
      id job.id
      attempts job.attempts + 1
      status "pending"
    }
  }
}

rule give_up
  when Job as job where job.attempts >= 3
=> {
  done job -> record JobAbandoned { id job.id }
}
```

## Gate on humans

`askHuman` creates an inbox item; a `human answered` rule reacts to the
answer. The [tutorial](tutorial.md) builds this pattern end to end, and
[`examples/human-review.whip`](../examples/human-review.whip) is the minimal
version. Operate the inbox with `whip inbox`, `whip inbox show <item>`, and
`whip inbox answer <item> (--choice X | --text "...") [--by NAME]`.

## Run a local command (escape hatch)

`exec` has a dev form and a hosted form.

`exec "<command>" as result` runs a local command as an effect and exposes
`result.exit_code` and `result.stdout`. It is a dev-profile escape hatch,
deliberately constrained:

- There is no source syntax to grant it. The operator allow-lists commands
  through `WHIPPLESCRIPT_EXEC_ALLOW` (colon-separated glob prefixes such as
  `scripts/*`); anything outside the list fails and routes to `after x fails`.
- There is no sandbox for raw dev `exec` — a grant is a documented trust
  decision. Keep the allow-list as narrow as the workflow needs, and prefer
  agents, plugins, or child workflows when one of those fits.

Hosted deployments should use named script capabilities instead:

```whip
exec backup_repo with request -> Report as backup
```

The operator supplies `--exec-profile hosted --script-manifest <path>`. The
manifest maps `backup_repo` to argv, a pinned SHA-256 digest, and optional
secret references:

```json
{
  "backup_repo": {
    "argv": ["bash", "scripts/backup.sh"],
    "sha256": "9f2c...",
    "env": { "BACKUP_TOKEN": "env:BACKUP_TOKEN" }
  }
}
```

Hosted `exec` rejects raw command strings, verifies the script bytes before
spawn, runs argv-direct with typed JSON stdin, and records the executing hash.

Use `exec` for deterministic local steps that genuinely belong in the
workflow — running a test script, a linter — not as a way to smuggle
orchestration into shell.

## Compose source

| Need | Use |
| --- | --- |
| Split declarations across files | `include "schemas/common.whip"` |
| Bring in BAML classes/functions | `include "review.baml"` |
| Import plugin capabilities | `use memory` |
| Reuse a rule/effect fragment at compile time | `pattern` + `apply` |
| Sequence fixed steps with shared bindings | `flow` |
| Pull work from a durable backlog | `queue` + `claim` |
| Run work with its own lifecycle and terminal contract | `workflow` + `invoke` |

Patterns are compile-time templates — `apply` expands them into ordinary
declarations before type checking. `invoke` is runtime composition — the
child is a real instance, and the parent sees only its declared output or
failure payload:

```whip
invoke ReviewPhase {
  phase PhaseReviewRequest {
    id phase.id
    title phase.title
  }
} as child

after child succeeds as result {
  record ReviewComplete { phaseId phase.id result result }
}

after child fails as failure {
  record ReviewBlocked { phaseId phase.id reason failure.reason }
}
```

## End the workflow

Declare what the workflow produces, and make some rule produce it:

```whip
output result PhaseReviewResult
failure error ReviewPhaseFailure
```

`complete result { ... }` and `fail error { ... }` are atomic with the rule
commit and validate against the declared contract. `whip check` rejects
workflows with no path to a terminal; tag genuinely perpetual workflows
`@service` and externally-fed rules `@external` (see
[liveness checks](language-reference.md#liveness-checks)).

Remember the failure split: a provider failure is effect/run state for rules
to react to; `fail` is the workflow itself giving up. Don't conflate them —
deciding which provider failures are fatal is exactly the policy the source
should express.

## Debug a run

Work through the views in order:

```sh
whip status <instance>        # lifecycle, counts, recent events
whip log <instance>           # the event sequence
whip facts <instance>         # current fact state
whip effects <instance>       # effect status + policy_block_reason
whip runs <instance>          # provider attempts
whip diagnostics <instance>   # recorded errors
whip evidence <instance>      # provider payloads and artifacts
whip trace <instance> --check # lifecycle conformance
```

`whip --json dev <file> --provider fixture --until idle` plus assertions is
the tightest authoring loop: assertions turn "it seems to work" into a
checked claim about the final state.

## Checklist before sharing a workflow

- `whip check` passes; `@service`/`@external` tags appear only where
  intentional.
- Effects are ordered by `after` blocks, not source order.
- Routing decisions are typed source data (`AgentRef`, enums, literals) —
  not model output.
- Every failure branch retries, escalates, or deliberately ignores; none
  fall through silently.
- Agent profiles are as narrow as the work allows; plugin calls are explicit
  `call` effects; skills are attached to agents or turns, not imported.
- `whip --json dev` passes its assertions with the fixture provider, and
  `whip trace --check` reports conformance.

## Common mistakes

| Mistake | Fix |
| --- | --- |
| Relying on source order to sequence effects | Use `after effect succeeds/fails/completes`. |
| Reading effect output outside its `after` branch | Bind output inside the branch that proves the status. |
| Treating `coerce` as a local function call | Branch on the effect's completion. |
| Letting a model choose the provider or route | Use `AgentRef<...>`, enums, or literal fields. |
| Treating provider failure as workflow failure | Write a rule that decides when to `fail`. |
| Importing skills with `use` | Attach skills to agents or turns; `use` is for plugins. |
| Hiding orchestration in shell scripts around the CLI | Express it as rules, facts, and effects. |
| Reading the clock in a guard | Use a `timeout`, `timer`, or recorded fact. |
| Treating a lost `claim` as an error | Branch on the claim failure and wait for the next ready item. |
| Reaching for `emit` to log an event | `emit` was removed; derive facts from effect completions. |
| Granting raw dev `exec` broadly | Keep `WHIPPLESCRIPT_EXEC_ALLOW` as narrow as the workflow needs; use hosted script capabilities for untrusted authoring. |
| Credentials in `.whip` source | Use provider configuration references. |
