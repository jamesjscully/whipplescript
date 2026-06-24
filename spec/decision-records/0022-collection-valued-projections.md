# DR-0022: Collection-valued projections (the `export` row-source foundation)

Status: accepted 2026-06-17. Drives `std.files` `export` sub-piece #6, but the
mechanism is a general language foundation, deliberately built "the right way"
(operator steer: *set the foundations, don't come back to this later*).

## Problem

`export <format> <Schema> to <store> at <path> { rows <?> mode <mode> }` must name
the rows to serialize. WhippleScript rule bodies have **no first-class collection
value** today: projections appear only as guard/expect aggregates
(`count(X where …)`, `exists(X …)`, `X where …` as a boolean/scalar query). The
easy path — a per-row `export` in a `when <Schema>` fan-out rule that appends one
line per firing — was rejected: it builds the file incrementally across firings
(append-only, no atomic batch), reads as a side effect smeared over time, and does
not generalize. We choose to introduce a real collection value instead.

## Decision

Introduce a **collection-valued projection**: an expression that evaluates to a
typed, deterministically-ordered multiset of facts.

```
<Schema>                       -- all current facts of the schema type
<Schema> where <predicate>     -- those matching the guard/projection predicate
```

- **Type.** It has type `Array<Ref<Schema>>` (`IrType::Array` already exists). Its
  elements are the typed `<Schema>` facts; field access on an element uses the
  ordinary record-field machinery.
- **Evaluation.** Reuses the exact fact-matching the `count`/`where` projection
  aggregates already use (`ProjQueryKind::Where` predicate kernel) — but returns
  the matched fact *values* instead of a count/boolean. It is a **projection read**
  (lowering class `projection_source`, already in the catalog and modeled), so it
  inherits the existing provenance/source-span and liveness analysis.
- **Determinism / replay.** The collection is ordered by fact admission sequence
  (then `fact_id` as a tiebreak), so a given fact set always serializes to the same
  bytes — preserving the "replay does not re-read the filesystem; recorded output
  reproduces" contract.

### Scope of exposure (the "foundation vs. easy" line)

We build the **general machinery** (a real collection value + `Array` type +
evaluator + projection-read lowering), but in v0 **expose it only where a
collection is consumed** — the `export … { rows <collection-projection> }` clause.
We do **not** yet add general collection-typed bindings, `let`, or iteration
constructs. This keeps the foundation correct and reusable while bounding the
v0 surface; generalizing (collection bindings, `for each`, set operations) is a
purely additive step on the same foundation, with no rework — which is exactly the
"don't come back to redo it" goal. The evaluator and type are general from day one;
only the *grammar entry points* are conservative.

### `export` semantics

```
export <format> <Schema> to <store> at <path> {
  rows <collection-projection>     -- element type must be <Schema>
  mode <mode>                      -- create | replace | upsert | append
} as <binding>
```

- **Codecs:** v0 `jsonl` / `json` / `csv` — the inverse of the import decoders
  (`jsonl` = one JSON object per line; `json` = a top-level array; `csv` = a header
  row from the schema's fields then one record per row, with the same quoting rules
  as the import decoder).
- **Mode:** identical policy to `write` (no silent overwrite; `create` fails if the
  file exists, `replace` if it does not, `upsert` either, `append` appends). A mode
  violation is an ordinary `file.export.failed` routed to `after e fails`.
- **Boundary:** same as read/write — declared `file store`, root containment +
  `allow write` globs, refused before any disk access.
- **Lifecycle:** `export` is a write effect; exactly-once / replay safety comes from
  the existing effect lifecycle (admission-and-idempotency.md), like `write`. No
  fact-batch admission (that is import's inbound concern); export only *reads* facts
  and writes bytes. Settles `file.export.completed` with row count + content hash.

## Models

No new formal model is required: the collection projection is a **projection read**
(the `projection_source` lowering class is already modeled in
`lowering-class-lifecycle.maude` and the construct catalog), and `export` is an
ordinary write effect (the effect run/terminal lifecycle is already modeled in
TLA+/Maude). The model review for this piece is confirming the projection-read
lowering and the write-effect lifecycle cover it — not authoring a new search.

## Gated implementation slices

1. **6a — collection-valued projection (foundation).** `Expr`/parse entry for
   `<Schema> [where <pred>]` as a collection value; `Array<Ref<Schema>>` typing;
   evaluator returning the ordered matched fact values; projection-read lowering +
   liveness (a collection projection over `<Schema>` is a fact *read*, and marks the
   rule as consuming `<Schema>`). Tests: parse + type + deterministic evaluation.
2. **6b — `export` parser/IR.** `BodyEffectKind::FileExport { format, schema, store,
   path, rows_expr, mode }` + `parse_export` (validate codec + required mode + that
   the `rows` element type matches `<Schema>`); `IrEffectKind::FileExport`; formatter;
   reserved keyword.
3. **6c — `export` runtime.** `run_file_export_effect`: resolve the collection at
   commit (against the rule context), serialize per format, enforce mode vs on-disk
   state + boundary, write, settle `file.export.completed`/`.failed` with row count +
   hash. Worker dispatch.

## Consequences

- WhippleScript gains a real (if narrowly-exposed) collection value — the basis for
  later iteration/aggregation features, added without reworking this.
- `export` rounds out `std.files` v0 (read, write, import, export).
- Risk: exposing collections only in `rows` may feel asymmetric until general
  collection bindings land; accepted as a conservative-surface / correct-foundation
  trade.
