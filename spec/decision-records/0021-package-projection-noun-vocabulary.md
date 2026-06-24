# 0021: Package-Projection Noun Vocabulary (multi-word `expect` nouns)

Status: proposed design — recommends deferral (awaiting decision)

## Problem

`workflow-testing.md` wants `expect` to read in package-domain vocabulary:

```whip
expect issue exists where title contains "Migration failed"
expect message sent to ops where body contains "run_123"
expect file project_files at "reports/triage.md" exists
expect memory project_memory learned where topic == "run_123"
```

Today `projection_noun := DottedName` — a single dotted fact name (`Issue`,
`agent.turn.completed`). The multi-word, parameterized forms above (`message sent
to <target>`, `file <store> at <path>`, `memory <pool> learned`) do not parse, and
there is no model that maps such a noun phrase to an underlying projection fact.

## What the feature actually requires

1. **A package projection-noun vocabulary.** Each package declares the domain
   nouns it projects, as phrase *templates* with parameter slots and a mapping to
   the underlying fact + field schema, e.g.
   `"message sent to <target>" -> fact event.notify.completed, slot target = $.target`.
   This is new package-manifest surface that does not exist today.
2. **Slot-aware, package-aware scenario parsing.** The scenario parser must parse
   an arbitrary phrase (greedily consume tokens until `where`/`exists`/`count`/`at`),
   then resolve it against the declared vocabulary, binding slot values (`ops`).
   This couples the scenario grammar to the package lock (the noun set is dynamic),
   and must handle the spec's "ambiguous package projection" case.
3. **Harness resolution.** Resolve the phrase to its fact + a slot-derived
   predicate and evaluate it against projection rows (the existing `proj_query`
   machinery, extended with the slot binding).

## The gating problem (why this is mostly blocked)

Each noun is only resolvable if its underlying projection exists. Current state:

| Noun | Underlying projection | Status |
|---|---|---|
| `issue` | tracker `queue.item.ready` | **exists** (and is now seedable via `given tracker`) |
| `message sent to <t>` | `event.notify.completed` | partial — the effect-completion event exists, but there is no declared "message" projection with a `target` field model |
| `file <store> at <path>` | `std.files` | **blocked** — `std.files` is unimplemented (DR-0019 design-only); no file projection exists |
| `memory <pool> learned` | `std.memory` projection | thin — no declared "memory learned" projection vocabulary |

So three of the four canonical nouns map to projections that are unbuilt or
und-modeled. Building the vocabulary + slot-parsing mechanism now would be a large
layer over projections that mostly don't exist yet — the same gating that deferred
`given file` (DR-0020 / #3).

Notably, the one resolvable noun, `issue`, is **already queryable today** via the
dotted form `expect queue.item.ready where title contains "…"`. The multi-word
`expect issue …` is pure ergonomic sugar over a capability that already works.

## Options

- **A — Full vocabulary now.** Add the package projection-noun declaration model,
  slot-aware package-aware parsing, and resolution. Cost: large, cross-cutting
  (manifest + compiler + parser + harness), and most nouns stay non-functional
  until their packages exist. High effort, low immediate payoff.
- **B — Built-in `issue`-only noun now.** Hard-code `issue -> queue.item.ready`
  (and maybe `message -> event.notify.completed`) as built-in nouns, defer the
  general vocabulary + the std.files/memory nouns. Smaller, but it builds a
  bespoke mechanism for one-to-two nouns that are already queryable by dotted
  name — marginal value.
- **C — Defer (recommended).** Keep dotted-name projections (which already cover
  every projection that exists, including `queue.item.ready`). Build the
  package-projection-noun vocabulary together with the package projection model,
  once the underlying projections (`std.files`, a `notification`/message
  projection, a `memory` projection vocabulary) exist — so the vocabulary is
  designed against real projections, not ahead of them.

## Recommendation

**Option C (defer).** The multi-word nouns are ergonomic sugar; the capability
they would add (asserting on package projections) is already met by dotted-name
projections for every projection that currently exists. The general mechanism is
large and most of its target nouns are blocked on unimplemented packages
(`std.files`) or unmodeled projections (`message`/`memory`). Building it now
inverts the dependency — a noun layer ahead of the nouns. Revisit when the
standard package projections land; the vocabulary then has concrete facts +
field schemas to bind to, and the spec's ambiguity rules can be defined against
real cases.

If immediate ergonomics are still wanted, the minimal honest step is Option B's
`issue` alias — but it duplicates `queue.item.ready` and is not recommended as a
standalone investment.

## Open question

Do we accept deferral (C), or is a built-in `issue`/`message` alias (B) wanted
now despite the dotted-name equivalent?
