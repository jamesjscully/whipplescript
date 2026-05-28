# Companion Skill

Status: draft

Armature should ship a companion skill for coding agents that author or operate
Armature workflows.

The skill is core because most target users will ask a coding agent to write the
workflow. The agent needs current language and runtime guidance that is not in
its training distribution.

## Skill Name

```text
armature-author
```

## Purpose

The skill teaches agents:

- Armature is a restricted event-sourced rule machine.
- Rules produce facts and durable effects; effects do not run inline.
- Source order does not sequence effects; use `after` for explicit dependency.
- Use Docket for project work tracking when available.
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
Docket-driven implementation loop example
coerce example
human review example
dependent-effect / `after` example
common diagnostics and fixes
capability/profile selection guidance
plugin discovery guidance
profile-selection guidance
evidence/status inspection guidance
```

## Anti-Patterns

The skill should warn against:

```text
writing arbitrary TypeScript control loops
using shell scripts as hidden workflow engines
encoding issue tracking inside Armature facts when Docket is available
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
Armature authors
agent harness turns that need to operate Armature
dev-mode sessions
```
