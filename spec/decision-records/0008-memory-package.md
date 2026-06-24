# 0008: Memory Package

Status: accepted design baseline

## Decision

WhippleScript should provide `std.memory` as a first-party standard package for
explicit agent memory. Memory is not a hidden model feature, an ambient provider
setting, or a mandatory ontology. It is a workflow-controlled way to give agents
relevant context from large durable memory pools exactly when they need it, and
to let new memories enter those pools under explicit policy.

The source-level category is a **memory pool**:

```text
memory pool = named durable place where memories live
memory      = an individual remembered note, lesson, preference, summary, etc.
```

The core package vocabulary should stay small:

```text
memory pool       declare a named memory pool
recall from       workflow reads a pool into an agent-usable context bundle
learn from into   workflow writes memory material into a pool
with access to    grant an agent turn bounded memory tools
curate            manage pool health under pool policy
keep              promote or preserve proposed memory
forget            discard proposed memory or retire active memory
```

`recall` and `learn` are deliberately asymmetric:

```text
recall = read from a memory pool into working context
learn  = add memory material to a memory pool
```

## Source Surface

### Pool Declaration

A memory pool is a declared resource:

```whip
use std.memory

memory pool project_memory {
  provider builtin

  search hybrid
  context limit 8

  learning reviewed
  retention 180 days
  capacity 10000 memories

  curation {
    dedupe
    distill
    propose forgetting
  }
}
```

The declaration names the pool and its policy. It does not inject memory into
agent turns and does not grant agents the ability to read or write the pool.

Initial policy clauses:

```text
provider <provider>         memory provider implementation
search hybrid|lexical|semantic
context limit <n>           default recall packing budget
learning direct|reviewed    whether learned material becomes active directly
retention <duration>        default retention horizon
capacity <n> memories       management threshold, not a hard parser limit
curation { ... }            allowed curation strategies
```

The exact search implementation is provider policy. `search hybrid` says the
author wants hybrid retrieval behavior; it does not expose BM25, embedding
tables, chunking, or vector-index maintenance as workflow concepts.

### Workflow-Managed Recall

When a workflow knows an agent needs memory before a turn starts, it recalls from
the pool explicitly:

```whip
recall from project_memory for issue as recalled

after recalled succeeds as context {
  tell coder with context context """markdown
  Work this issue:

  {{ issue.title }}
  """
}
```

Meaning:

```text
Given this memory pool and this subject, prepare a bounded context bundle of
relevant memories for downstream use.
```

The result is `MemoryContext`, an agent-usable context artifact plus evidence.
It is not a raw database cursor and not ambient model memory.

A more precise recall may use a block:

```whip
recall from project_memory for issue {
  query issue.title
  query issue.body
  context limit 12
} as recalled
```

Block clauses refine retrieval intent. They do not choose a specific vector
index or retrieval table.

### Workflow-Managed Learning

When a workflow deterministically decides that some experience should become
memory material, it learns from that experience into a pool:

```whip
learn from turn into project_memory for issue {
  note turn.summary
} as learned
```

Meaning:

```text
Take memory material from this source, associate it with this subject, and
submit it to the pool's learning policy.
```

`from` is provenance: the experience, artifact, message, signal, turn, or typed
value the memory material came from.

`into` is destination: the memory pool.

`for` is subject: what future recalls should understand this memory to be about.

The `for` subject should usually be present, but providers may allow pool-wide
learning for material that is not about a specific issue, file, project object,
or agent turn.

If the pool policy is:

```text
learning direct
```

the learned material may become active memory immediately.

If the pool policy is:

```text
learning reviewed
```

the learned material becomes proposed memory until it is kept or forgotten.

The source form is the same in both cases. Pool policy decides whether learning
is direct or reviewed.

### Agent-Managed Memory Access

Sometimes the workflow should not precompute every relevant context bundle. An
agent may discover during a turn that it needs memory, or may notice something
worth saving. That should be explicit, bounded, and turn-scoped:

```whip
tell coder
  with access to project_memory {
    recall for issue
    learn for issue
  }
"""markdown
Implement this issue. Use memory when it is relevant, and record durable lessons.
"""
```

`with access to` is a grant clause, not an executable block. It means:

```text
during this agent turn, expose memory tools backed by project_memory
the agent may recall memories about issue
the agent may submit memory material about issue
all access is bounded by the grant, pool policy, provider capability, and agent
profile
```

This is distinct from preselected context:

```whip
tell coder with context context "..."
```

`with context` passes already-selected material into the turn. `with access to`
grants controlled tools the agent may choose to use during the turn.

The grant is not ambient. An agent without an explicit memory grant has only the
ordinary prompt, attached skills/context, and provider-native tools allowed by
its profile.

Multiple pools require multiple explicit grants:

```whip
tell researcher
  with access to project_memory {
    recall for issue
  }
  with access to org_memory {
    recall for issue
  }
"""markdown
Research the issue with the allowed memory pools.
"""
```

### Curation

Memory pools grow. A useful memory package must support pool management without
turning ordinary workflows into database administration.

`curate` runs the pool's curation policy:

```whip
curate project_memory as curation
```

or:

```whip
curate project_memory {
  reason "pool over capacity"
  focus stale
} as curation
```

Meaning:

```text
review pool health under its declared policy and produce allowed changes or
proposals.
```

Curation may:

```text
deduplicate overlapping memories
distill many noisy memories into fewer durable memories
surface conflicts
archive old low-value memories
propose forgetting obsolete memories
repair retrieval quality when provider projections are stale
```

Distillation is a curation strategy, not the primary workflow verb:

```text
curate  = manage pool health
distill = synthesize many memories or experiences into fewer better memories
```

The first source surface should prefer `curate` over a top-level `distill`
operation. If authors later need direct distillation as a common workflow action,
it can be added as a narrower operation.

### Review Projections

Reviewed learning and conservative curation expose work through projections:

```whip
rule review_proposed_memory
  when project_memory has proposed memory as proposal
=> {
  tell reviewer with context proposal "Review this proposed memory."
}
```

Keeping proposed memory:

```whip
keep proposal as kept
```

Forgetting proposed or active memory:

```whip
forget proposal {
  reason "duplicate"
} as forgotten
```

```whip
forget old_memory {
  reason "outdated"
} as forgotten
```

`keep` and `forget` are memory-domain lifecycle operations. They should not be
called `accept`, `reject`, `delete`, or `gc` in the source surface.

## Program Model

At the workflow level, `std.memory` is about placing context at the right point
in an agent workflow:

```text
before the turn   recall from pool -> pass MemoryContext with context
during the turn   grant access to pool -> agent can recall/learn through tools
after the turn    learn from turn -> pool policy stores or proposes memory
maintenance       curate pool -> keep/forget proposed changes
```

The common loop is:

```whip
rule implement_issue
  when backlog has ready issue as issue
  when coder is available
=> {
  recall from project_memory for issue as recalled

  after recalled succeeds as context {
    tell coder
      with context context
      with access to project_memory {
        recall for issue
        learn for issue
      }
      as work
    """markdown
    Implement this issue.
    """

    after work succeeds as turn {
      learn from turn into project_memory for issue {
        note turn.summary
      } as learned
    }
  }
}
```

No memory movement is hidden:

```text
pre-turn memory appears as an explicit recall and context value
during-turn memory appears as an explicit access grant and provider/tool events
post-turn memory appears as an explicit learn operation
pool management appears as explicit curation or operator action
```

## Construct Graph Contract

`std.memory` uses the ordinary package construct system. It should not become a
parser plugin or agent-provider side channel.

### `memory pool`

```text
family: declaration_block
shape: memory pool <name> { <clauses> }
provides: Resource<MemoryPool>
requires: provider kind, policy clauses, capability bindings
lowering class: metadata today; future resource_declaration
```

### `recall from`

```text
family: effect_operation
shape: recall from <pool: MemoryPoolRef> for <subject: Expr> [block] as <binding>
requires: Resource<MemoryPool>
requires: Value<Subject>
requires: Capability<memory.recall>
provides: EffectHandle<MemoryContext>
lowering class: capability_call today; future typed_effect_call
```

### `learn from into`

```text
family: effect_operation
shape: learn from <source: Expr> into <pool: MemoryPoolRef> [for <subject: Expr>] <block> as <binding>
requires: Resource<MemoryPool>
requires: Value<ProvenanceSource>
may require: Value<Subject>
requires: Capability<memory.learn>
provides: EffectHandle<MemoryLearnResult>
lowering class: capability_call today; future typed_effect_call
```

### `curate`

```text
family: effect_operation
shape: curate <pool: MemoryPoolRef> [block] as <binding>
requires: Resource<MemoryPool>
requires: Capability<memory.curate>
provides: EffectHandle<MemoryCurationResult>
lowering class: future typed_effect_call
```

### `keep` / `forget`

```text
family: effect_operation or resource_operation
shape: keep <memory-or-proposal-ref> as <binding>
shape: forget <memory-or-proposal-ref> [block] as <binding>
requires: Value<MemoryProposal | MemoryEntry>
requires: Capability<memory.keep> or Capability<memory.forget>
provides: EffectHandle<MemoryLifecycleResult>
lowering class: future typed_effect_call
```

### Turn Access Grant

The `tell` construct remains core. `std.memory` should instantiate a
core-owned turn-grant composition point:

```text
family: turn_access_grant
shape: with access to <resource> { <grant clauses> }
appears in: tell clauses
requires: Resource<MemoryPool>
requires: requested grant permissions such as recall/learn
requires: agent/provider capability to expose the tool safely
provides: TurnGrant<MemoryPoolAccess>
lowering class: agent_turn_grant
```

Grant clauses are permission declarations, not statements:

```text
recall for <subject>
learn for <subject>
```

The checker must reject a memory grant if the target agent/profile/provider
cannot expose a turn-scoped memory tool while preserving the ordinary
`agent.tell` lifecycle and evidence.

## Capability Surface

The stable capability names should describe memory-domain authority:

```text
memory.recall
memory.learn
memory.curate
memory.keep
memory.forget
```

Provider implementations may internally call older or lower-level contracts such
as `memory.query`, but the source-level package contract should prefer the
domain vocabulary above.

## Events, Evidence, And Projections

The source surface should not force authors to think in event-record shapes, but
the implementation must still map cleanly to WhippleScript's durable model:

```text
recall from     -> durable recall effect -> MemoryContext artifact + evidence
learn from into -> durable learn effect -> active or proposed memory
agent grant use -> child recall/learn effects correlated to the agent turn
curate          -> durable curation effect -> proposed or applied pool changes
keep/forget     -> durable lifecycle effects over proposed/active memory
```

Memory entries are durable pool state. Search indexes, embeddings, summaries,
and context bundles are rebuildable projections or artifacts. A provider may use
SQLite FTS, BM25, vectors, reranking, summaries, or future knowledge-engine
techniques, but those are implementation choices behind the memory provider.

Recall evidence must explain what was selected and why:

```text
pool
subject
query material
policy snapshot
candidate memories
selected memories
packing budget
search/retrieval strategy
provider and projection freshness
```

Learning and curation evidence must explain why material entered, stayed in, or
left the active memory view.

## Operator Surface

Memory management needs an operator and agent-facing CLI, but the CLI should
mirror the language concepts:

```text
whip memory pools
whip memory recall <pool> --for <subject>
whip memory learn <pool> --from <artifact-or-turn> --for <subject>
whip memory proposals <pool>
whip memory keep <proposal>
whip memory forget <memory-or-proposal>
whip memory curate <pool>
whip memory explain <recall-or-memory>
```

The exact CLI is future work. The important product boundary is that humans and
agents can inspect, curate, and explain memory without editing storage internals.

## Core/Package Boundary

Core owns:

```text
agent declarations
agent.tell effects and turn lifecycle
with context context-passing
turn-grant composition point
effect lifecycle and after branches
artifacts and evidence
capability/profile enforcement
event log and replay invariants
```

`std.memory` owns:

```text
memory pool declarations
memory provider contracts
recall, learn, curate, keep, forget operations
memory access grant schema
memory provider tools exposed during agent turns
memory context bundle construction
pool curation policies
memory-specific CLI
```

Agent providers own the mechanics of exposing granted memory access as native
tools, skills, commands, or sidecar calls. They do not get to invent ambient
memory outside the grant.

## Anti-Goals

Do not make v0 memory:

```text
a hidden personalization channel
an ambient model/provider setting
a vector database API exposed as the language model
a Markdown-file source of truth
an always-on context injector
a semantic ontology every workflow must adopt
a way for providers to mutate facts outside the event log
a package-specific agent-turn lifecycle
```

Do not require the agent to choose a memory tool for ordinary workflow
continuity. The workflow author decides whether memory is pre-recalled, granted
during a turn, learned from deterministic events, or curated by policy.

## Research Notes

This direction is informed by current public memory systems:

- OpenAI separates saved memories, chat-history reference, and project memory;
  the product is powerful but not transparent enough for WhippleScript's
  orchestration model.
- Anthropic exposes chat search/memory summaries in Claude and discusses memory
  as part of context engineering for long-running agents; the developer-facing
  shape is closer when memory is external, inspectable, and tool-mediated.
- OpenClaw's practical memory stack is retrieval-first: local SQLite, BM25,
  vector search, hybrid scoring, optional reranking, and later compiled
  projections. Its "dreaming" lifecycle is useful precedent for curation, but
  WhippleScript should call the source-level operation `curate`.

Useful references:

```text
https://help.openai.com/en/articles/8590148-memory-faq
https://help.openai.com/en/articles/10169521-projects-in-chatgpt
https://support.claude.com/en/articles/11817273-use-claude-s-chat-search-and-memory-to-build-on-previous-context
https://claude.com/blog/context-management
https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
https://docs.openclaw.ai/concepts/memory-builtin
https://docs.openclaw.ai/concepts/dreaming
https://docs.openclaw.ai/reference/memory-config
```

## Open Questions

- What is the exact `MemoryContext` schema, and how much retrieval detail should
  be source-visible versus evidence-only?
- What is the smallest useful curation policy grammar?
- Which curation changes may apply automatically, and which should always
  produce proposed memory for review?
- Should future knowledge-engine projections live under `std.memory`, a
  companion `std.knowledge`, or provider-specific packages?
