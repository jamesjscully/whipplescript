# Capability Policy

Status: design proposal

Capability policy controls what workflows, agents, adapters, and resources are
allowed to do.

The product should be easy locally and strict in enterprise environments. The
mechanism should be the same in every mode; only defaults and enforcement levels
change.

## Modes

### Local

Local mode optimizes for fast experimentation.

Defaults:

```text
undeclared capabilities produce warnings
common local adapters are allowed
filesystem resources must still be declared for writes
external process adapter is allowed only if explicitly enabled
```

### Team

Team mode encourages explicit authority.

Defaults:

```text
undeclared capabilities are errors for writes and starts
read-only broad access may be allowed with warnings
adapter capabilities must be declared
resource writes must be declared
```

### Enterprise

Enterprise mode is deny-by-default.

Defaults:

```text
all capabilities must be declared
all adapter capabilities must be declared
all resource access must be declared
effective authority is the intersection of policy layers
policy violations are errors
```

## Policy Layers

Effective authority is resolved from:

```text
workflow declaration
workspace policy
resource policy
adapter advertised capabilities
target agent/thread/session policy
runtime mode defaults
```

Enterprise mode uses strict intersection:

```text
effective = requested_by_workflow
          ∩ allowed_by_workspace
          ∩ allowed_by_resource
          ∩ advertised_by_adapter
          ∩ allowed_by_target
```

Local and team modes may treat missing upper-layer policy as broad allow, but
the runtime must still record the decision.

## Capability Names

Capabilities are strings with optional namespaces:

```text
read_repo
edit_code
run_tests
message_agents
askHuman
baml.coerce
resource.plan.read
resource.plan.write
adapter.untie.start
adapter.process.start
```

Capability names are exact non-empty strings and must not contain whitespace or
control characters. This keeps adapter manifests and policy documents from
silently spelling different authorities with visually similar names.

The initial implementation should not require a global registry for all possible
capabilities, but built-in capabilities and adapter capabilities must be
schema-described.

## Adapter Capability Advertisement

Each adapter must advertise:

```text
adapter name
supported effects
required capabilities per effect
input schemas
output schemas
idempotency behavior
failure categories
model abstraction
```

Validation checks workflow effects against these declarations.

## BAML Policy

BAML is available only through the `coerce` effect.

Policy controls:

```text
which BAML clients/providers may be used
which model environment variables may be read
whether network/model access is allowed
which BAML tools, if any, may be exposed
```

The first implementation should treat BAML as structured model output only. BAML
tools that perform side effects should be disabled unless explicitly enabled by
adapter policy and represented as ordinary workflow effects.

BAML clients are generated from Armature `coerce` declarations and workspace
defaults in the first implementation. Policy validates whether the selected
providers, models, and environment variables are allowed.

## Resource Policy

External authority is declared through capabilities in workflow source:

```armature
capability plan = adapter("implementationPlan")
```

Adapters may expose file, database, command, or service operations, but each
operation must have a declared schema and capability requirement. Source code may
call only approved operations:

```armature
let planText = plan.snapshot()
plan.markBlocked(workItemId, reason)
```

Resource access levels used by adapters:

```text
none
read
write
read_write
```

Write access is never implicit, even in local mode.

## Validation Outcomes

Policy checks can produce:

```text
allow
allow_with_warning
deny
unknown
```

Mode determines how outcomes are treated:

```text
local       deny is error, unknown is warning
team        deny is error, unknown write/start is error
enterprise  deny and unknown are errors
```

Runtime enforcement still checks every effect before dispatch. A validation
warning is not a permanent grant of authority.

## Initial Policy Document

The first implemented policy document is intentionally small JSON:

```json
{
  "mode": "enterprise",
  "allowed_capabilities": [
    "adapter.agent.start",
    "message_agents",
    "resource.plan.read",
    "resource.plan.write"
  ],
  "denied_capabilities": []
}
```

Rules:

- capability names are exact strings
- capability names must be non-empty and contain no whitespace/control
  characters
- `denied_capabilities` always wins
- `allowed_capabilities` grants the exact capability
- unknown capabilities are warnings in `local`
- unknown capabilities are errors in `enterprise`
- unknown capabilities are errors in `team` for starts, messages, human
  obligations, `adapter.*`, and write-like capability names containing `.write`

This is not the final enterprise policy system. It is the first typed boundary
that lets `validate`, `overview`, `run`, `build`, `check`, and `emit-model`
apply the same supplied policy documents to manifest-required capabilities.
The manifest dispatcher also enforces the same supplied policy at runtime before
dispatching adapter-backed effects, so static validation is not treated as a
permanent grant of authority. Runtime policy denials return structured required
capabilities with the failed effect outcome, allowing `status`, `overview`, and
`log --json` to explain the authority boundary without parsing the error text.

## Diagnostics

Policy diagnostics should answer:

```text
what effect was requested?
which capability was required?
which layer allowed or denied it?
what declaration would fix it?
```

Example:

```text
Denied: start(worker) requires adapter.untie.start
Workflow declared: StartWorker
Workspace policy: allowed
Target worker policy: denied run_tests
Fix: remove run_tests from worker capabilities or update the target policy.
```
