# WhippleScript documentation

User documentation for the WhippleScript language and the `whip` CLI. Design
records and implementation trackers live in [`spec/`](../spec/).

## Learn

- [Quickstart](quickstart.md) — install, run an example, inspect the result.
- [Tutorial](tutorial.md) — build a triage workflow with an agent and a human
  approval gate, from an empty file to a completed instance.
- [Concepts](concepts.md) — the execution model: facts, events, rules,
  effects, agents, providers, and workers.
- [Manual](manual.md) — the authoring guide: structuring workflows, modeling
  data, sequencing effects, and debugging runs.
- [Examples](examples.md) — the checked example catalog.

## Reference

- [Language reference](language-reference.md) — every construct in `.whip`
  source: declarations, rules, guards, effects, terminals, and static checks.
- [CLI & API reference](api-reference.md) — command syntax, JSON output
  shapes, status values, event types, and Rust crate APIs.
- [Runtime & operations](runtime-operations.md) — stores, instance lifecycle,
  effects and leases, provider failures, revision, and incident capture.
- [Providers & plugins](providers.md) — the fixture provider, native provider
  adapters, credential configuration, and plugin authoring.

## Help

- [Install](install.md) — binaries, source install, checksums, platform notes.
- [Troubleshooting](troubleshooting.md) — common first-session problems.
- [Current state](current-state.md) — what works today, what is experimental,
  and what stability claims mean in this repository.
