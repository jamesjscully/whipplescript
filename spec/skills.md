# Skill Registry

Status: draft

Skills are deterministic context bundles for agents. They teach an agent how to
use a capability, workflow, tool, or project convention.

Skills are core because almost every practical Armature workflow needs to give
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
  docket/
    SKILL.md
  thoth/
    SKILL.md
  repo-worker/
    SKILL.md
```

## Registry

The skill registry loads skills from:

```text
project .armature/skills/
installed Armature packages
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

## Attachment

Skills may be attached to agents:

```armature
agent worker {
  profile "repo-writer"
  skills ["repo-worker", "docket", "thoth"]
}
```

Or to individual turns:

```armature
tell worker with skills ["docket"] """
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
armature-author
docket-user
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
