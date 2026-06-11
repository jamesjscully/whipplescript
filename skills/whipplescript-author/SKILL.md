# WhippleScript Author

Use this skill when authoring, reviewing, or operating `.whip` workflows.

## Mental Model

WhippleScript is a restricted event-sourced rule machine.

- Rules match durable facts and write new facts or durable effects.
- Effects do not run inline. Providers claim and complete them later.
- Source order does not sequence effects. Use `after <effect> succeeds`,
  `after <effect> fails`, or `after <effect> completes`.
- Every effect must be auditable through status, events, evidence, and trace.
- Workflow revision is a control-plane operation. Source rules may propose
  candidate patch artifacts, but they must not activate a running revision.
- Prefer Loft for project work, BAML `coerce` for typed model decisions, and
  plugins for memory or external systems.

## Valid Minimal Workflow

```whipplescript
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
whip --json run examples/minimal-noop.whip
```

## Common Patterns

Claim a ready work item before an agent turn (declare `queue backlog {
tracker builtin }`):

```whipplescript
rule work_ready_item
  when backlog has ready item as item
  when worker is available
=> {
  claim item as lease

  after lease succeeds {
    tell worker "Implement {{ item.title }}"
  }

  after turn succeeds as outcome {
    finish item { summary outcome.summary }
  }
}
```

Sequential work reads better as a `flow`, which lowers to rules:

```whipplescript
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

BAML classification with human fallback:

```whipplescript
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

```whipplescript
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

Typed dynamic agent routing:

```whipplescript
class LanguageTask {
  provider AgentRef<codex | claude | pi>
  language string
  artifactPath string
  status "queued"
}

rule run_language_task
  when LanguageTask as task
  when task.provider is available
=> {
  tell task.provider as turn """markdown
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
}
```

Use `AgentRef<...>` when the workflow data selects a logical agent. The value is
source metadata, not a model decision. Do not ask BAML or a provider prompt to
choose the provider, model, or route; only include provider fields in model
outputs when reviewing observed provider evidence.

Static validation table:

```whipplescript
table language_tasks as LanguageTask [
  {
    provider codex
    language "French"
    artifactPath "target/dogfood/language/codex-french.txt"
    status "queued"
  }

  {
    provider claude
    language "Hindi"
    artifactPath "target/dogfood/language/claude-hindi.txt"
    status "queued"
  }
]
```

Use `table` for small deterministic seed data such as providers x languages or
phases x reviewers. Table rows are typed and compile to ordinary `record`
writes on `started`; they are not runtime loops and must not hide provider
calls, model decisions, or external lookups.

Tags for validation metadata:

```whipplescript
@fixture
@acceptance
workflow ProviderLanguageE2E

@acceptance
assert count(LanguageTask where status == "queued") == 0
```

Use tags to mark fixture-backed, acceptance, release-gate, slow, or provider
specific source. Tags are metadata only; do not rely on them for routing,
authority, rule readiness, or provider behavior.

Descriptions for report prose:

```whipplescript
@fixture
description "Fixture-backed provider x language acceptance workflow"
workflow ProviderLanguageE2E

description "Route one queued task to its selected provider"
rule run_language_task
  when LanguageTask as task
=> {
  done task
}
```

Use descriptions on workflows, tables, assertions, and rules when reports need
human-readable labels. Descriptions are metadata only and cannot attach to
schemas, agents, harnesses, coerces, includes, plugins, workflow contracts,
patterns, or applications.

Content-typed prompt metadata:

```whipplescript
tell worker as turn """markdown
Summarize the result.
"""

askHuman """application/json
{
  "question": "Approve this release?"
}
"""
```

Use a content type on multiline `tell`, `askHuman`, or `coerce` prompts when
reports or renderers should know how to display the body. Treat it as metadata
only: it does not validate JSON, change model behavior, or choose a provider.
In `dev --json` reports, matched effect reads surface the preserved
`prompt_content_type` when assertions read those effects.

Fan-out with a visible tracker:

```whipplescript
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
  tell reviewer as turn """markdown
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

Companion-skill validation routing:

```whipplescript
class ReviewTask {
  reviewer AgentRef<codex | claude | pi>
  phase int
  trackerPath string
  route "spec" | "validation" | "docs"
  status "queued"
}

rule dispatch_review
  when ReviewTask as task where task.route in ["spec", "validation", "docs"]
  when task.reviewer is available
=> {
  tell task.reviewer requires ["agent.tell"] as turn """markdown
  Use the whipplescript-author companion skill.
  Review phase {{ task.phase }} and update {{ task.trackerPath }}.

  Do not infer provider, model, or route identity from prompt text; the workflow
  selected the logical reviewer through typed AgentRef metadata.
  """
}
```

Use this shape for provider/language/phased-review tables. One shared task
schema plus `AgentRef` metadata is preferred over one class per provider. Source
assertions should check dispatch/result counts; BAML reviews should judge the
artifact or evidence, not choose the route.

## Revision Guidance

When a workflow needs to adapt itself or repair a running instance, model that
as ordinary work:

- produce a candidate `.whip` file or patch artifact through `tell`, `coerce`,
  `call`, `invoke`, or `askHuman`
- record where the proposal lives and why it was produced
- ask a human or operator workflow to review the proposal when appropriate
- instruct the operator to run `whip revise --dry-run` first

Do not write `.whip` source that pretends to call `revise` from a rule body.
Activation must happen from the control plane:

```sh
whip revise <instance> candidate.whip --root Workflow --dry-run
whip revise <instance> candidate.whip --root Workflow --cancel keep
```

Use `--cancel keep` when old effects should finish, `--cancel queued` when
queued old effects should be terminal-cancelled, and `--cancel running` when
running providers should receive cancellation requests.

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

Guards are supported for deterministic routing over the expression kernel:

```whipplescript
when Review as review where review.status == Accept
```

Keep guards over typed facts and deterministic values: booleans, comparisons,
membership, presence checks, projections, map indexes, and count/exists/empty
queries. Use BAML or a registered capability only when semantic judgment is
required.

Put `as binding` on the effect line:

```whipplescript
tell worker "Do it" as turn
```

Do not put `as turn` after a closing multiline string delimiter.

Use generic plugin calls for now:

```whipplescript
call memory.query for item as context
```

Do not invent plugin-specific control-flow syntax.

## Debug Workflow

Use these commands first:

```sh
whip doctor
whip check workflow.whip
whip check --model-search workflow.whip
whip --json run workflow.whip
whip --json dev workflow.whip --provider fixture --until idle
whip dev workflow.whip --provider fixture --until idle --stream ndjson
whip --json dev workflow.whip --provider fixture --until idle --include-tag acceptance
whip --json accept workflow.accept.json
whip status <instance>
whip effects <instance>
whip runs <instance>
whip --json evidence <instance>
whip --json diagnostics <instance>
whip --json trace <instance> --check
```

When writing `.accept.json` fixtures, keep `workflow` and
`provider_config_paths` relative to the fixture file when possible. Use
`input` for ordinary workflow start input and `setup.facts` for typed external
setup facts validated against declared class schemas. Use `setup.inbox` for
pre-existing human review items; do not fake those as workflow facts. Use
`expect.assertion_tags` and `expect.assertion_untagged` for executable-spec groups,
`expect.source_metadata` for source tag/description targets,
`expect.diagnostics_by_code` for durable diagnostic checks, `expect.summary`
for total final active fact/effect counts, and `expect.facts` or
`expect.effects` for grouped counts. Use `expect.runs` when provider run status
or artifact count is part of the contract, and `expect.artifacts` for
metadata-only artifact counts by kind/MIME type. Use `expect.evidence` for
metadata-only evidence counts by kind/subject type. Use `expect.assertion_reads`
for deterministic assertion projections; effect match groups can assert prompt
content type, trace/evidence link counts, and stable `trace_sequences`. Each
assertion-read expectation must include at least one selector: `source`, `kind`,
`head`, or `guard`.
Observed `evidence_ids` are useful drilldown links, but avoid pinning them unless
the ids are deliberately stable. Use `expect.trace.items` only for stable
abstract trace records, such as sequence/type/status, not for incidental runtime
ids; each trace item expectation must include at least one selector. Fixture and
expectation fields are shape-checked before a workflow starts, so wrong-typed
expectations are rejected rather than treated as absent. `setup.effects` and
`setup.artifacts` are rejected in v0. `dev` and `accept` reports include compact
artifact/evidence item links but omit raw artifact paths, content, and evidence
payloads. `dev --stream ndjson` includes batched `dev.events` runtime event
deltas and a `dev.assertions` event before the final report. Run suites by
invoking one fixture per command with an isolated store per fixture.

If an effect does not run, check its status and policy block reason before
changing prompts. Prefer the single `dev --json` report for local validation:
it includes source metadata, assertion groups, deterministic reads, durable
diagnostics, assertion event/diagnostic links, and table provenance.

## Safety

- Never hide orchestration in shell scripts or prompt text.
- Never rely on source order for effect sequencing.
- Do not silently inject memory. Query it as an effect and preserve evidence.
- Do not self-modify live instances from source rules. Propose a patch artifact
  and let the control plane activate it with `whip revise`.
- Keep provider credentials outside workflow source.
- Use human review for destructive or ambiguous steps.
