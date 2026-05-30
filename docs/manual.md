# WhippleScript Manual

Status: draft

This manual explains how to build, run, inspect, and extend WhippleScript
workflows. Use [API Reference](api-reference.md) when you need exact command
syntax or crate-level surfaces.

## 1. What WhippleScript Is

WhippleScript is a durable orchestration language for agent work. It is not a
general programming language and it is not a prompt-only convention.

The core loop is:

```text
facts/events are present
rules match them
rules commit new facts and durable effects
workers execute effects
provider results return as events/facts
more rules may run
the workflow eventually completes, fails, or is cancelled
```

The language owns policy. The runtime owns durability, effect delivery, leases,
idempotency, retries, replay, and inspection.

## 2. Install And Check The Workspace

From the repository root:

```sh
cargo build --workspace
cargo run -p whipplescript-cli -- doctor
```

Optional formal tooling is available through the repo Nix shell:

```sh
nix develop
```

The full local check set is:

```sh
cargo fmt --all -- --check
cargo test --workspace
scripts/check-formal-models.sh
scripts/check-tla-models.sh
scripts/check-e2e.sh
```

For a single readiness artifact:

```sh
scripts/check-release-readiness.sh
```

## 3. Write A Minimal Workflow

Create `examples/minimal-noop.whip` style source:

```whip
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

Check it:

```sh
cargo run -p whipplescript-cli -- check examples/minimal-noop.whip
```

Compile it:

```sh
cargo run -p whipplescript-cli -- compile examples/minimal-noop.whip
```

## 4. Start And Inspect An Instance

Use a dedicated store for local runs:

```sh
STORE=.whipplescript/manual.sqlite
```

Start the workflow:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" \
  run examples/minimal-noop.whip \
  --input '{}' \
  --json
```

Save the returned `instance_id`.

Inspect:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" status <instance>
cargo run -p whipplescript-cli -- --store "$STORE" log <instance>
cargo run -p whipplescript-cli -- --store "$STORE" facts <instance>
```

At this point `run` has created the instance and start event. It has not stepped
rules unless you use `dev`.

Advance deterministic rules:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" \
  step <instance> --program examples/minimal-noop.whip
```

Inspect facts again:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" facts <instance>
```

## 5. Use `dev` For Local Validation

`dev` starts a new instance, steps rules, runs fixture workers, and evaluates
assertions:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" \
  dev examples/provider-language-e2e.whip \
  --provider fixture \
  --until idle \
  --json
```

Use fixture outcome switches to exercise terminal branches:

```sh
cargo run -p whipplescript-cli -- --store "$STORE" \
  dev examples/human-review.whip --provider fixture --fail

cargo run -p whipplescript-cli -- --store "$STORE" \
  dev examples/human-review.whip --provider fixture --timeout

cargo run -p whipplescript-cli -- --store "$STORE" \
  dev examples/human-review.whip --provider fixture --cancel
```

## 6. Model Data With Classes And Enums

Use classes for durable facts and effect payloads:

```whip
enum ReviewStatus {
  Accept
  Revise
  Blocked
}

class WorkItem {
  id string
  title string
  body string
  status "queued" | "reviewed"
}

class WorkReview {
  status ReviewStatus
  reason string
  confidence float
}
```

Use literal fields for small state machines. This keeps guards deterministic:

```whip
rule review_ready
  when WorkItem as item where item.status == "queued"
=> {
  ...
}
```

## 7. Request Agent Work

Declare an agent:

```whip
agent worker {
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell"]
  skills ["whipplescript-author"]
}
```

Use `tell` to enqueue work:

```whip
rule implement
  when WorkItem as item where item.status == "queued"
  when worker is available
=> {
  tell worker requires ["agent.tell"] as turn """
  Implement this item:

  {{ item.title }}
  {{ item.body }}
  """

  after turn succeeds as completed => {
    done item -> record WorkItem {
      id item.id
      title item.title
      body item.body
      status "reviewed"
    }
  }
}
```

`tell` creates an `agent.tell` effect. The provider does not run during the rule
commit. A worker claims and completes the effect later.

## 8. Use BAML Coercion For Typed Model Decisions

Declare a coerce function:

```whip
coerce reviewWork(title string, summary string) -> WorkReview {
  prompt """
  Review the completed work.

  Title:
  {{ title }}

  Summary:
  {{ summary }}

  {{ ctx.output_format }}
  """
}
```

Call it from a rule:

```whip
after turn succeeds as completed => {
  coerce reviewWork(item.title, completed.summary) as review
}

after review succeeds as result => {
  record ReviewedWork {
    item item
    review result
  }
}

after review fails as failure => {
  askHuman "Review failed: {{ failure.reason }}"
}
```

`coerce` is effectful. It is not a local function call.

## 9. Use Plugins And Skills Correctly

Import plugins with `use`:

```whip
use memory
```

Call plugin capabilities as effects:

```whip
call memory.query for item as context

after context succeeds as memory => {
  tell worker as turn "Use this context: {{ memory.summary }}"
}
```

Assign skills to agents or turns:

```whip
agent planner {
  profile "repo-reader"
  capacity 1
  skills ["whipplescript-author", "human-review-user"]
}
```

Skills are context bundles. They are not imports and they do not extend grammar.

## 10. Compose Source

Use `include` for source files:

```whip
include "schemas/common.whip"
include "review.baml"
```

Use `pattern` and `apply` for compile-time reuse:

```whip
pattern ReviewWithAgent<Input, Output> {
  input Input as item

  rule dispatch
    when Input as item
    when reviewer is available
  => {
    tell reviewer as turn "Review {{ item.title }}."

    after turn succeeds as completed => {
      done item -> record Output {
        turn completed
        status "reviewed"
      }
    }
  }
}

apply ReviewWithAgent<WorkItem, ReviewedWork> as ReviewWork {
  reviewer worker
}
```

Use `invoke` for runtime child workflows:

```whip
invoke ReviewPhase {
  phase PhaseReviewRequest {
    id phase.id
    title phase.title
  }
} as child

after child succeeds as result => {
  record ReviewComplete {
    phaseId phase.id
    result result
  }
}
```

Rule of thumb:

| Need | Use |
| --- | --- |
| Split declarations across files | `include` |
| Import provider/plugin resources | `use` |
| Reuse rule/effect fragments inline | `pattern` + `apply` |
| Run a child instance with its own lifecycle | `workflow` + `invoke` |

## 11. Complete Or Fail A Workflow

Declare terminal contracts:

```whip
workflow ReviewPhase {
  input phase PhaseReviewRequest
  output result PhaseReviewResult
  failure error ReviewPhaseFailure
  ...
}
```

Complete:

```whip
complete result {
  phaseId phase.id
  status "accepted"
}
```

Fail:

```whip
fail error {
  phaseId phase.id
  reason "review blocked"
}
```

Workflow terminal actions are atomic with the rule commit. A completed, failed,
or cancelled instance is terminal.

## 12. Branch Deterministically

Use guards for simple filtering:

```whip
rule accept
  when ReviewedWork as reviewed where reviewed.review.status == Accept
=> {
  ...
}
```

Use `case` for finite-domain branches:

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

Use `after effect completes` when all terminal statuses matter:

```whip
after turn completes {
  case turn.output {
    Completed result => {
      record TurnSucceeded { summary result.summary }
    }
    Failed failure => {
      record TurnFailed { reason failure.reason }
    }
    TimedOut timeout => {
      record TurnTimedOut { reason timeout.reason }
    }
    Cancelled cancelled => {
      record TurnCancelled { reason cancelled.reason }
    }
  }
}
```

Keep branching over typed values. Do not parse prompt text to decide route or
status.

## 13. Inspect And Debug

Use this sequence first:

```sh
whip status <instance>
whip log <instance>
whip facts <instance>
whip effects <instance>
whip runs <instance>
whip diagnostics <instance>
whip evidence <instance>
whip trace <instance> --check
```

If an effect did not run:

1. Check `effects <instance>` for status and `policy_block_reason`.
2. Check `runs <instance>` for provider attempts.
3. Check `diagnostics <instance>` for assertion/provider errors.
4. Check `evidence <instance>` for provider artifacts and failure details.
5. Check `trace <instance> --check` for lifecycle conformance.

## 14. Handle Human Review

`askHuman` creates a `human.ask` effect. The fixture worker turns it into an
inbox item.

List pending items:

```sh
whip inbox
```

Show one item:

```sh
whip inbox show <item>
```

Answer:

```sh
whip inbox answer <item> --choice approve --by alice
whip inbox answer <item> --text "Split this into smaller work" --by alice
```

The answer appends a durable event.

## 15. Operate Instances

Pause:

```sh
whip pause <instance>
```

Resume:

```sh
whip resume <instance>
```

Cancel:

```sh
whip cancel <instance>
```

Retry an eligible effect:

```sh
whip retry <instance> <effect>
```

Pause/resume are nonterminal. Cancel is terminal.

## 16. Validate Provider Boundaries

Default validation uses fixture providers. Optional real-provider smoke checks
are configured with environment variables:

```sh
WHIPPLESCRIPT_E2E_REAL_PROVIDERS=1 \
WHIPPLESCRIPT_REAL_PROVIDERS=loft,baml,codex \
scripts/check-real-providers.sh
```

For the smallest Codex smoke:

```sh
scripts/check-codex-message.sh
```

For an OpenAI-backed local BAML-compatible coerce bridge:

```sh
scripts/check-openai-coerce.sh
```

Provider failures must appear as events, run/effect terminal state, diagnostics,
and evidence. They do not automatically fail the workflow unless source rules
execute `fail`.

## 17. Authoring Checklist

Before treating a workflow as ready:

- `whip check <file>` succeeds.
- `whip compile <file>` snapshot is stable and understandable.
- Effects are ordered with `after`, not source order.
- Provider/model routing is deterministic source metadata, not an LLM decision.
- Agent profiles are as narrow as possible.
- Plugin capabilities are explicit `call` effects.
- Skills are attached to agents or turns.
- Terminal behavior is explicit through `complete`, `fail`, or operator cancel.
- Failure branches either retry, escalate, or intentionally ignore failures.
- `whip dev <file> --json` passes assertions with fixture providers.
- `whip trace <instance> --check --json` reports conformance.

## 18. Common Mistakes

| Mistake | Fix |
| --- | --- |
| Relying on source order for effect sequencing. | Use `after effect succeeds/fails/completes`. |
| Asking a model to choose provider identity. | Use `AgentRef<...>`, enums, or literal source fields. |
| Treating `coerce` as a local function. | Branch on the `baml.coerce` effect completion. |
| Importing skills with `use`. | Attach skills to agents or turns. |
| Hiding orchestration in shell scripts. | Represent work as rules, facts, and effects. |
| Storing credentials in source. | Use provider/runtime configuration references. |
| Reading effect output outside an `after` branch. | Bind output inside the terminal branch. |
| Treating provider failure as workflow failure. | Add a rule that chooses to `fail` when policy requires it. |
