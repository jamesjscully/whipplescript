# Tutorial: Route And Review Agent Work

This tutorial runs a fixture-backed workflow that sends language tasks to three
logical agents, reviews each result with a typed model-decision effect, and
records inspectable facts.

It does not require Codex, Claude, Pi, BAML, Loft, or external credentials. The
fixture provider stands in for real providers so you can see the orchestration
model first.

## What You Will Run

The tutorial uses
[`examples/provider-language-e2e.whip`](../examples/provider-language-e2e.whip).
The workflow does four things:

1. Seeds six language tasks.
2. Routes each task to a logical provider agent: `codex`, `claude`, or `pi`.
3. Reviews each completed turn with `reviewLanguageArtifact`.
4. Records a `LanguageE2EResult` fact for each reviewed task.

The workflow also has assertions that check the run reached the expected state:
two reviewed results per provider, no queued tasks left, six completed
`agent.tell` effects, and six completed `baml.coerce` effects.

## 1. Install And Check

From a checkout:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
whip doctor
```

If you already have a checkout and prefer not to install, replace `whip` in the
commands below with:

```sh
cargo run -p whipplescript --
```

## 2. Read The Workflow Shape

The workflow declares task and result facts:

```whip
class LanguageTask {
  provider AgentRef<codex | claude | pi>
  language string
  expectedScript string
  prompt string
  artifactPath string
  status "queued"
}

class LanguageE2EResult {
  provider string
  language string
  artifactPath string
  turn AgentTurn
  review LanguageQualityReview
  status "reviewed"
}
```

It declares three logical agents:

```whip
agent codex {
  profile "repo-writer"
  capacity 2
}

agent claude {
  profile "repo-writer"
  capacity 2
}

agent pi {
  profile "repo-writer"
  capacity 2
}
```

Then one rule routes each queued task to the task's selected agent:

```whip
rule run_language_task
  when LanguageTask as task where task.status == "queued"
  when task.provider is available
=> {
  tell task.provider as turn """
  Complete this language e2e task.

  Target language:
  {{ task.language }}

  Task:
  {{ task.prompt }}
  """

  after turn succeeds {
    coerce reviewLanguageArtifact(task.language, task.expectedScript, task.artifactPath, turn.summary) as review
  }

  after review succeeds {
    done task

    record LanguageE2EResult {
      provider task.provider
      language task.language
      artifactPath task.artifactPath
      turn turn
      review review
      status "reviewed"
    }
  }
}
```

The important point is that agent work and review are durable effects. They do
not run inside the rule commit. The runtime records the request, a worker
executes it, and later rules continue from the effect result.

## 3. Check The Workflow

```sh
whip check examples/provider-language-e2e.whip
```

You should see the compiled workflow summary with agents, assertions, and rules.
The exact hashes may differ, but the command should exit successfully.

## 4. Run With The Fixture Provider

Use `dev` for local validation:

```sh
mkdir -p .whipplescript
whip --store .whipplescript/tutorial.sqlite \
  dev examples/provider-language-e2e.whip \
  --provider fixture \
  --until idle \
  --json
```

Save the returned `instance_id`.

A successful run has six passing assertions. The JSON is verbose, but the shape
to look for is:

```json
{
  "workflow": "ProviderLanguageE2E",
  "assertions": [
    {"status": "passed"},
    {"status": "passed"}
  ],
  "steps": [
    {"committed_rules": 7, "facts_created": 6, "effects_created": 6},
    {"committed_rules": 6, "facts_created": 0, "effects_created": 6},
    {"committed_rules": 6, "facts_created": 6, "effects_created": 0}
  ],
  "workers": [
    {"provider": "fixture", "ran_effects": 6},
    {"provider": "fixture", "ran_effects": 6}
  ]
}
```

## 5. Inspect Status

```sh
whip --store .whipplescript/tutorial.sqlite status <instance_id>
```

Expected shape:

```text
instance ins_... running
facts=24 queued_effects=0 blocked_effects=0 active_runs=0 failures=0
recent events:
  #... assertion.passed source=assertion
```

The instance is still `running` because this example validates assertions but
does not declare a workflow terminal `complete` action. That is fine for this
tutorial; the important part is that all expected work has become durable facts
and completed effects.

## 6. Inspect Facts

```sh
whip --store .whipplescript/tutorial.sqlite facts <instance_id>
```

You should see six `LanguageE2EResult` facts, plus projected completion facts.
One result looks like:

```text
LanguageE2EResult ... {
  "artifactPath":"target/dogfood/language/codex-french.txt",
  "language":"French",
  "provider":"codex",
  "review":{
    "confidence":0.75,
    "isTargetLanguage":true,
    "isWellFormed":true,
    "usesExpectedScript":true
  },
  "status":"reviewed"
}
```

This is the payoff: after the workflow runs, you can ask the runtime what
happened without reconstructing it from a chat transcript.

## 7. Inspect Effects

```sh
whip --store .whipplescript/tutorial.sqlite effects <instance_id>
```

Expected shape:

```text
key_... agent.tell status=completed target=codex profile=repo-writer
key_... agent.tell status=completed target=claude profile=repo-writer
key_... agent.tell status=completed target=pi profile=repo-writer
key_... baml.coerce status=completed target=reviewLanguageArtifact
```

The `agent.tell` rows are the routed agent turns. The `baml.coerce` rows are the
typed review decisions. With real providers, these same effect records are where
you would inspect provider status, failures, retries, and evidence.

## 8. What To Change Next

Try editing `examples/provider-language-e2e.whip`:

- Add another `LanguageTask` in `seed_language_matrix`.
- Change an agent capacity.
- Change an assertion and watch `dev` report the mismatch.
- Add a new result field to `LanguageE2EResult`.

Then run:

```sh
whip check examples/provider-language-e2e.whip
whip --store .whipplescript/tutorial.sqlite \
  dev examples/provider-language-e2e.whip \
  --provider fixture \
  --until idle
```

## Where To Go Next

- [Concepts](concepts.md): the core terms behind this tutorial.
- [Language Reference](language-reference.md): syntax and authoring details.
- [Runtime And Operations Reference](runtime-operations.md): stores, effects,
  workers, providers, failures, and inspection.
- [Providers And Plugins](providers.md): how fixture and real providers fit.
