# Agent Guide

The primary agent entrypoint is the local companion skill:
`skills/whipplescript-author/SKILL.md`
([web link](https://github.com/jamesjscully/whipplescript/blob/main/skills/whipplescript-author/SKILL.md)).

Use that file first. It explains the mental model, feature map, canonical
patterns, validation loop, and common mistakes. This page exists so the
website has the same route.

## Authoring Route

1. Read the companion skill.
2. Choose a checked example from [Examples](examples.md).
3. Use [Concepts](concepts.md) only if terms are unclear.
4. Use [Manual](manual.md) to choose between rules, flows, queues, child
   workflows, package capabilities, timers, and revision proposal patterns.
5. Use [Language reference](language-reference.md) for exact syntax.
6. Use [Diagnostics guide](diagnostics.md) when `check`, `dev`, or runtime
   inspection reports an error.
7. Validate with:

```sh
whip doctor
whip check workflow.whip
whip --json dev workflow.whip --provider fixture --until idle
whip --json trace <instance> --check
```

## Operating Route

If a workflow already ran, inspect state before editing source:

```sh
whip status <instance>
whip log <instance>
whip effects <instance>
whip runs <instance>
whip --json diagnostics <instance>
whip --json evidence <instance>
whip --json trace <instance> --check
```

To rewind an instance's context (file state, transcript, and event-log
position) as one coherence-checked cut, use `whip checkpoint <instance>` and
`whip restore <instance> <cut-id>`. To run a workflow on the cloud runtime, use
`whip deploy`.

Read [Runtime & operations](runtime-operations.md) for the full operator surface
(lifecycle behavior, checkpoint/restore, and cloud deployment) and
[Providers & packages](providers.md) for provider setup.
