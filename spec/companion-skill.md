# Companion Skill

Status: implemented local package, active validation

WhippleScript should ship a companion skill for coding agents that author or operate
WhippleScript workflows.

The skill is core because most target users will ask a coding agent to write the
workflow. The agent needs current language and runtime guidance that is not in
its training distribution.

## Skill Name

```text
whipplescript-author
```

## Purpose

The skill teaches agents:

- WhippleScript is a restricted event-sourced rule machine.
- Rules produce facts and durable effects; effects do not run inline.
- Source order does not sequence effects; use `after` for explicit dependency.
- Use `std.tracker` for project work tracking when available.
- Use `coerce` for typed schema coercion; coerce is a backend, not workflow logic.
- Use skills and capabilities instead of inventing shell scripts.
- Choose profiles by authority intent, not provider brand.
- Keep workflows small, explicit, and analyzable.
- Avoid internal effect recursion.
- Treat workflow revision as a control-plane action. Source rules may propose
  candidate patch artifacts, but they must not activate running revisions.
- Prefer package capabilities/providers for memory, GitHub, browser, etc.
- Inspect status/evidence before guessing.

## Required Content

The skill should include:

```text
minimal workflow example
Ralph loop example
tracker-driven implementation loop example
coerce example
human review example
dependent-effect / `after` example
common diagnostics and fixes
workflow revision patch-proposal guidance
capability/profile selection guidance
package/provider discovery guidance
profile-selection guidance
evidence/status inspection guidance
```

Current authoring guidance from validation:

- Use guarded fact matches for deterministic routing over typed fields, for
  example `when LanguageTask as task where task.provider == "codex"`.
- Prefer `AgentRef<codex | claude | pi>` for dynamic agent routing. `tell`
  targets should be literal declared agents or `AgentRef` fields such as
  `tell task.provider`; never ask a model or schema-coercion output to decide
  the route.
- Keep provider/model identity as source metadata or observed evidence. Do not
  make language models identify which provider is active unless the task is
  explicitly reviewing provider evidence.
- Put `as binding` on the same line as the effect keyword. Multi-line strings
  may follow, but the binding must be visible on the effect line for the current
  parser.
- Use `call <capability> ... as <binding>` for package capabilities such as
  memory. Do not invent package-specific control-flow syntax.
- For running workflow revision, propose a candidate `.whip` artifact with
  ordinary effects or child workflow invocations. Tell the operator to run
  `whip revise --dry-run` and activate the revision from the control plane.

## Validation Fixture

The checked companion-skill validation fixture uses:

- one shared `CompanionReviewTask` schema with
  `reviewer AgentRef<codex | claude | pi>`.
- deterministic source-seeded review tasks for spec, validation, and docs
  review phases.
- `tell task.reviewer requires ["agent.tell"]` so the compiler and runtime
  enforce declared agent capability metadata before provider execution.
- source assertions over `CompanionReviewDispatch` and `effect kind agent.tell`
  counts.

The fixture deliberately does not ask coerce, Codex, Claude, Pi, or any other
model to identify which provider/model is active or which route should be
selected. The prompt repeats that route identity has already been selected by
typed source metadata and asks the thread only to review its assigned phase and
update the visible tracker.

Validation runs through the checked example fixture and the CLI e2e suite.

## Anti-Patterns

The skill should warn against:

```text
writing arbitrary TypeScript control loops
using shell scripts as hidden workflow engines
encoding issue tracking inside WhippleScript facts when `std.tracker` is available
silently injecting memory/context without provenance
starting agent turns before tracker claims and capability checks are accepted
depending on source order to sequence effects
depending on prompt text as a completion condition
using one powerful profile for unrelated research and write tasks
self-modifying a running instance from source rules instead of proposing a
patch artifact for `whip revise`
```

## Delivery

The skill should be installed as a first-party package resource and exposed by
the skill registry. It should be available to:

```text
WhippleScript authors
agent harness turns that need to operate WhippleScript
dev-mode sessions
```

Current local install path:

```sh
scripts/install-whipplescript-skill.sh
```

By default this copies `skills/whipplescript-author/SKILL.md` to
`$HOME/.codex/skills/whipplescript-author/SKILL.md`. Set `WHIPPLESCRIPT_SKILL_DIR` to
install into a different skill directory.

Current package path:

```sh
scripts/package-whipplescript-skill.sh
```

By default this writes `dist/whipplescript-author-skill.tar.gz` plus a `.sha256`
checksum. Set `WHIPPLESCRIPT_SKILL_DIST_DIR` to write the package elsewhere.
