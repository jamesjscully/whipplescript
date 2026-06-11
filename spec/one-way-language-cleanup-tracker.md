# One-Way Language Cleanup Tracker

Status: complete

This tracker captures the syntax consolidation pass. The goal is to make
WhippleScript feel like a small language with one obvious authoring path, closer
to Go or Gleam than to a collection of equivalent conveniences.

Because the language has no external users yet, this tracker does not preserve
legacy aliases for the affected core syntax. The implementation should update
the language, examples, docs, snapshots, and tests together.

## Principles

- Prefer one canonical construct per concept.
- Keep the core mental model visible: rules observe facts and produce durable
  facts/effects.
- Do not keep alternate spellings just because they already exist.
- Keep advanced constructs only when they solve a different problem, not as a
  second spelling for the same problem.
- Make examples the style guide.

## Status Legend

- [x] Done and validated.
- [~] In progress or partially complete.
- [ ] Not started.

## Work Items

| ID | Change | Spec | Implementation | Examples/Docs | Validation | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| OWL-001 | Replace `matrix` with `table` | [x] | [x] | [x] | [x] | `table name as Class [ ... ]` is the only static source-row syntax. Generated rules and report metadata use `table` / `table_row` user-facing terms. |
| OWL-002 | Canonicalize effect sequencing | [x] | [x] | [x] | [x] | Only block-style `after effect succeeds/fails [as binding] { ... }` remains. `after ... =>` and `then ...` are rejected. |
| OWL-003 | Canonicalize assertions | [x] | [x] | [x] | [x] | `count(Query) == N` and `exists(Query)` are canonical. `one`, `none`, and `empty` are removed from parser/runtime support and examples. |
| OWL-004 | Add direct agent provider binding | [x] | [x] | [x] | [x] | Canonical agent form is `agent worker { provider codex ... }`. `harness` remains advanced indirection and is mutually exclusive with direct `provider`. |

## Decisions

### Static Source Rows

Canonical:

```whip
table tasks as Task [
  {
    title "Review parser"
    status "queued"
  }
]
```

Non-goal:

```whip
matrix tasks as Task [ ... ]
```

`matrix` should stop parsing as a declaration. If users paste old syntax, the
diagnostic should point to `table`.

### Sequencing

Canonical:

```whip
tell worker as turn """
Implement this task.
"""

after turn succeeds as completed {
  done task

  record CompletedTask {
    summary completed.summary
    status "done"
  }
}
```

Non-goals:

```whip
after turn succeeds => { ... }
then coerce reviewWork(turn.summary) as review
then done task -> record CompletedTask { ... }
```

### Assertions

Canonical:

```whip
assert count(Task where status == "queued") == 0
assert count(ReviewedWork where status == "reviewed") == 1
assert exists(Review where status == "accepted")
```

Non-goals:

```whip
assert none(Task where status == "queued")
assert empty(Task where status == "queued")
assert one(ReviewedWork where status == "reviewed")
```

### Agent Provider Binding

Canonical:

```whip
agent worker {
  provider codex
  profile "repo-writer"
  capacity 1
}
```

Advanced:

```whip
harness secure_codex: codex

agent worker using secure_codex {
  profile "repo-writer"
  capacity 1
}
```

Direct `provider` and `using harness` should be mutually exclusive.

## Validation Checklist

- [x] Parser rejects old `matrix` declarations and accepts `table`.
- [x] Parser rejects `then` sequencing.
- [x] Parser rejects `after ... =>` sequencing.
- [x] Parser/runtime rejects removed assertion aliases where they have special
  support.
- [x] Direct `agent provider` lowers into runtime provider selection.
- [x] Examples compile and IR snapshots are stable.
- [x] Docs/spec/companion skill teach canonical forms; `harness` is documented as
  advanced endpoint indirection.
- [x] Existing control-plane and acceptance fixtures pass with updated sources.

## Validation Run

Completed on 2026-06-09:

- `cargo test -p whipplescript-parser`
- `cargo test -p whipplescript-parser example_ir_snapshots_are_stable`
- `cargo test -p whipplescript`
- `scripts/check-report-schemas.sh`
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check`
