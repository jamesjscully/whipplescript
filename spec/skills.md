# Skill Registry

Status: draft

Skills are deterministic context bundles for agents. They teach an agent how to
use a capability, workflow, tool, or project convention.

Skills are core because almost every practical WhippleScript workflow needs to give
agents operational instructions without bloating every prompt.

## Skill Object

```text
Skill = {
  id
  version
  source
  description
  file_path
  required_capabilities
  provided_commands?
  examples?
}
```

The canonical file format may follow the `SKILL.md` convention:

```text
skills/
  loft/
    SKILL.md
  thoth/
    SKILL.md
  repo-worker/
    SKILL.md
```

## Registry

The skill registry loads skills from:

```text
project .whipplescript/skills/
installed WhippleScript packages
first-party bundled skills
explicit CLI/config paths
plugin resource discovery
```

The registry records provenance for each skill:

```text
source package
version/hash
path
capability requirements
enabled/disabled state
```

The first runtime implementation persists that provenance in SQLite. Registering
a skill records its name, version, source text, source path, deterministic
content hash, required capabilities, and metadata. Attachments are explicit rows
scoped to `program`, `agent`, or `run`, so the harness never has to infer hidden
context. Before a provider turn starts, the harness can write `skills.injected`
evidence for the run; that evidence includes each injected skill's exact
version, path, and hash.

## Attachment

Skills may be attached to agents:

```whipplescript
agent worker {
  profile "repo-writer"
  skills ["repo-worker", "loft", "thoth"]
}
```

There is no top-level `use skill` form. Top-level `use` imports plugins; skills
enter provider context only through explicit agent or turn attachment.

Or to individual turns:

```whipplescript
tell worker with skills ["loft"] """markdown
Claim one ready issue and implement it.
"""
```

The harness resolves skills into context before the provider turn starts.

## Rules

- Skill loading must be deterministic.
- Skill provenance must be visible in artifacts.
- Skill content must not silently grant capability authority.
- Skills may instruct agents to use tools, but policy decides whether tools are
  available.
- Skills should be small and operational, not giant hidden manuals.

## First-Party Skills

First-party skills are Claude-style agent context bundles. They are attached to
agents or individual turns; they do not extend WhippleScript syntax and do not
grant capabilities.

```text
whipplescript-author
loft-user
human-review-user
```

Optional package/plugin skills:

```text
thoth-user
memory-user
github-user
browser-user
```
