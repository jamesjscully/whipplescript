# Language Sketch

Status: exploratory

The language should feel close to rewrite rules:

```text
when facts are present
and guards are true
=> produce facts and durable effects
```

It should avoid table boilerplate, explicit turn schemas, and hidden
coordination nouns. Built-in orchestration facts cover work, agents, turns,
capacity, attempts, and effect lifecycle. The control plane and runtime store
provide those facts; the language defines policy over them.

## Example Shape

```whippletree
workflow SpecImplementation

agent worker {
  profile "repo-writer"
  capacity 5
  capabilities ["agent.tell", "repo.write"]
}

agent reviewer {
  profile "repo-reader"
  capacity 1
  capabilities ["agent.tell", "repo.read"]
}

rule discover_ready_work
  when work is open as item
  when item.dependencies are done
=> {
  ready item
}

rule implement
  when ready work as item
  when worker is available
=> {
  tell worker """
  Implement this work item:

  {{ item.goal }}

  Stay within:
  {{ item.files }}
  """
}

rule review
  when worker completed work as item
=> {
  tell reviewer """
  Review this work item:

  {{ item.goal }}

  Check correctness, tests, and scope.
  """
}

rule accept
  when reviewer accepted work as item
=> {
  complete item
}

rule retry
  when reviewer rejected work as item
  when item.attempts < 3
=> {
  ready item
}

rule escalate
  when worker failed work as item
  when item.attempts >= 3
=> {
  askHuman """
  This work item failed three times:

  {{ item.goal }}

  Please clarify, split, or cancel it.
  """
}
```

`tell worker` lowers to an `agent.tell` effect. It does not synchronously run
the provider. The runtime creates a durable effect, the harness executes it, and
the completion returns as facts/events that other rules can match.

If a rule produces multiple effects, they are unordered unless the source uses
explicit dependency syntax. Source order does not imply execution order.

## Built-In Concepts

The author should not define these in ordinary workflows:

- `work`
- `agent`
- `turn`
- `attempt`
- `available`
- `completed`
- `failed`
- `accepted`
- `rejected`
- `blocked`

The compiler lowers conversational phrases into typed facts and effects.

Core integrations also provide source-level affordances for common workflow
boundaries:

```text
loft has ready issue
claim issue with loft
askHuman
coerce
attach skill
```

These must remain visibly causal. A phrase may be friendly, but if it changes
durable state or touches the world, the lowering must be explainable as facts
and effects.

## External Effects

Language constructs that touch the world lower into effect categories defined in
[effects-and-capabilities.md](effects-and-capabilities.md):

```text
tell       -> agent.tell
askHuman   -> human.ask
coerce     -> baml.coerce
emit       -> event.emit
call       -> capability.call
```

Registered plugins may provide additional namespaced effects, but not new
control-flow semantics. For example, a memory plugin may provide
`memory.query`; a Thoth plugin may provide `thoth.verify`. Rules still compose
those through ordinary durable effects and completion facts.

Every effect has an idempotency key, required capabilities, and a completion
contract.

The runtime enforces effect authority through a durable registry:

- capability schemas define the authority name and input contract
- effect providers bind effect kinds to executable providers
- profiles describe allowed capability sets and whether enforcement is strict
- capability bindings grant a program access to a provider for a capability

The built-in registry ships `permissive`, `repo-reader`, `repo-writer`,
`internet-research`, and `human-review` profiles. If an effect requests a
capability that is not registered, not bound, or not allowed by its profile, the
effect is blocked before a provider run starts. The block reason is written to
the event log and effect projection, so `status` and trace-conformance checks can
explain why no worker was started.

Plugins are loaded as manifests that register capability schemas, effect
providers, optional profiles, and optional bindings. They extend the registry
but do not receive mutable access to kernel state or control-flow semantics.

All fact payloads, effect payloads, and `coerce` signatures use the type system
defined in [type-system.md](type-system.md). Whippletree supports BAML-compatible
boundary types, but only a small pure expression kernel. It should not grow
loops, collection pipelines, numeric libraries, or media manipulation.

## Parser Strategy

The initial compiler uses a hand-written lexer/parser in
`crates/whippletree-parser`. The grammar is still settling, so this keeps the
parser easy to adjust while preserving the properties the compiler needs:

```text
byte-accurate source spans
recoverable diagnostics
raw rule/effect block preservation
typed top-level syntax nodes for workflow, skills, schemas, agents, and rules
```

The parser should only become generator-backed if the grammar stabilizes enough
that generated parse tables are easier to maintain than direct Rust code.

`whip check` renders diagnostics with source excerpts, caret underlines,
and next-step help where the compiler can identify a likely fix. The parser
crate also exposes formatter scaffolding that canonicalizes declarations while
preserving rule and coerce block bodies for later lowering work.

## Facts

Facts are durable workflow memory. Rules match facts in `when` clauses and
produce facts with `record`.

Every fact has provenance: runtime, rule-recorded, effect completion, external
projection, or plugin projection. See
[fact-provenance.md](fact-provenance.md).

Typed fact declarations use classes:

```whippletree
class ReviewedWork {
  turn AgentTurn
  review WorkReview
}
```

Producing a fact:

```whippletree
record ReviewedWork {
  turn turn
  review review
}
```

Matching a fact:

```whippletree
when ReviewedWork as reviewed
when reviewed.review.status == Accept
```

Rules may also attach pure guards to typed fact matches. Guards are evaluated by
the deterministic stepper before any fact or effect is committed:

```whippletree
class LanguageTask {
  provider "codex" | "claude" | "pi"
  language string
  expectedScript string
  prompt string
  artifactPath string
  status "queued"
}

rule run_codex_language_task
  when LanguageTask as task where task.provider == "codex"
  when codex is available
=> {
  tell codex as turn """
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
}
```

The guard language must stay small and deterministic. It may read matched fact
fields, literals, enums, null, booleans, scalar comparisons, membership, and
presence checks. It must not call providers, `coerce`, plugins, host-language
functions, or string parsers. If a workflow needs semantic judgment, that
judgment belongs in an explicit effect such as `coerce` or `call
validator.checkScript`, and the resulting typed fact can be matched by a later
rule.

Guards are the preferred way to express routing over a shared schema. Authors
should not need one schema per provider when the data shape is identical and
only a literal or enum field selects the target.

### Consumed Facts

Rules can finish a matched work item without mutating it in place:

```whip
rule finish
  when Task as task where task.status == "queued"
=> {
  consume task
  record Done {
    status "done"
  }
}
```

`consume binding` marks the fact bound by `when Class as binding` as consumed in
the same rule commit. `done binding` is accepted as an alias for the same
transition. Consumed facts remain in the append-only event log and replay
history, but they leave the current fact projection: later rule matching,
projection queries, and assertions do not see them by default. This is the
first-class queue-item/done transition for rules that dispatch work, record a
replacement fact, or otherwise need to avoid matching the same fact forever.

Only fact bindings introduced by `when` clauses are consumable. Effect-output
bindings are terminal observations and must be handled with `after` blocks
instead.

### Expression Parser Coverage

The source expression parser covers guards, assertions, projection filters,
matrix rows, typed effect arguments, interpolation paths, and branch guards with
one deterministic expression kernel. Each surface must parse to the same typed
IR nodes so validation, snapshots, Maude checks, and runtime evaluation do not
grow separate dialects.

| Surface | Accepted expression forms | Notes |
| --- | --- | --- |
| `when Fact as x where <expr>` | paths, literals, `null`, booleans, `!`, `&&`, `||`, comparisons, membership, `exists path`, `empty(expr)`, `count(query)`, arrays, map indexes | Result must be boolean. Guard `false` means non-match; guard `Error` is diagnostic and no commit. |
| Top-level `assert <expr>` | all guard forms plus fact/effect projection queries | Result must be boolean. Assertions are read-only checkpoints over committed facts/effects. |
| `Class where <expr>` | field paths rooted at the projected class alias, comparisons, booleans, membership, presence, map indexes | Projection filters are pure reads and cannot enqueue effects or call providers. |
| `effect kind K where <expr>` | effect status/kind/profile/output paths, comparisons, booleans, membership, presence, map indexes | Output paths must respect completion status and terminal-output union tags. |
| Static matrix rows | typed literals, arrays, records in schema context, enum/literal values, `AgentRef` values | Matrix rows are compile-time seed data, not runtime loops. |
| Effect and `record` arguments | typed paths, literals, arrays, records in expected schema context, `AgentRef` values | Arguments must satisfy the declared payload or fact schema before lowering. |
| Interpolation paths | field paths, optional-present paths after proof, map indexes | Interpolation is path-oriented; it does not admit arbitrary provider calls or string parsing. |
| `case expr` scrutinees and branch guards | finite-domain enum/literal/optional/tagged-union values and ordinary boolean branch guards | Branch guards reuse the same parser and evaluator as `where` guards. |

Golden IR fixtures should exercise every row with stable snapshots for source
span preservation, precedence, query reads, dynamic `AgentRef` values, and
runtime-visible `Missing` versus `null` behavior. The fixtures should include
both concise examples and one provider-language dogfood workflow that routes
through deterministic metadata instead of model judgment. The checked fixtures
now include `examples/expression-kernel-dogfood.whip`,
`examples/provider-language-e2e.whip`,
`examples/terminal-output-union.whip`, and
`examples/companion-skill-dogfood.whip`.

## Pattern Branches

Whippletree supports only typed finite-domain pattern matching. The feature is
for deterministic branching over workflow data, not for general destructuring or
string parsing.

The v0 pattern surface is:

```text
enum variants
literal-union values
optional Some/None or present/missing branches
tagged terminal-output unions from effect completions
explicit wildcard/default branches
```

Pattern branches may have `where` guards, and those guards use the expression
kernel. Branch guards cannot call providers, invoke `coerce`, read files, parse
strings, or perform I/O.

Example finite enum branch:

```whippletree
case review.status {
  Accept => record AcceptedWork {
    review review
  }
  Revise => record RevisionNeeded {
    review review
  }
  Blocked => askHuman """
    This review is blocked:
    {{ review.reason }}
  """
}
```

Example optional branch:

```whippletree
case issue.assignee {
  Some assignee => tell reviewer """
    Review {{ issue.title }} for {{ assignee.name }}.
  """
  None => askHuman """
    Assign an owner before this issue can continue.
  """
}
```

Example tagged terminal-output branch:

```whippletree
case turn.output {
  Completed result where exists result.artifactPath => record LanguageArtifact {
    path result.artifactPath
    summary result.summary
  }
  Failed failure => record ProviderFailure {
    reason failure.reason
  }
  Blocked block => askHuman """
    Provider run was blocked:
    {{ block.reason }}
  """
}
```

The concrete branch syntax may still change as implementation lands. The
semantic requirements are fixed:

- variants must belong to the scrutinee's enum or literal-union domain
- optional `Some` branches bind a proven-present value
- `None` branches cannot read fields through the missing value
- tagged terminal-output branches must match declared completion tags and expose
  only fields valid for that tag
- finite domains are exhaustiveness-checked when a total result is required
- non-matching branches do not commit facts or effects
- exhaustive finite-domain misses produce diagnostics, not hidden fallthrough

Deferred until there is a concrete workflow need:

```text
deep object destructuring
array/list destructuring
user-defined extractors
regex/string pattern matching
provider transcript pattern matching
```

`record` is the source-level marker for durable fact production. It is not
assignment and not an inline local variable. If a rule commits, recorded facts
commit atomically with any effect graph nodes and dependency edges produced by
the same rule.

Fact construction must satisfy the declared class schema. Unknown fields are
errors. Missing required fields are errors. Optional fields may be omitted or
set to `null`.

Conversational fact sugar is allowed for core integrations:

```whippletree
when loft has ready issue as issue
when worker is available
```

But sugar must lower to typed fact queries. Source text should not invent hidden
workflow state.

## Correlation

Agent turns and effect outputs must carry enough correlation to avoid relying
on prompt text.

When an effect is created from a typed object, the runtime records correlation
metadata:

```text
effect_id
rule_name
source fact ids
input object ids
dependency outputs used
logical agent
capability/effect kind
```

Examples:

```whippletree
claim issue with loft as claim

after claim succeeds {
  tell worker """
  Implement {{ claim.issue.title }}
  """
}
```

The downstream `agent.tell` effect is correlated with the `loft.claim` output
and the claimed issue. Later completion facts can therefore support patterns
like:

```whippletree
when worker completed turn for loft issue as turn
```

without asking the compiler to infer meaning from prompt text.

## Agent Routing

Agent targets are workflow-owned routing decisions, not model outputs. A rule
must identify the target agent deterministically through either:

- a literal declared agent name, such as `tell codex`
- a typed agent reference whose value is proven to be one of the workflow's
  declared agents
- a registered routing capability that returns a typed, auditable route before
  any provider turn is enqueued

The runtime must never ask a language model to decide which provider is being
tested, which model is active, or which logical agent should receive a turn.
Those values may be copied into result/audit facts by rule literals or typed
metadata, but they should not be fields in BAML review output unless the review
is explicitly about verifying observed provider evidence.

Dynamic agent routing is typed:

```whippletree
class LanguageTask {
  provider AgentRef<codex | claude | pi>
  language string
  expectedScript string
  prompt string
  artifactPath string
  status "queued"
}

rule run_language_task
  when LanguageTask as task
  when task.provider is available
=> {
  tell task.provider requires ["agent.tell"] as turn """
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """
}
```

The compiler rejects `tell` targets that are plain strings or non-`AgentRef`
dynamic fields. Runtime lowering resolves the `AgentRef` value from the matched
fact before enqueuing the `agent.tell` effect, so effect targets and profiles are
chosen deterministically before any provider starts.

`agent` declarations may include a finite `capabilities [...]` list. A `tell`
statement may declare the target capability contract with `requires [...]`; for
dynamic `AgentRef` targets, every possible target in the refined domain must
declare every required capability. The runtime repeats the same check against
the program-version agent metadata before claiming or starting a provider run,
so externally inserted or replayed effects cannot bypass source validation.

## Reuse And Matrices

Dogfood workflows often need a deterministic matrix: providers x languages,
phases x reviewers, or fixtures x validators. The language should provide a
source-level way to seed small static matrices without hiding effects:

```whippletree
matrix language_tasks as LanguageTask [
  { provider "codex", language "French", expectedScript "Latin" },
  { provider "claude", language "Hindi", expectedScript "Devanagari" },
  { provider "pi", language "Japanese", expectedScript "Kana and kanji" },
]
```

Matrix rows lower to ordinary `record` writes during rule evaluation. They must
be fully typed and deterministic; they are not loops over runtime collections.
Until matrix syntax lands, dogfood workflows should seed the equivalent typed
facts explicitly in a `when started` rule. That is the shape used by
`provider-language-e2e.whip` and `companion-skill-dogfood.whip`.

Repeated effect chains should be reusable without obscuring the durable graph.
A rule template or action block may abstract identical `tell -> coerce ->
record` shapes only if expansion is static and inspectable in the compiled IR:

```whippletree
action run_language_task(agent AgentRef, task LanguageTask, provider string) {
  tell agent as turn """
  Write {{ task.language }} text to {{ task.artifactPath }}.
  """

  after turn succeeds {
    coerce reviewLanguageArtifact(task.language, task.expectedScript, task.artifactPath, turn.summary) as review
  }

  after review succeeds {
    record LanguageE2EResult {
      provider provider
      language task.language
      artifactPath task.artifactPath
      turn turn
      review review
      status "reviewed"
    }
  }
}
```

This is syntactic reuse, not a general function system. Expansion must preserve
source spans, idempotency keys, dependencies, and effect/fact provenance.

## Deterministic Assertions

Workflows and e2e tests need first-class deterministic assertions over facts and
effects so CI can check the intended orchestration without relying on provider
wording:

```whippletree
assert count(LanguageE2EResult where provider == "codex") == 2
assert exists(LanguageE2EResult where language == "Japanese")
assert count(effect kind agent.tell where status == completed) == 6
assert count(effect kind baml.coerce where status == completed) == 6
```

Assertions are read-only and run after stepping or at named checkpoints. Failed
assertions should produce diagnostics and trace evidence, not partial workflow
state.

## Dependent Effects

Use `after` when one effect must wait for another:

```whippletree
rule implement_claimed_issue
  when loft has ready issue as issue
  when worker is available
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """
    Implement {{ claim.issue.title }}
    """
  }
}
```

`after` compiles to durable effect dependency edges. It is not a callback, not a
subroutine, and not general control flow.

Allowed v0 predicates:

```text
succeeds
fails
completes
```

Effect outputs are available only after the matching dependency predicate is
satisfied. The compiler rejects use of `claim.issue.title` outside the
`after claim succeeds` scope.

Joins should be expressed as normal rules over completion facts, not as nested
effect graph syntax.

## Coerce

`coerce` should read like a typed model decision, but it is semantically
asynchronous and durable:

```whippletree
rule classify
  when worker completed work as item
=> {
  coerce classifyWork(item.summary) as classification
}

rule accept
  when classification.status is Accepted for work as item
=> {
  complete item
}
```

The first rule requests the BAML call. The second rule reacts when the typed
coerce output has arrived. See [coerce.md](coerce.md).

## Design Pressure

The syntax must stay honest. If a construct changes durable state or enqueues
an effect, it should be visible. Conversational syntax is good only when it maps
to a small, explainable rewrite.

Bad direction:

```whippletree
manage team until done
```

Good direction:

```whippletree
when ready work as item
when worker is available
=> tell worker item
```

The second form is friendly but still exposes the causal edge.
