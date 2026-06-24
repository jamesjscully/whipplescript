# JSON and JSONL ingestion: deterministic typed parsing at effect boundaries

Status: spec drafted 2026-06-10 from decided design
([`language-ergonomics-tracker.md`](decision-records/language-ergonomics-tracker.md) C3).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**One primitive: validate JSON against a declared schema, deterministically,
with no model.**

It is the deterministic sibling of `coerce`. `coerce` lifts *messy text* into
a typed shape through a schema-coercion backend, usually model-backed; coerce is
the current backend. JSON ingestion lifts *already-structured bytes* into typed
facts *with no LLM* — a total, replayable function. Both land typed values
against the same schema layer; the only difference is whether a model is
involved.

The primitive already has three call sites; it wears three hats:

- `--input` at workflow start (seeds a typed fact).
- An effect that returns raw text (`exec` stdout; future HTTP/file reads).
- An injected external event payload ([`event-ingress.md`](event-ingress.md)).

This spec covers the **effect-output** hat. The gap it closes: today `exec`
returns `stdout` as an opaque string, so a script that emits JSON is a
dead-end — your only options are a `coerce` (absurd: an LLM call to read bytes
that are already structured) or string surgery (declined,
[C2](decision-records/language-ergonomics-tracker.md#c2-considered-and-declined)).

JSON ingestion adds no general computation. It is a data primitive sufficient
for workflow execution and nothing more: bytes crossing an effect boundary
become typed state. There is no in-language JSON manipulation, querying, or
construction — production of outbound JSON stays covered by `record`/payload
construction serialized into effect inputs.

Because this primitive is the deterministic sibling of `coerce`, it is also the
**deterministic validation capability** the e2e plan calls for
([`e2e.md`](e2e.md)): a non-LLM checker invoked as `exec "<validator>" ->
Schema` emits a typed verdict that rules branch on, with no provider access, so
exact script/format/fixture properties can be asserted in CI alongside
model-judged `coerce` review. See `examples/deterministic-validation.whip`.

## Surface

The effect-output type is declared with `->`, read as "typed as", consistent
with `coerce fn(...) -> Type` and `decide "..." -> { ... }` (the core
anonymous-coercion sugar defined in [`language.md`](language.md)).

### Single value

```whip
exec "scripts/report.sh" -> Report as x

after x succeeds as r {
  complete result { total r.count }
}
after x fails {
  fail error { reason "report.sh produced no valid Report" }
}
```

`x` binds the typed `Report` in the success branch. The effect's success
condition becomes **exit 0 AND stdout parses as `Report`**; a non-zero exit or
a parse/validation failure both route to `after x fails` — one branchable
outcome, reusing the effect lifecycle. The top-level JSON must be a single
object conforming to the schema.

### Stream (JSONL or top-level JSON array)

```whip
exec "scripts/list-items.sh" -> each WorkItem

rule handle
  when WorkItem as item
=> { ... }
```

`-> each WorkItem` parses stdout as a stream of `WorkItem` and records **one
fact per line/element**, reacted to by ordinary per-fact rule fan-out (no
loops — fan-out is the `when` trigger's job, as everywhere else). `each`
abstracts over JSONL (one object per line) and a top-level JSON array; there is
no collection binding because there are many values. A malformed line fails the
effect (all-or-nothing: either every line lands or the effect fails, so a
partial stream never half-commits).

`Report`/`WorkItem` may be any declared schema, including a sum type
([`sum-types.md`](sum-types.md)) — a JSONL line can be a tagged variant.

## Mechanics

- `->` parse target attaches to any effect that returns raw text. v1: `exec`.
  (`coerce`/`call` already return typed values; they do not take `->`.)
- The parse runs at the **effect-result boundary** on the worker pass, after
  the command terminates, before the terminal fact is recorded. It is a total
  deterministic function of the bytes; it never reads the clock or external
  state.
- Single: the validated value is the effect's success payload, bound by the
  `after ... succeeds as` clause exactly like `coerce` output.
- Stream: each validated element is recorded as a fact of the target schema
  with `provenance_class: "ingest"`, carrying the source effect id; rules
  match them as normal facts. Standard fact identity/idempotency applies so a
  re-run of the same effect does not double-record.
- Failure: a parse or schema-validation error makes the effect `failed` with
  the validation detail as failure evidence; `x.stdout` (raw) remains
  available on the failure for diagnosis.

## Typed input to `exec`

`->` types the bytes coming *out* of an effect; its mirror types the bytes
going *in*. Interpolating values into command strings
(`exec "triage.sh {{ r.payload }}"`) is quoting-fragile — precisely the
string-surgery bug class this spec exists to remove. Hosted script
capabilities feed a typed record to the script's stdin as JSON:

```whip
exec triage_script with r -> each WorkItem
```

`with <binding>` serializes the bound record to JSON on the command's stdin
(the same serializer that records effect inputs — one canonical encoding).
Typed bytes in, typed facts out: `exec` becomes a typed pipe in both
directions. In hosted mode the `with` form is the *only* way to hand a script
data — the command string itself is replaced by a content-pinned capability name
([`script-capabilities.md`](script-capabilities.md)).

## Static checks

- A `->` parse target must name a declared schema; an unknown schema is a
  check error.
- `-> each T` is legal only where per-fact production is meaningful (rule/flow
  effect position); the bound `as x` form is rejected with `each` (a stream
  has no single binding), and `-> T as x` is rejected without a binding when
  `T` is consumed as a value.
- Reading raw `x.stdout` and reading the parsed `x` are mutually consistent:
  on a parsed single effect, `x` is the typed value and `x.stdout` is the raw
  string (available chiefly on the failure branch).
- No untyped JSON access: there is no `x.someField` on an unparsed string and
  no path expression that reaches into raw JSON. Want structure, declare a
  schema.

## Dependencies

Reuses the schema/validation layer that backs `coerce` output and table seeds.
Composes with sum types ([`sum-types.md`](sum-types.md)) and is the shared
parser behind injected event payloads
([`event-ingress.md`](event-ingress.md)). No new runtime concept: a parsed
effect is an ordinary effect whose success condition includes schema
conformance.

`std.files` reuses the same deterministic validation rules for `import json`
and `import jsonl`; it owns file-store authority and path policy, while this
spec owns the byte-to-schema validation semantics.

## Modeling notes

- Determinism: the same bytes against the same schema always produce the same
  typed value or the same failure (total function; property test over
  fixtures).
- Boundary discipline: parsing occurs only at the effect-result boundary;
  guards never parse. Replay re-reads the recorded typed fact, not the bytes.
- Stream atomicity: `-> each T` over N lines records all N facts or fails the
  effect; no partial-stream commit (golden test over a malformed-mid-stream
  fixture).
- Failure routing: a malformed payload lands `failed` and routes to
  `after ... fails`, never an untyped or partial fact.
