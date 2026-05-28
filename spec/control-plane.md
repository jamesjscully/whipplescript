# Control Plane

Status: draft

Armature runs many workflow instances concurrently. A `.armature` source file is
not itself a process. It compiles into a versioned program, and each execution is
a durable instance managed by the local or hosted control plane.

## Core Objects

```text
Program       compiled source plus version hash
Instance      one durable running copy of a program
Event         append-only observation for an instance
Fact          current materialized truth for an instance
Effect        durable request for external work
EffectEdge    durable dependency between two effects
Run           provider execution attempt for an effect
Capability    registered external authority or effect surface
Profile       policy bundle for agent/tool authority
Runtime       daemon/control-plane process
Artifact      durable file/log/output associated with a run
Evidence      causal record linking events, rules, effects, runs, and artifacts
Skill         deterministic context bundle for an agent or turn
Plugin        package-provided effect/fact-schema/resource extension
InboxItem     pending human review request
```

Every object that belongs to a running workflow is namespaced by:

```text
instance_id
```

The same program version may have many concurrent instances.

## Program Lifecycle

```text
source -> parse -> typecheck -> analyze -> verify/check -> compile -> deploy
```

A compiled program records:

- source path or source bundle identity
- source hash
- compiler version
- generated IR hash
- declared capabilities
- declared profiles
- declared skills
- declared fact/event schemas
- analysis results
- optional generated verification artifacts
- optional generated BAML artifacts

Deploying a program does not run it. Starting creates an instance.

## Instance Lifecycle

An instance has a durable state:

```text
created
running
paused
blocked
completed
failed
cancelled
```

The control plane may process multiple instances concurrently, but each
instance's rule commits are serialized. External effects may run concurrently
according to policy and capacity.

Pausing an instance means:

- no new effectful rule rewrites are committed
- already claimed provider runs may continue unless cancellation is requested
- incoming events are still recorded
- status explains the pause boundary

Stopping or cancelling an instance appends an event and transitions the
instance into a terminal control-plane state. It does not delete the log.

## CLI Shape

Target local CLI:

```sh
armature check workflow.armature
armature deploy workflow.armature --name spec-impl
armature start spec-impl --input input.json
armature dev workflow.armature --input input.json

armature ps
armature status <instance>
armature facts <instance>
armature log <instance>
armature effects <instance>
armature runs <instance>

armature pause <instance>
armature resume <instance>
armature stop <instance>
armature emit <instance> event.type --json payload.json

armature plugins
armature skills
armature inbox
armature trace <instance>
armature evidence <run-or-effect>
```

`armature dev` is a convenience command for dogfooding. It should compile,
start one instance, run local effect workers, and stream useful status.

## Control Plane Responsibilities

The control plane owns mechanical reliability:

- compile and validate programs
- create and recover instances
- serialize rule commits per instance
- append events
- materialize facts
- enqueue effects
- persist effect dependency edges
- lease and dispatch effects to workers
- record provider runs and artifacts
- record evidence linking events, rules, effects, runs, artifacts, skills, and
  policy decisions
- expose status/log/fact/effect views
- expose inbox, trace, evidence, plugin, and skill views
- pause/resume/cancel instances
- recover abandoned leases

It calls the kernel operations defined in [kernel-api.md](kernel-api.md) for
instance transitions. It should not duplicate rule-commit, effect-claim, or
completion semantics in ad hoc operational code.

The source language owns policy:

- when work should start
- which facts imply readiness
- when to ask humans
- when to retry or escalate
- which typed model decisions are needed

The control plane should not become a gateway that owns every integration.
Plugins and separate kernels own domain behavior; the control plane owns
durability, authority binding, visibility, and lifecycle.

## Concurrency Model

Concurrency exists at three layers:

1. Many instances may run at once.
2. One instance may have many outstanding durable effects.
3. Provider workers may execute effects in parallel subject to capability and
   profile policy.

Rule commits inside one instance are serialized. This avoids exposing authors to
distributed transaction bugs. Concurrency is expressed through durable facts and
effects, not simultaneous mutation of workflow state.
