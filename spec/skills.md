# Skill Registry

Status: draft

Skills are deterministic context bundles for agents. They teach an agent how to
use a capability, workflow, tool, or project convention.

Skills are core because almost every practical Whippletree workflow needs to give
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
project .whippletree/skills/
installed Whippletree packages
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

```whippletree
agent worker {
  profile "repo-writer"
  skills ["repo-worker", "loft", "thoth"]
}
```

Or to individual turns:

```whippletree
tell worker with skills ["loft"] """
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

## Core Skills

First-party core skills:

```text
whippletree-author
loft-user
baml-coerce-user
human-review-user
```

Optional package/plugin skills:

```text
thoth-user
memory-user
github-user
browser-user
```
