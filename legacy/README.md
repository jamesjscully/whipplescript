# Legacy WhippleScript Work

This directory preserves prior WhippleScript designs and implementations. They are
reference material, not the current product model.

## Contents

- `v0.3-runtime/`: older task/service/script runner work.
- `statechart-workflows-runtime/`: the Rust statechart workflow runtime and
  specs that preceded the current rule-machine reset.

## Reuse Policy

Legacy code may still provide useful implementation material:

- Rust workspace and CLI patterns
- process/log capture code
- SQLite persistence code
- event/effect terminology where it remains accurate
- tests and packaging lessons
- formal-model scaffolding

It should not define the new language or runtime architecture. Current design
work starts from the root `spec/` directory and targets an event-sourced
relational rule machine.
