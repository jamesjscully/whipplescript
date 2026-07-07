# DR-0034 — Managed vs. delegated harnesses

Status: accepted (2026-07-07). Splits the harness abstraction that DR-0024 (owned
brokered harness) was retrofitted into. Reframes and supersedes
`spec/context-assembly-tracker.md` **Phase 6** (delegated-path context), and
absorbs the "formalize the sidecar protocol" candidate-DR-0034 flagged in
`spec/compute-plane-design-note.md` at the architecture level (the wire-level
protocol becomes a follow-on under this record). Cross-refs: DR-0024 (owned
harness loop), DR-0026/0027 (authority envelope), the context-assembly tracker
(Phases 1–5 + 7, the managed context path).

## Problem

WhippleScript runs an agent turn one of two ways, and today both wear one
abstraction. `is_supported_harness_kind` is a **flat enum** —
`"codex" | "claude" | "fixture" | "native-fixture" | "command" | "owned"` (pi
retired) — and `run_agent_effect` funnels every kind through one
`AgentTurnExecution` and one evidence path. `owned` is just another string in
that list.

That is a retrofit. The `harness`/`provider` construct was designed for **external
providers** before the owned harness existed; the owned brokered harness (DR-0024)
was then shoehorned in as "one more provider." The two are not one more of each
other — they provide **categorically different guarantees**:

- The **owned** harness *is* the agent runtime. It assembles context hermetically,
  records every bundle as evidence, brokers each tool through the kernel, owns
  compaction, and the turn is reproducible. The guarantees are WhippleScript's,
  end to end (Phases 1–5 + 7 of the context-assembly effort).
- An **external** harness (codex/claude sidecar) is a *foreign* runtime WhippleScript
  invokes. It has its own context assembly, its own ambient config, its own skill
  discovery, its own loop. WhippleScript cannot see inside.

Forcing both through one abstraction forces a bad choice everywhere. We chose to
**cripple the external path** to fake uniformity: `build_claude_agent_tool_policy`
hard-codes `setting_sources: Vec::new()`, so the Claude sidecar reads *none* of its
own `CLAUDE.md` / settings / native skills. That is the wrong trade. We cannot give
WhippleScript's hermetic, reproducible, evidence-grade guarantee for a runtime we
do not control, and pretending to — by disabling the very context assembly the
provider is good at — makes the external harness worse without making it hermetic.
The guarantee asymmetry is **real and permanent**; it should be modeled as a type,
not papered over with a config flag.

## Decision 1 — Two harness classes

Introduce `HarnessClass`, the axis everything downstream branches on:

- **`Managed`** — WhippleScript is the runtime (owned; the credential-free
  `fixture` model client is a Managed variant). Full guarantee: hermetic context,
  per-bundle provenance, brokered tools, deterministic compaction, reproducible.
- **`Delegated`** — WhippleScript invokes a foreign runtime (codex, claude;
  `native-fixture` and `command` are Delegated variants). The runtime owns its own
  context; WhippleScript owns the **envelope**.

Managed is the substrate. Delegated kinds are pluggable foreign delegates. This is
not a spectrum with a knob — it is a two-valued sort, because the guarantee it
selects is two-valued.

## Decision 2 — The distinction is legible in the source

Spell the class at the agent declaration so a reader (and an auditor) can tell
which guarantee a turn carries without tracing dispatch:

```
agent Reviewer {                    -- Managed (default): WhippleScript harness
  profile "repo-reader"
  compaction tool_results
}

agent Coder delegated to claude {   -- Delegated: foreign runtime
  profile "repo-writer"
  settings project                  -- delegation knob (see Decision 4)
}
```

`agent Foo { … }` is Managed by default (the substrate). `agent Foo delegated to
<provider> { … }` is Delegated. The legacy `provider owned` / `provider codex` /
`using <harness>` forms migrate onto this axis (Decision 6). The word `provider` no
longer means two incompatible contracts.

## Decision 3 — Knob partition (compile-time, not silent)

Each class admits only its meaningful knobs; the other class **rejects them with a
diagnostic**, never silently ignores them.

- **Managed-only** (context is WhippleScript's to shape): `compaction <strategy>`
  (DR — context-assembly Phase 5), `tell … with skills [...]` pinning (Phase 7),
  the context-bundle apparatus. On a Delegated agent these are errors — *you cannot
  tell Claude how to compact; it does its own.*
- **Delegated-only** (the runtime's context is its own): `settings <sources>`
  (ambient config exposure — Decision 4), native skill-dir exposure, provider MCP
  config. On a Managed agent these are errors — WhippleScript already assembles the
  context; there is nothing foreign to configure.
- **Shared** (authority, not context): `profile`, `capabilities`, `capacity`,
  `with access to`, `tools [...]`. Authority stays WhippleScript's for both classes
  (Decision 7).

Silent no-ops are the current failure (a `compaction` on a delegated agent does
nothing today); the split turns them into honest compile errors.

## Decision 4 — Delegated harnesses assemble their own context, honestly

Stop force-disabling `setting_sources`. A Delegated harness reads its own ambient
config (`CLAUDE.md`, settings), discovers its own skills, and assembles its own
context — because that is what it is good at, and WhippleScript's value on this path
is the **envelope**, not the context. The `settings` knob selects *which* sources
the delegate may read (e.g. `project`, `user`, `none`), defaulting to the
provider's own default rather than the crippled empty set.

This is the direct resolution of the Phase-6 complaint: delegated turns get their
real, ambient-config-included behavior **without** diluting the owned guarantee,
because the two paths no longer share one contract.

Materializing WhippleScript's *own* assets into a delegate-discoverable location
(the Phase-6 "materialize skills" idea) remains available as an *optional* Delegated
knob, but is no longer the primary story: a Delegated harness is trusted to
assemble its context; WhippleScript does not have to launder its own assets through
the provider to preserve a hermeticity it is not claiming here.

## Decision 5 — The evidence model forks with the class

The audit record must not conflate two provenance stories.

- **Managed** emits full provenance: `context.bundle` per assembled bundle,
  `skills.pinned`, brokered tool observations, `context.compaction` — the reproducible
  trail Phases 1–5 + 7 record.
- **Delegated** emits an **attestation**, not a fabricated assembly trail: provider
  identity + version, the prompt hash, the tool/permission policy, and the terminal,
  explicitly tagged *`context: provider-assembled`* (not WhippleScript-hermetic). It
  does **not** emit `context.bundle` rows for a context WhippleScript did not build.

An auditor reading the record can then tell, from the record alone, which guarantee
they are holding. Both classes still emit the single `agent.turn.<status>` terminal
fact (the shared lifecycle surface is preserved; only the provenance depth differs).

## Decision 6 — Reclaim `harness` for the delegate role; managed is the substrate

`harness <name>: <kind>` predates the owned harness and was *for* external
providers. Owned is not a "harness" one binds — it is the native execution mode. So
the model reverses the retrofit: **Managed execution is the default/native mode; a
`harness` declaration (or `delegated to`) names a foreign delegate.** The flat
`is_supported_harness_kind` enum is replaced by a `HarnessClass` classification:

| kind | class |
|------|-------|
| owned | Managed |
| fixture | Managed |
| claude, codex | Delegated |
| native-fixture, command | Delegated |

Migration is incremental and behavior-preserving as a first step (Decision 8): keep
the existing kinds, classify them, and gate knobs/evidence/`setting_sources` on the
class. The surface sugar (`delegated to`) and the Managed-default reading land on
top of that floor.

## Decision 7 — Authority is shared; only context and its provenance differ

The split is about **context**, not **authority**. For *both* classes WhippleScript
still: gates tools by profile + capabilities (allowed-tools may only narrow, never
widen — DR-0024/skills), holds the workspace lease, and enforces the capability
envelope (DR-0026/0027). A Delegated harness reading its own `CLAUDE.md` cannot
thereby grant itself a tool the program did not authorize — ambient config steers
*behavior within the granted authority*, it does not expand authority. This bound is
what makes Decision 4 safe: we hand back **context** freedom, not **authority**
freedom.

## Decision 8 — Build sequencing

1. **Classify (behavior-preserving floor).** Add `HarnessClass` + a
   `kind → class` map; thread it through lowering and `run_agent_effect` dispatch.
   No behavior change yet; every gate stays green.
2. **Fork the evidence path.** Managed keeps the full-provenance recording; Delegated
   switches to the attestation record with the `context: provider-assembled` tag.
3. **Un-cripple delegated context.** Replace the hard-coded
   `setting_sources: Vec::new()` with the `settings` knob (default = provider
   default). Add the `settings` grammar to Delegated agents.
4. **Partition the knobs.** Compile-error a Managed-only knob on a Delegated agent
   and vice-versa (Decision 3).
5. **Surface sugar + Managed default.** `agent Foo delegated to <provider>` and
   `agent Foo { … }`-is-Managed; migrate `provider …` / `using …` onto the axis.
6. **(Follow-on)** Formalize the delegated sidecar wire protocol (the original
   candidate-DR-0034 scope), now a sub-decision under this record.

Model-first per repo discipline: the class invariants (a Managed-only knob is
unreachable on a Delegated agent; a Delegated turn emits no `context.bundle`; ambient
config never widens authority) get a Maude bite before the Rust lands.

## Open questions

- **Surface spelling.** `delegated to <provider>` reads well; an alternative is a
  `kind`/`class` field. Committing to `delegated to` (Decision 2) unless the grammar
  makes it awkward against `using <harness>`.
- **`settings` vocabulary.** The value set for `settings` (`project` / `user` /
  `none` / a path) mirrors the provider's own setting-source model; needs a small
  survey of what codex/claude actually accept.
- **Fixture placement.** `fixture` (Managed, credential-free owned client) vs
  `native-fixture` (Delegated test adapter) — confirm the split matches how the two
  are used in tests before locking the map.
- **Cross-package agents.** How a package that exports a Delegated agent attests its
  (provider-assembled) context surface under DR-0029's package IFC — likely "opaque,
  attested at the boundary," but out of scope here.
