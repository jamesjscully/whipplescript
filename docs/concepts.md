# Concepts

This page explains the core WhippleScript terms before you need the full
language reference.

## Workflow

A workflow is the durable boundary for a unit of agent work. Starting a workflow
creates an instance with its own event log, facts, effects, provider runs,
evidence, and lifecycle state.

## Fact

A fact is typed workflow state. Rules match facts, create new facts, and consume
facts when work moves forward.

Use facts for things like queued tasks, reviewed results, approvals, failures,
and summaries.

## Event

An event records something that happened. The event log is append-only and is
the source of truth for projections such as current facts, effects, runs, and
status.

Examples include `external.started`, `rule.committed`, `effect.terminal`,
`workflow.completed`, and `human.answer.received`.

## Rule

A rule is deterministic orchestration policy:

```whip
rule dispatch
  when WorkItem as item where item.status == "queued"
  when worker is available
=> {
  tell worker as turn "Do {{ item.title }}"
}
```

Rules decide what should happen next. They do not directly call external
providers.

## Effect

An effect is a durable request for external work. Agent turns, typed model
decisions, human review requests, plugin calls, and child workflow invocations
are all effects.

Effects make agent workflows inspectable: you can see what was requested, which
provider tried to run it, whether it completed, and what evidence was recorded.

## Agent

An agent is a logical target declared in source:

```whip
agent codex {
  profile "repo-writer"
  capacity 2
}
```

The workflow routes work to logical agents. Provider configuration decides how a
logical agent maps to real execution.

## Provider

A provider executes effects. The fixture provider is deterministic and local.
Real providers, such as Codex-style or Claude-style agent adapters, are the
experimental bridge to actual agent systems.

Use the fixture provider first so you can validate orchestration before
debugging credentials or external tools.

## Worker

A worker claims ready effects and completes them through a provider. The worker
does not invent workflow policy; it only executes effects that rules already
materialized.

## `run`, `step`, `worker`, And `dev`

These commands are separate on purpose:

| Command | What it does |
| --- | --- |
| `run` | Starts an instance and records the start event. It does not evaluate rules or providers. |
| `step` | Evaluates deterministic rules and materializes facts/effects. It does not execute providers. |
| `worker` | Claims ready effects and completes them through a provider. |
| `dev` | Local validation loop that composes `run`, `step`, and fixture workers, then evaluates assertions. |

For first experiments, use `dev`. Use `run`, `step`, and `worker` separately
when you want to inspect each runtime boundary.

## Fixture Provider Vs Real Providers

The fixture provider gives deterministic completions for local validation. It is
the right default for tutorials, tests, and workflow design.

Real providers connect to external tools or agent systems. They are still
experimental in this project, and their configuration and behavior may change.

## Skills And Plugins

A skill is context attached to an agent or turn. It does not extend the
language.

A plugin registers capabilities, providers, schemas, profiles, resources, and
optional skills. Plugins should expose explicit effects rather than hidden
control flow.

## The Mental Model

```text
facts/events + rules -> durable facts/effects
effects + workers    -> provider runs
provider results     -> events/facts
workflow terminals   -> completed/failed instances
```

WhippleScript is useful when the route, review, retry, and audit trail matter as
much as the individual model response.
