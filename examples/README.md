# Examples

This directory contains workflow-first examples for the new `.armature` product
surface.

## Workflows

[`workflows/`](workflows/) contains native `.armature` source sketches:

- `minimal.armature` is a tiny statechart shape for parser/runtime scaffolding.
- `simple-supervisor.armature` is a compact director notification workflow for
  completion and idle observation events.
- `spec-implementation.armature` is the target managed spec implementation
  workflow.

The parser accepts both examples as static fixtures. Runtime execution is still
minimal: it currently processes the small durable event flow in
`minimal.armature`, while the spec implementation example drives parser,
validation, and later interpreter work.

## Policies

[`policies/`](policies/) contains JSON capability policy examples:

- `spec-implementation.enterprise-policy.json` allows the fake spec
  implementation adapter's required capabilities in deny-by-default enterprise
  mode.

## Managed Spec Orchestration

[`managed-spec-orchestration/`](managed-spec-orchestration/) contains the earlier
contract/script exploration that motivated the statechart workflow design. Treat
it as background material, not the new implementation center.
