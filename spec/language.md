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
}

agent reviewer {
  profile "repo-reader"
  capacity 1
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
