# Harness Profiles

Status: implemented first slice; target semantics clarified in
[provider-adapters.md](provider-adapters.md)

Harness profiles are the authority boundary for native agent execution. They
bind semantic workflow roles to provider adapters such as Codex, Claude Code,
Pi, or deterministic command fixtures.

Armature workflows should describe agent intent. Harness policy should decide
what that intent is allowed to do in the current environment. This keeps
`.armature` files portable while letting local users stay permissive and
enterprise teams enforce stricter provider behavior.

## Problem

Native `start` and `send` effects are intentionally constrained inside the
workflow language. `codingAgent` declares a workflow role, not a concrete
runner. The provider adapter is selected by harness policy.

The current harness can still run arbitrary provider commands through
`provider: "command"`. That is useful for local deterministic fixtures, but it
is not the right product boundary for nontechnical or enterprise users.

The design separates three authorities:

```text
workflow authority  what the statechart may request
provider authority  what the harness may launch to satisfy that request
agent authority     what the launched agent may do after it starts
```

Armature can enforce workflow and provider authority directly. Agent authority
can be enforced only to the degree the selected provider exposes sandbox,
permission, network, and filesystem controls, or an external sandbox wraps the
provider.

## Goals

- keep the default local path easy
- make safer separation available without rewriting workflows
- give coding agents semantic profile names and descriptions they can choose
  correctly
- let operators define custom profiles outside workflow source
- make enforcement explicit about whether it is native, best-effort, or
  delegated to an external sandbox

## Non-Goals

- perfectly control arbitrary LLM behavior after launch
- make every provider expose the same sandbox flags
- force enterprise users to accept Armature's default profile set
- put provider command strings in ordinary workflow source

## Profile Modes

Harness policy has a mode:

```json
{
  "mode": "permissive"
}
```

### `permissive`

Local default.

Unknown profiles may fall back to provider defaults if the user opts into that
behavior. `command` is allowed. This mode is for experimentation and should be
clearly labeled as broad authority.

### `separated`

Safer built-in mode.

Internet/research authority and repository write authority are separated by
default:

```text
research     network allowed, repo read-only or no repo writes
repo-reader  repo read-only, network disabled by default
repo-writer  repo writes allowed, network disabled by default
human-review no repo mutation, structured review/approval only
```

Unknown profiles are errors. `command` is allowed only through an explicitly
approved profile.

### `custom`

Operator-defined profile set.

Unknown profiles are errors. Every profile must declare a description,
provider, timeout, environment allowlist, filesystem posture, network posture,
and enforcement mode.

## Built-In Profiles

The built-in profile names are intentionally plain. Coding agents should be able
to choose them without memorizing provider internals.

```json
{
  "profiles": {
    "permissive": {
      "description": "Use for trusted local experiments where broad provider defaults are acceptable.",
      "provider": "codex",
      "filesystem": "provider_default",
      "network": "provider_default",
      "enforcement": "best_effort"
    },
    "research": {
      "description": "Use for web research, package documentation, issue discovery, and summarizing external information. Do not use for code edits.",
      "provider": "codex",
      "filesystem": "read_only",
      "network": "allowed",
      "enforcement": "native_or_best_effort"
    },
    "repo-reader": {
      "description": "Use for inspecting repository files, architecture, tests, and logs without making changes.",
      "provider": "codex",
      "filesystem": "read_only",
      "network": "denied",
      "enforcement": "native_or_best_effort"
    },
    "repo-writer": {
      "description": "Use for implementation work after the task is clear. This profile may edit the repository but should not perform internet research.",
      "provider": "codex",
      "filesystem": "workspace_write",
      "network": "denied",
      "enforcement": "native_or_best_effort"
    },
    "human-review": {
      "description": "Use for structured review, approval, or decision collection. Do not use for autonomous code changes.",
      "provider": "command",
      "filesystem": "none",
      "network": "denied",
      "enforcement": "external"
    }
  }
}
```

Provider defaults are installation-dependent. The built-in profiles define
intent and desired authority; each provider adapter must report which fields it
can actually enforce.

## Harness Policy Document

Harness profile policy is separate from workflow source:

```json
{
  "mode": "custom",
  "defaultProfile": "repo-writer",
  "allowCommandProvider": false,
  "profiles": {
    "research": {
      "description": "Use for external documentation and web research. Do not edit repository files.",
      "provider": "codex",
      "command": ["codex", "exec", "{{prompt}}"],
      "args": [],
      "cwd": ".",
      "timeoutSeconds": 1200,
      "filesystem": "read_only",
      "network": "allowed",
      "allowedEnv": ["OPENAI_API_KEY"],
      "allowedTools": ["read", "web"],
      "enforcement": "best_effort"
    },
    "implementer": {
      "description": "Use for code changes in the repository after the plan is clear.",
      "provider": "codex",
      "command": ["codex", "exec", "{{prompt}}"],
      "timeoutSeconds": 1800,
      "filesystem": "workspace_write",
      "network": "denied",
      "allowedEnv": ["OPENAI_API_KEY"],
      "allowedTools": ["read", "edit", "test"],
      "enforcement": "best_effort"
    }
  }
}
```

Field meanings:

```text
mode              permissive | separated | custom
defaultProfile    profile used when a workflow agent omits profile
allowCommandProvider
                  whether raw command profiles are allowed
profiles          map of profile name to profile definition
description       human/agent-facing guidance for when to use the profile
provider          command | codex | claude-code | pi
command           optional command template override
args              optional extra command args
cwd               working directory
timeoutSeconds    provider timeout
filesystem        provider_default | none | read_only | workspace_write
network           provider_default | denied | allowed
allowedEnv        environment variable allowlist
allowedTools      semantic tool names; provider adapters map what they can
enforcement       native | best_effort | external | native_or_best_effort
```

## Workflow Source

Workflow authors should select profiles by intent:

```armature
agent researcher = codingAgent {
  profile "research"
  maxActive 2
}

agent worker = codingAgent {
  profile "repo-writer"
  maxActive 3
}
```

The parser may accept `codingAgent()` as a compatibility alias while examples
and diagnostics migrate to the declaration-style `codingAgent` form. The
parentheses form should not be used to imply a function call or concrete
provider.

If omitted, the profile is resolved from harness policy:

```text
custom/separated mode  use defaultProfile, or error if absent
permissive mode        use permissive
```

Profile names are not provider names. A team may map `repo-writer` to Codex
locally, Claude Code in CI, or an external sandbox in enterprise.

## Resolution

For each native invocation:

```text
declared agent -> requested profile -> harness policy profile -> provider runner
```

Resolution must record:

```text
requested profile
resolved profile
provider
enforcement level
requested authority
enforced authority
unsupported requested restrictions, if any
```

If a profile requests stricter authority than the provider can enforce:

```text
permissive  allow with warning and harness event
separated   deny unless enforcement is explicitly best_effort
custom      obey profile enforcement field
```

## Provider Enforcement

Providers advertise enforcement support:

```json
{
  "provider": "codex",
  "supports": {
    "filesystem": ["read_only", "workspace_write"],
    "network": ["denied", "allowed"],
    "envAllowlist": true,
    "toolAllowlist": "best_effort"
  }
}
```

Armature should not silently claim enforcement it cannot provide. When a
restriction is best-effort, the harness event should say so.

Implemented provider mappings:

```text
codex   filesystem read_only/workspace_write -> --sandbox
codex   network allowed/provider_default -> --search
codex   network denied -> do not enable --search, report limited denial
claude-code filesystem read_only/none -> --permission-mode plan
claude-code filesystem workspace_write -> --permission-mode acceptEdits
claude-code allowedTools -> --allowedTools
claude-code network denied -> best-effort warning; no stable native flag mapped
command requires explicit command-provider approval outside permissive mode
pi      best-effort only until stable sandbox flags are documented and tested
```

Provider preset commands apply the mapped native flags directly. If a profile
overrides the command for `codex`, `claude-code`, or `pi`, Armature records
that the custom command is responsible for applying equivalent provider flags
and marks the launch evidence as best-effort for that command shape.

When a profile supplies `allowedEnv`, provider processes inherit only that
allowlist plus minimal runtime keys needed to execute (`PATH`, `HOME`,
`TMPDIR`) and Armature's own invocation environment variables.

## CLI Surface

Harness commands should accept:

```sh
armature harness once workflow.armature \
  --config harness.json \
  --profile-policy .armature/harness-policy.json

armature harness run workflow.armature \
  --config harness.json \
  --profile-policy .armature/harness-policy.json \
  --drive-workflow

armature validate workflow.armature \
  --profile-policy .armature/harness-policy.json

armature validate-profile-policy .armature/harness-policy.json \
  --workflow workflow.armature
```

`--profile-policy` is the product path for governed environments and should be
sufficient by itself once provider adapters are complete. `--config` remains a
concrete provider runner override for deterministic local experiments and tests.
The concepts should stay distinct: profile intent versus concrete runner
details.

## Skill Guidance

The Armature skill should teach coding agents:

- use `research` for internet/package/docs discovery
- use `repo-reader` for codebase inspection without edits
- use `repo-writer` for implementation
- use `human-review` for approval or decision collection
- do not combine network and repo write authority unless the user explicitly
  asks for permissive mode or supplies a custom profile that allows it
- prefer semantic profile names over provider names in workflow source

The profile descriptions in policy are part of the authoring interface. Coding
agents should read them before assigning profiles.

## Open Implementation Questions

- whether external sandbox wrappers should be modeled as providers or
  enforcement backends
- stable Pi sandbox flag mapping
- whether the runtime should retire the completed invocation before evaluating
  `start` effects in the same `finished` transition, or whether users should
  model that sequencing through an internal event
