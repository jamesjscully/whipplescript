# `std.telemetry`: read-side event and evidence export

Status: spec drafted 2026-06-14 ([`observability.md`](observability.md));
**concrete package design 2026-07-04** under the std-package campaign. Every
decision in [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md)
(M1–M8, E1–E7) binds this document; none are re-decided here.

> **Reserved-class prerequisites:** NONE (no construct instances; lowering
> class `none`). The v1 exporter core is SHIPPED (`whip otel-export` +
> `whip telemetry status|reset-cursor`, cli/main.rs:29895, :29791-29893);
> remaining work = drift reconciliation, Q2, allowlist v1, identity slices.

## Framing

**Telemetry is a read-side export package over core runtime records.**
Execution already records durable events, facts, effects, runs, artifacts,
diagnostics, and evidence; `std.telemetry` adds no workflow syntax and runs in
no rule body — it exports those records after the fact (durable log →
cursor-tracked exporter → telemetry backend). Execution must not depend on
exporter availability.

## Package Identity (operator-config package, E3)

`std.telemetry` is one of the two **operator-config packages** (with
`std.coercion`). Per the ecosystem shape's "Authoring surfaces vs
operator-config packages" decision, "package" here means, machine-checkably:
an **embedded manifest** (M5) contributing registry rows, a **CLI surface**,
and the **import advisory** posture — except that with no source constructs
there is no construct usage for the lint to fire on, so `use std.telemetry` is
legal, registers the library in the compile-time ContractRegistry, and is
otherwise inert; authors must never assume the `use` line gates export
authority (E5 risk). Today `std.telemetry` exists only in prose — absent from
the parser's stdlib list (parser/lib.rs:2700-2715), no manifest, no registry
rows; slice T3 ends that.

**Design-tracker checklist answers.** *Core functionality:* cursor-tracked,
emit-once, read-side export of durable runtime records to OTLP, plus export
status/cursor CLI. *Why it belongs:* export policy (mapping, cursor state,
attribute naming, redaction/allowlist, backend reachability) is operator
policy over core records, not core semantics or workflow authority; isolating
it keeps the execution hot path free of observability hooks. *NOT in the
package (core owns):* the durable event log; fact/effect/run, artifact,
evidence, and diagnostic records; source spans; causal ids and idempotency
keys; the trace/evidence JSON shape; provider-run lifecycle; `whip trace`/
`whip log`/evidence CLI. *The package owns:* exporter provider contracts;
OTLP mapping; cursor state; attribute naming; redaction/allowlist policy;
export status rendering and failure diagnostics. *Target feature set (v1):*
shipped exporter + Q2 + allowlist v1 + embedded-manifest identity.
*Dependencies:* core durable store read surface (`list_runs`/`list_effects`);
substrate slice S6 for T3; no other std package. *Provider expectations:* one
exporter provider, `otlp`; backends reached through the OpenTelemetry
Collector, never bespoke exporters ([`observability.md`](observability.md)
"One target, fanned out at the Collector"). *Naming-boundary questions /
verdict:* sections below; verdict = KEEP as a package, ship slices T1–T4,
defer the rest with cause.

## Surface

**Declarations:** none — no telemetry block, no rule-body export operation, no
workflow-level exporter hook; authors produce ordinary records, operators
decide whether/where/how they export. **Effect operations:** none —
`std.telemetry` contributes **zero effect kinds**; no `telemetry.*` string
ever enters the effect executor, the idempotency-key hash, or the admission
gate. **Capability ids:** none in v1 — the M3 id==kind rule is vacuous with no
effect kinds, and capability rows no enforcement point reads would be
decorative registry rows, the dishonesty M8's honesty audit catches (see
Deferred).

The whole surface is operator-plane:

```text
whip otel-export <instance> [--dry-run]     # shipped, cli/main.rs:29895
whip telemetry status [--json]              # shipped, cli/main.rs:29791
whip telemetry reset-cursor [<instance>]    # shipped, cli/main.rs:29791-29893
```

The `<instance>` argument is the **contract**: export is per-instance (the
earlier sketch showed no argument; reality wins — drift D1); store-wide sweep
deferred with cause. Environment surface (shipped subset marked):

```text
OTEL_EXPORTER_OTLP_ENDPOINT     shipped (cli/main.rs:29810, :30027)
OTEL_SERVICE_NAME               shipped (cli/main.rs:29953)
OTEL_EXPORTER_OTLP_HEADERS      Q2 slice — auth headers
OTEL_EXPORTER_OTLP_PROTOCOL     Q2 slice — validated, http/json only in v1
OTEL_RESOURCE_ATTRIBUTES        Q2 slice — parsed onto the OTLP resource
```

## Providers

One provider, classified per the M2 seam taxonomy honestly: the exporter is
**not an effect provider** — it never executes inside the effect lifecycle, so
none of M2's three effect-execution seams apply as shipped. It is an
operator-plane HTTP client invoked by the CLI after the store handle is
dropped (cli/main.rs:29921-29932; POST at :30084-30109). **Provider id
`otlp`**, contributed by the manifest. **Transport:** v1 moves from the
hand-rolled TcpStream HTTP/1.1 client to the in-tree `ureq` client (the coerce
precedent), gaining `https://` — the prerequisite for auth headers. **DO
plane:** DO export would re-enter through M2 seam (1), the HTTP sans-IO step
machine — to be registered in the DO tracker, not built here (drift D6). Registry
honesty: the provider row gets a **real reader** — the CLI resolves exporter
defaults from the row's `config_json` when a runtime store is present, with
env vars and flags as **operator-override-over-registry-default**, the same
rule M2 requires of `WHIPPLESCRIPT_COERCE_PROVIDER`. No row is written that
nothing reads.

## Manifest

Per M5, `std.telemetry` ships as an **embedded manifest**, validated by the
same pipeline as third-party manifests (cli/main.rs:16870-16912),
catalog-privileged, registered at worker start via `register_locked_packages`
(cli/main.rs:18953-18964). It contributes:

```text
libraries[]:    one library, id std.telemetry, standard:true —
                effect_contracts: [] and constructs: [] (both empty)
capabilities[]: [] in v1 (no enforcement point; see Deferred)
providers[]:    one row — "otlp", config_json = default endpoint + protocol
profiles[]:     []      workflow_tools[]: []
```

**Shipped-validator gap (in T3's scope).** As shipped, this manifest is
REJECTED by the very pipeline the T3 test requires it to pass:
`package_provider_contracts` (cli/main.rs:18133-18152) hard-errors unless
every `providers[]` row carries both `capability` and `provider_kind`, and
`validate_package_manifest_consistency` (cli/main.rs:16996-17020,
`validate_declared_capability`) requires that capability to be a member of
the manifest's declared `capabilities[]` — so satisfying the shipped rule
would force declaring exactly the decorative telemetry capability this design
withdraws. On the store side, `register_package_manifest` defaults a provider
row's effect kind to `capability.call` (`manifest_effect_kind`,
store/lib.rs:8113-8124) and writes it into `effect_providers` — the runtime
admission-gate table — which would violate this design's own Non-Goals line.
T3 therefore explicitly includes **amending the manifest schema and validator**
(owned by [`package-management.md`](package-management.md),
`whipplescript.package_manifest.v0`) to admit **capability-free operator-plane
provider rows**, and defining where such a row registers at runtime: either a
distinct operator-provider registry surface, or an `effect_providers` row
explicitly documented as admission-inert — never a defaulted `capability.call`
row. This package and `std.coercion` are the test case that E3's thin
operator-config shape **must become expressible in T3**; if the validator
rejects a construct-free library or a capability-free provider row, fixing the
schema is part of slice T3, not a reason to fake a construct or a capability.

## Construct Graph Contract And Capabilities

`std.telemetry` contributes no workflow construct instances. Its manifest
registers provider metadata only; it must not add rule-body effects, source
declarations, direct fact writes, scheduler hooks, or lifecycle states.
Family: none; lowering class: none; runtime entrypoint: operator command. The
earlier `ProviderKind<TelemetryExporter>` sketch is retired — no such type
exists, and a provider-kind taxonomy for one provider is premature
abstraction.

Telemetry authority is operator authority: importing the package grants
nothing; export credentials and allowlists live in operator config. The
sketched capability ids (`telemetry.export`, `telemetry.configure`,
`telemetry.cursor.write`) are **withdrawn from v1**: the capability plane's
only teeth are effect-admission gating (store/lib.rs:6873-6877 keys required
capabilities on effect kinds), and telemetry has no effect kinds — the rows
would bind nothing and gate nothing. They return, under the M3 one-namespace
rule, if a hosted control plane ever gives the CLI an enforcement point.

## Content Policy

Default export is structural only: ids, kinds, statuses, timings, counts,
source metadata (shipped: cli/main.rs:29982-29996). Fact field values, prompt
bodies, model responses, message text, file contents, memory contents, and
artifact bytes require explicit operator allowlists.

### Allowlist mechanism v1

Shipped reality is structural-only with **no allowlist at all** (drift D3);
the v1 mechanism is deliberately narrow. **Carrier:** an operator JSON file,
`--telemetry-allowlist <path>` (env `WHIPPLESCRIPT_TELEMETRY_ALLOWLIST`),
listing entries `"<Schema>.<field>"` — no source annotation; the operator owns
the cardinality/compliance budget ([`observability.md`](observability.md)
"Content policy: structural by default, operator allowlist for the rest").
**Scope:** only what v1 exports — effect **run input/output fields** typed by
a declared schema; facts, event logs, and artifacts wait for the
span-hierarchy work. **Validation:** entries validate against the compiled
program's schemas; an unknown schema or field is a config error aborting
before any network I/O; an allowlist without `--program` is refused (fail
closed). **Emission:** an allowlisted field exports as span attribute
`whipplescript.field.<Schema>.<field>`; everything else stays structural.
**Never allowlistable:** credentials/headers, artifact bytes, raw provider
request/response bodies.

## Auth Headers And Cursor Scoping (quick win Q2)

**Auth headers.** `OTEL_EXPORTER_OTLP_HEADERS` honored with standard OTel
semantics (`key=value,key=value`, URL-encoded values). Header values are
secrets: never printed by `--dry-run` or `status`, never written to the cursor
file, never exported. Headers over plaintext transport are refused unless the
endpoint host is loopback — authenticated export requires `https://`, which is
why Q2 moves the POST onto `ureq` (replacing cli/main.rs:30084-30109).
`OTEL_EXPORTER_OTLP_PROTOCOL` validates before export: `http/json` (the
shipped wire shape) accepted; anything else is a config error, not a silent
ignore. `OTEL_RESOURCE_ATTRIBUTES` parses onto the OTLP resource.

**Cursor scoping.** The shipped cursor (`<store>.otel-cursor.json`,
cli/main.rs:29933-29947) is keyed by instance only; changing endpoint or
mapping silently reuses it (drift D2). v1 re-keys per the original contract —
store, exporter provider, endpoint, mapping version:

```text
{ "version": 2, "cursors": {
    "<scope-key = H(provider, endpoint, mapping_version)>": {
      "provider": "otlp", "endpoint": "...", "mapping_version": 1,
      "instances": { "<instance_id>": ["<run_id>", ...] } } } }
```

Emit-once holds **per scope**: a new endpoint gets full history exactly once;
a mapping-version bump deliberately re-exports under the new mapping;
span/trace ids stay deterministic (stable_hash, cli/main.rs:29949-29952) so
re-export is idempotent at backends that dedupe by id. `status` lists every
scope; `reset-cursor` operates on one. A legacy v1 cursor file is **treated as
absent** — ignored on read, superseded on first write, no migration mechanism
(M4's posture: one-way pre-release breaks, blast radius = re-init). The cost
is one full re-export per scope, which is safe for the reason already stated:
span/trace ids stay deterministic, so re-export is idempotent at backends
that dedupe by id. The unbounded run-id list is a known limitation;
compaction deferred with cause.

## Contract-vs-Implementation Drift Register

Each item resolves in a v1 slice or defers explicitly.

- **D1 — per-instance CLI argument.** Sketch was store-wide; shipped requires
  `<instance>` (cli/main.rs:29895). Per-instance IS the contract.
- **D2 — cursor scoping weaker than contract.** Instance-only key. → Q2/T2.
- **D3 — allowlist absent.** → T4.
- **D4 — env surface incomplete.** Only ENDPOINT + SERVICE_NAME. → Q2/T1.
- **D5 — refusal doc-comment lie.** cli/main.rs:29782-29783 claims refusal
  without an endpoint; code defaults to `http://localhost:4318`, documented in
  docs/api-reference.md. The default stands; comment fixed in T1.
- **D6 — cursor side-file undefined on the DO plane.** The cursor is a
  filesystem side-file; the exporter reads via a direct store handle, never
  lifted through `RuntimeStore`; nothing in host-do touches telemetry.
  Stated honestly: **DO-plane export does not exist and this design does not
  make it exist** — and as of this writing no telemetry row exists in
  [`durable-object-runtime-tracker.md`](durable-object-runtime-tracker.md)
  either (its Phase 8 covers only the sidecar compute plane). Per M7's
  one-concern-one-tracker rule, a Phase-8-adjacent telemetry-export +
  cursor-as-store-table row is **to be registered** there; adding that row
  rides slice T2/T3, and until it lands this deferral is unregistered.
  Re-entry in Deferred.
- **D7 — no package identity.** Prose-only today. → T3.
- **D8 — export scope narrower than the observability contract.** Only
  provider runs export — no event-log spans, flow segments, span hierarchy, or
  evidence records ([`observability.md`](observability.md) "Trace Shape" /
  "Export Shape"). Deferred with cause, not silently.
- **D9 — one-shot only.** The "sidecar loop" never existed; the contract now
  says one-shot, sidecar deferred.

## Static Checks (M8: tier 2, hand-coded)

Per M8, `std.telemetry` brings **no catalog-flag-driven generic checks** and
sets no static-guarantee flags — no source surface, so nothing meets the rule
of three. Its checks are hand-coded, config-time, named here, and
**implemented and enforced by core** (E7): workflow checking ignores
`std.telemetry`; export config validates endpoint/protocol/headers before any
network I/O, and `--dry-run` runs the same validation; allowlists validate
against declared schema fields (unknown field = config error, no program =
refusal); cursor state is scope-keyed; exporter failure never blocks workflow
execution and never advances the cursor.

## Information-Flow Face

Posture per [DR-0029 (cross-package information flow)](decision-records/0029-cross-package-information-flow.md):
`std.telemetry` exports **no `@tool`** and brokers no workflow-plane crossing,
so it carries **no `information_flow` contract surface and makes no IFC
claim** — DR-0029's fail-closed default for claim-free packages applies. What
crosses the boundary is operator-plane egress. **Reads:** labeled durable
records, with operator authority, outside the membrane — the same trust
position as `whip trace` and the evidence CLI. **Egress sink:** the OTLP
endpoint is an external sink; structural-only is the label-respecting posture
— ids/kinds/statuses/timings are control-plane metadata, **content fields
carry data labels**, and allowlisting a field is an explicit, config-recorded
**operator declassification** of that field to the telemetry endpoint,
extending the shipped artifact-redaction discipline. **Never crosses:**
header/credential values; `redact`-dropped fields (absent from the record,
unexportable by construction); artifact bytes. **Integrity inbound:** nothing
flows back — the exporter ignores response bodies beyond status; telemetry
cannot become an unmodeled ingress door.

## Non-Goals

No rule-body export operation; no execution hot-path hooks; no content export
by default; no provider-specific backend zoo; no replay/recovery
double-export; no effect kinds, capability rows, or profiles — nothing enters
the runtime admission plane.

## v1 Implementation Slices

Each slice is independently gateable under the per-piece review discipline;
none depends on another except T3 → S6.

- **T1 — Q2 transport + env + validation** (parallel quick win). `ureq`
  transport with https; `OTEL_EXPORTER_OTLP_HEADERS` (secret handling +
  loopback-only-plaintext rule); protocol validation; resource attributes;
  pre-export config validation incl. `--dry-run`; fix D5. *Tests:* in-process
  collector asserts the received header; invalid protocol errors before any
  socket; headers absent from status/dry-run output; https path exercised.
- **T2 — cursor scoping v2. CLOSED: code landed 2026-07-04 (Q2 commit); Maude
  model built 2026-07-14** (`models/maude/telemetry-cursor.maude`, verdicts
  SSSNSS: full-export + fresh-scope re-export + isolation coverage, emit-once
  bite, and the demanded negative fixture — a shared-cursor module where
  serving a second scope rewinds the first and double-export becomes
  reachable). Scope-keyed cursor (`scope-key = H(provider, endpoint,
  mapping_version)`); legacy v1 cursor file
  ignored and superseded (no migration mechanism — one idempotent re-export);
  status lists scopes; reset-cursor per scope (a deliberate operator
  re-export, outside the model's invariant). All shipped and covered by
  `otel_export_rekeys_cursor_per_endpoint` + 4 sibling tests. *Tests:* endpoint change re-exports under the new
  scope without disturbing the old; failed POST advances nothing; a legacy v1
  file is ignored, superseded on write, and yields exactly one re-export.
- **T3 — package identity** (rides S6). Embedded manifest with empty
  constructs/effect_contracts; **manifest schema/validator amendment**
  admitting capability-free operator-plane provider rows (the shipped
  provider-requires-declared-capability rule and the defaulted
  `capability.call` → `effect_providers` write both block this manifest; see
  the Manifest section) plus a defined runtime registration target for such
  rows (distinct operator-provider surface, or an admission-inert
  `effect_providers` row — documented as such); provider row `otlp` with
  config_json defaults; CLI registry-default reader with env/flag override;
  stdlib registration. *Tests:* manifest passes the (amended) third-party
  validation pipeline; provider row present after registration and read by
  otel-export; **registration writes no admission-plane `capability.call`
  row** (Non-Goals holds machine-checkably); manifest-vs-code drift test per
  M5's lockstep rule.
- **T4 — allowlist v1.** Config carrier, schema-field validation, span
  attribute emission, fail-closed no-program refusal. *Tests:* allowlisted
  field appears as an attribute; non-allowlisted never does; unknown field is
  a config error; no allowlist = byte-identical structural spans.

## Deferred With Cause

- **Metrics + logs pillars** (lease contention, counter consumption-vs-cap,
  ledger append rate; logs-as-OTLP). *Cause:* no consumer demand; traces are
  the proving ground. *Re-entry:* [`observability.md`](observability.md) "The
  three pillars", forced when the experimentation subsystem's `gauge` plane
  needs an external metrics sink.
- **Event/span hierarchy** (event-log spans, flow segments, parentSpanId,
  evidence records — D8). *Cause:* requires unifying span construction with
  the `whip trace` builder; `otel_export` builds spans directly from runs, and
  growing that fork duplicates the trace shape. *Re-entry:* a slice refactoring
  export over the shared trace-building code, then widening scope.
- **Sidecar loop / `--all-instances`** (D1, D9). *Cause:* one-shot
  per-instance covers the dev loop; a daemon is operational surface nobody
  demands. *Re-entry:* first hosted deployment wanting continuous export; the
  scoped cursor already supports it.
- **DO-plane export + cursor-as-store-table** (D6). *Cause:* exporter never
  lifted through `RuntimeStore`; M7 keeps DO sequencing in the DO tracker
  (tracker row to be registered with slice T2/T3 — see D6).
  *Re-entry:* the DO tracker's package/compute phase; cursor moves behind the
  store trait, transport re-enters via the HTTP sans-IO seam.
- **Operator capabilities.** *Cause:* no enforcement point; decorative rows
  fail M8's honesty audit. *Re-entry:* a hosted multi-operator control plane
  gives the CLI an authorization check; ids return under the M3 namespace.
- **Second exporter provider / provider-kind taxonomy.** *Cause:*
  one-target-fanned-at-the-Collector is the decided posture. *Re-entry:* a
  backend the Collector genuinely cannot reach.
- **Cursor compaction** (unbounded run-id lists). *Cause:* dev-scale stores;
  compaction needs an ordered high-water mark — a real design. *Re-entry:*
  first operator-reported cursor-size pain; rides any cursor-format rev.

## Open Naming-Boundary Questions

- `whip otel-export` vs `whip telemetry export`: the split verb predates the
  `whip telemetry` group; folding it in is a one-way CLI rename (M4 posture).
  Flagged for the CLI-surface pass, not decided here.
- Provider id `otlp` vs `std.telemetry.otlp`: this design uses the bare id;
  the dotted form remains available if provider ids ever globalize.
- Boundary with core read surfaces: `whip trace`/`whip log`/evidence CLI stay
  core (E3 risk note); the line is "core renders local truth, telemetry
  exports it" — a future `whip telemetry tail` must not grow a second trace
  renderer.

## Spec Amendments

- [`observability.md`](observability.md), "`std.telemetry`: read-side export
  package": the "provider bindings" line in its Surface block is superseded —
  v1 contributes no capability or binding rows; this document's capability
  section is the contract. One-line pointer update.
- [`package-management.md`](package-management.md), manifest schema
  (`whipplescript.package_manifest.v0`): amended by slice T3 to admit
  capability-free operator-plane provider rows and to define their runtime
  registration target (admission-inert; no defaulted `capability.call`
  `effect_providers` row) — see the Manifest section's shipped-validator gap.
- No other substrate spec is amended: the ecosystem-shape note is consumed as
  decided (E3/M5/M7/M8); [`coerce.md`](coerce.md), [`files.md`](files.md),
  [`messaging.md`](messaging.md), [`event-ingress.md`](event-ingress.md),
  [`std-time.md`](std-time.md), and [`human-review.md`](human-review.md) have
  records read, not redefined.

## Modeling Notes

**Failure isolation:** exporter failure does not affect workflow execution.
**Emit-once:** successful export advances a cursor; failed export does not —
modeled in slice T2, the package's previously missing modeling artifact.
**Replay safety:** replay/recovery never re-emits telemetry through workflow
execution (holds by construction: read-side ambassador, no hot-path hooks).
**Redaction:** content is excluded unless operator config explicitly allows
it; a `redact`-dropped field is unexportable by construction.

Detailed evidence, trace, and OpenTelemetry mapping rationale lives in
[`observability.md`](observability.md).
