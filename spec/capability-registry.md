# Capability Registry

Status: draft

The capability registry binds source-level intent to concrete providers,
profiles, credentials, and enforcement.

It is core because the restricted rule language depends on effects being
auditable and policy-controlled.

## Registry Inputs

```text
program declarations
environment policy
installed package manifests
operator configuration
provider self-reported capabilities
```

## Binding Record

```text
capability_id
effect_kinds
provider
input_schema
output_schema
required_credentials
filesystem_authority
network_authority
process_authority
retention_policy
enforcement_mode
```

Provider bindings extend capability bindings with operational configuration:

```text
provider_id
provider_kind
profiles
capabilities
credentials_ref
workspace_policy
adapter_config_ref
max_parallel_runs
native_enforcement_level
health_check
artifact_policy
retention_policy
```

`credentials_ref` points to operator-managed configuration or secret storage.
Workflow source must never contain provider credentials.

## Profiles

Agent profiles are also registry objects:

```text
profile_id
description
provider
sandbox
allowed_capabilities
allowed_skills
default_timeout
retry_policy
artifact_policy
```

Descriptions matter because coding agents writing WhippleScript scripts should be
able to choose profiles by intent.

## Default Profile Sets

The distribution should ship a tiny number of understandable defaults.

Permissive local mode:

```text
local-permissive
```

This is useful for experimentation. It may grant broad repo, network, and
process authority through the chosen provider if the operator accepts that
risk.

Safer local mode:

```text
repo-reader
repo-writer
internet-research
human-review
```

This separates read/write repo authority from internet research. It is the
minimum useful discipline for agent workflows that should not let the same turn
both fetch arbitrary internet content and write project files.

The current runtime seeds repository, process, and tracker mutation authority as
separate capabilities: `repo.read`, `repo.write`, `command.run`,
`tracker.file`, `tracker.claim`, `tracker.finish`, `tracker.release`,
`tracker.update`, `tracker.write`, and `workflow.invoke`. Owned-harness file
tools use `repo.read`/`repo.write`; `bash` also requires `command.run`, an explicit
`with access to command { run }` turn grant, and the operator command allow-list.
Tracker `add_todo` requires `with access to tracker { file }`; `update_todo`
requires `claim`, `finish`, or `release` for the requested status transition
(`update` grants all update transitions, and `write` grants all tracker
mutations). Curated `@tool` sub-workflow tools use `workflow.invoke`. A
`tell ... requires [...]` list narrows an owned turn to the known harness
capabilities it names; the store already checks that the target agent's
declaration contains each required capability before any provider run starts.
When an IFC governance envelope is active, file-store resources named by turn
grants, the `command` resource (for bash), the `tracker` resource for mutating
tracker grants, and cross-package `invoke:<pkg>/<tool>` workflow-tool doors must
also be governed by that envelope before the owned turn is admitted.

These runtime profile *objects* and the `std.agent` canonical preset list
([`decision-records/0009-agent-package.md`](decision-records/0009-agent-package.md))
share one vocabulary. The review role is named `human-review` everywhere — spec,
implementation plan, and code (the shipped default profile). Earlier drafts also
called it `review-only` here and `reviewer` in the std.agent preset list; those are
the same role and have been unified to `human-review`.

Enterprise mode is configuration-driven. Operators can define their own
profiles, descriptions, provider bindings, credentials, retention policies, and
enforcement requirements.

## Enforcement Modes

```text
strict
native_or_best_effort
advisory
fixture
```

In governed environments, `strict` should be the default. Local developer
setups may choose permissive defaults.

The registry must distinguish:

```text
requested by script
granted by environment
enforced by provider
```

An effect can proceed only when environment policy accepts the provider's
enforcement level for the requested profile or capability.

Provider selection must be explainable. If multiple providers can satisfy a
profile, the registry should record which provider was selected and why. If no
provider can satisfy the request, the blocked effect should name the missing or
insufficient binding.

## CLI Shape

```sh
whip capabilities list
whip capabilities show <id>
whip profiles list
whip profiles show <id>
whip packages list
whip packages enable <package>
whip packages disable <package>
```

The status view for a blocked effect must show which capability or profile
binding failed.
