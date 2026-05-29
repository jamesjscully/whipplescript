# Companion Skill

Status: implemented local package, active dogfood

Whippletree should ship a companion skill for coding agents that author or operate
Whippletree workflows.

The skill is core because most target users will ask a coding agent to write the
workflow. The agent needs current language and runtime guidance that is not in
its training distribution.

## Skill Name

```text
whippletree-author
```

## Purpose

The skill teaches agents:

- Whippletree is a restricted event-sourced rule machine.
- Rules produce facts and durable effects; effects do not run inline.
- Source order does not sequence effects; use `after` for explicit dependency.
- Use Loft for project work tracking when available.
- Use BAML `coerce` for typed model decisions.
- Use skills and capabilities instead of inventing shell scripts.
- Choose profiles by authority intent, not provider brand.
- Keep workflows small, explicit, and analyzable.
- Avoid internal effect recursion.
- Prefer plugin capabilities for memory, Thoth, GitHub, browser, etc.
- Inspect status/evidence before guessing.

## Required Content

The skill should include:

```text
minimal workflow example
Ralph loop example
Loft-driven implementation loop example
coerce example
human review example
dependent-effect / `after` example
common diagnostics and fixes
capability/profile selection guidance
plugin discovery guidance
profile-selection guidance
evidence/status inspection guidance
```

Current authoring guidance from dogfood:

- Use guarded fact matches for deterministic routing over typed fields, for
  example `when LanguageTask as task where task.provider == "codex"`.
- Prefer `AgentRef<codex | claude | pi>` for dynamic agent routing. `tell`
  targets should be literal declared agents or `AgentRef` fields such as
  `tell task.provider`; never ask a model or BAML output to decide the route.
- Keep provider/model identity as source metadata or observed evidence. Do not
  make language models identify which provider is active unless the task is
  explicitly reviewing provider evidence.
- Put `as binding` on the same line as the effect keyword. Multi-line strings
  may follow, but the binding must be visible on the effect line for the current
  parser.
- Use `call <capability> ... as <binding>` for plugin capabilities such as
  memory. Do not invent plugin-specific control-flow syntax.

## Dogfood Fixture

`examples/companion-skill-dogfood.whip` is the checked companion-skill dogfood
fixture. It uses:

- `use skill "whippletree-author"` to make the authored workflow explicitly
  depend on this skill.
- one shared `CompanionReviewTask` schema with
  `reviewer AgentRef<codex | claude | pi>`.
- deterministic source-seeded review tasks for spec, validation, and docs
  review phases.
- `tell task.reviewer requires ["agent.tell"]` so the compiler and runtime
  enforce declared agent capability metadata before provider execution.
- source assertions over `CompanionReviewDispatch` and `effect kind agent.tell`
  counts.

The fixture deliberately does not ask BAML, Codex, Claude, Pi, or any other
model to identify which provider/model is active or which route should be
selected. The prompt repeats that route identity has already been selected by
typed source metadata and asks the thread only to review its assigned phase and
update the visible tracker.

Validation:

```sh
cargo run -q -p whippletree-cli -- check examples/companion-skill-dogfood.whip
cargo run -q -p whippletree-cli -- compile examples/companion-skill-dogfood.whip
cargo test -p whippletree-cli --test control_plane dev_companion_skill_dogfood_routes_with_agentref_metadata
```

## Anti-Patterns

The skill should warn against:

```text
writing arbitrary TypeScript control loops
using shell scripts as hidden workflow engines
encoding issue tracking inside Whippletree facts when Loft is available
silently injecting memory/context without provenance
starting agent turns before claims/capabilities are accepted
depending on source order to sequence effects
depending on prompt text as a completion condition
using one powerful profile for unrelated research and write tasks
```

## Delivery

The skill should be installed as a first-party package resource and exposed by
the skill registry. It should be available to:

```text
Whippletree authors
agent harness turns that need to operate Whippletree
dev-mode sessions
```

Current local install path:

```sh
scripts/install-whippletree-skill.sh
```

By default this copies `skills/whippletree-author/SKILL.md` to
`$HOME/.codex/skills/whippletree-author/SKILL.md`. Set `WHIPPLETREE_SKILL_DIR` to
install into a different skill directory.

Current package path:

```sh
scripts/package-whippletree-skill.sh
```

By default this writes `dist/whippletree-author-skill.tar.gz` plus a `.sha256`
checksum. Set `WHIPPLETREE_SKILL_DIST_DIR` to write the package elsewhere.
