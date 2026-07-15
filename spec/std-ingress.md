# `std.ingress`: typed signal admission — concrete package design

Status: concrete package design 2026-07-04 (std-package campaign, Process
step 6). Constitution: [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md)
(M1–M8/E1–E7 bind this design). Substrate spec:
[`event-ingress.md`](event-ingress.md) — semantics live there; this document
adds package identity, provider execution, and build slices only.

## Design-tracker answers

- **Core functionality.** The typed boundary where outside observations become
  durable signal facts: `signal` declarations (admission contracts), `source`
  declarations (provider → observation → explicit emit mapping), directed peer
  injection (`emit signal … to`), and the operator admission door
  (`whip signal`). All of this is SHIPPED end-to-end for the cli path
  (parser/lib.rs:18246-18304, :18716-18841; body.rs:2818-2869;
  kernel/effect_handlers.rs:1649-1800; main.rs:29322-29484). What is NOT
  shipped is source-provider execution: the only source-resolving worker pass
  skips every non-clock source (main.rs:20831).
- **Why it belongs.** Ingress is a semantic domain (constitution "Semantic
  domains vs provider catalogs"): it owns the signal/source nouns, the
  admission contract, and the least-magic explicit-mapping rule. Replay
  cleanliness (replay reads the recorded fact, never the network) is a
  package-level promise, per "Framing" in `event-ingress.md`.
- **What is NOT in the package.** Clock/recurrence grammar and the clock
  provider (std.time); generic Message envelopes and `source interaction`
  (std.messaging, per "Relationship To `std.messaging`"); file CONTENT
  read/write/import/export (std.files, per "Relationship To `std.files`");
  the core admission machinery itself — unique-index dedup, H8 integrity
  gate, fact derivation — which is core lifecycle per constitution E7; timers
  and `timeout` (core, spec/time.md).
- **Target feature set (v1).** Q1 static check; provider config clauses in
  the source grammar; executing drivers for cli (shipped), stdio, file, http;
  `--delivery-id` on `whip signal`; embedded manifest identity exercising the
  `signal_source` lowering class (E4).
- **Dependencies.** Core: admission core + events unique index
  (migrations/0001:75-76), H8 gate (ifc.rs:1063), worker source pass. Substrate
  slices: S1 (`event.notify` → `signal.emit` rename rides ahead of every slice
  here), S6 (embedded manifests — which for this package must ALSO ship the
  catalog privilege additions in "Catalog privilege additions this manifest
  requires": reserved-keyword tuples for `signal`/`emit` → std.ingress plus
  a privilege mechanism for the non-package-authorable lowering classes
  `metadata`/`signal_source`/`signal_emit`). Sibling packages: std.time (shared
  `source_declaration` family), std.files (content plane), std.messaging
  (envelope plane).
- **Provider expectations.** Each provider row declares kind, transport, auth
  modes, dedup key source, correlation strategy, and can-start-instances per
  "Provider Contract" in `event-ingress.md`; v1 encodes these as static
  manifest data, with probed capability-report machinery deferred demand-side
  per constitution E3.
- **Open naming/boundary questions.** (a) Resident-driver command name —
  `whip ingress serve` chosen here, picked once (E1 discipline). (b) Source
  header provider kinds stay bare idents (`source http as …`); dotted
  qualification reserved for a cross-package kind collision, which is a check
  error until then. (c) `signal` declaration is core grammar authorized to
  std.ingress post-parse (M1); if Jack reverses M5, it degrades to
  compiled-in with no surface change.
- **Verdict.** BUILD. Shipped core surface + a real execution gap + a
  security-relevant missing check make this a high-value, low-invention v1.

## Surface

Declarations and operations (all grammar core-owned per M1, authorized
post-parse against the std.ingress manifest):

- `signal <dotted.name> { fields }` — admission contract
  (parser/lib.rs:18246-18304). Declaring one registers std.ingress today
  (parser/lib.rs:2705-2707); under M5 the embedded manifest owns the
  registration instead.
- `when <dotted.signal> as x` — typed reaction (parser/lib.rs:7662-7679).
- `source <provider> as <name> { <config clauses> observe as <b> emit <signal> { … } }`
  — source_declaration family, lowering class `signal_source`
  (parser/lib.rs:18716-18841). NEW in this design: provider config clauses,
  so the "External Source" example in `event-ingress.md` parses:

  ```text
  path <string>                      http: endpoint path
  auth <mode> secret <ident>         http: hmac|bearer|shared; ident = env-style secret REFERENCE
  correlate <observation-path>       http: maps a delivery to a target instance id
  dedup <observation-path>           optional: provider delivery-id source; default = payload hash
  watch <glob-string>                file: watch pattern under the watched root
  ```

  Clause set is closed per provider kind (see Static checks). No source-level
  secret literals ("Non-Goals" in `event-ingress.md`).

  **Reality reconciliation (2026-07-15, I2a as built).** Between this design
  and the build, POLLING file/http drivers shipped with their own clauses:
  `file` reads `path <string>` line-by-line (one signal per non-empty line)
  and `http` GETs `url <string>` (one signal per JSON-array element) — both
  keyed by positional ordinal. I2a landed on that shipped surface rather than
  the table above: `watch <glob-string>` is the file OCCURRENCE mode (exactly
  one of `path`/`watch`; one signal per new (path, content-hash) occurrence,
  observation `{path, content_hash, watch}`) and
  `dedup <observe>.<field>` is the provider delivery-id source for `file`
  (line mode) and `http` (replaces the positional key; a watch source is
  already content-keyed, so `dedup` is rejected there). The table's
  `path`-as-endpoint, `auth`, and `correlate` clauses belong to the INBOUND
  listener and defer WITH slice I4 ("Deferred with cause") — building
  secret/auth grammar with no listener to consume it would be dead surface.
- `emit signal <name> to <target> { payload } as b` — directed peer injection
  (body.rs:2818-2869).

Effect operations, EXACT kind strings (post-S1; the wire kind today is
`event.notify`, parser/lib.rs:3628, kernel/lib.rs:3017 — S1 renames it and
purges the legacy `event.emit` kind + line-based fallback, parser/lib.rs:3617,
:15510, kernel/effect_handlers.rs:34-132, main.rs:20523):

| operation | effect kind | terminal facts | capability id (M3: id == kind) |
|---|---|---|---|
| `emit signal … to` | `signal.emit` | `signal.emit.completed` / `signal.emit.failed` | `signal.emit` |

Source admission is NOT an effect operation: sources have no rule-body form
and no claim/run/settle lifecycle. They append signal facts through the
admission core only ("No direct fire", `event-ingress.md` Modeling Notes).
Consequently `signal.emit` is this package's only effect kind and only
capability id. No `ingress.*` operator capabilities are invented; driver
authority is operator authority, like std.telemetry's CLI surface.

## Providers

M2's three seams govern OUTBOUND effect execution. Ingress providers are
INBOUND admission drivers — a genuinely different mechanism, kept separate
(house rule: avoid premature unification). The load-bearing invariant: **every
driver converges on one shared admission function** (extracted from the
`whip signal` path, main.rs:29322-29484 — schema validation, H8 gate,
idempotency key, provenance, `derive_fact`), so no provider ever becomes a
second door around validation or integrity carriage.

- **`std.ingress.cli`** — shipped. One-shot operator door (`whip signal`),
  no resident process. Gains `--delivery-id` (slice I5), completing the
  "CLI Admission" contract in `event-ingress.md`.
- **`std.ingress.stdio`** — resident operator driver: `whip ingress serve
  --stdio [--program …]` reads JSONL envelopes
  `{instance, signal, payload, delivery_id?}` from stdin, validates, admits.
  Dev/test reference path. Native-only; no M2 effect seam involved.
- **`std.ingress.file`** — worker-pass poller, the clock-source execution
  precedent (main.rs:20823-20913): the source pass drops its
  `!source.is_clock` skip for file sources (and relocates kernel-side —
  landing site named in slice I3), polls `watch` globs, admits one
  signal per new (path, content-hash) occurrence with file evidence recorded.
  Correlation default: admit to running instances of the declaring program —
  the clock-source targeting rule. Content reading stays std.files
  ("Relationship To `std.files`"). Native filesystem only in v1: the FileStore
  seam has no directory listing (store/files.rs:20-58); DO counterpart
  deferred below.
- **`std.ingress.http`** — resident listener in `whip ingress serve`:
  hand-rolled HTTP/1.1 over TcpStream, the std.telemetry exporter precedent
  (main.rs:30084-30109), routing on each source's `path`, enforcing `auth`
  (hmac/bearer/shared secret via env-style references), delivery id from
  `dedup` else payload hash, instance from `correlate`. Cleartext limited to
  localhost/behind-proxy per "`std.ingress.http`" in `event-ingress.md`. On
  the DO plane the natural driver is the DO fetch handler — registered in the
  DO tracker per M7, not designed here.
- **`std.ingress.grpc`** — DEFERRED (see below); the seam analysis warrants
  it.

## Manifest

Per M5, std.ingress becomes an embedded manifest (rides S6) contributing:

- **libraries**: `std.ingress` with effect_contracts: `signal.emit`
  (effect_operation family, lowering class `signal_emit`, core/lib.rs:78;
  required_capabilities `["signal.emit"]`).
- **constructs**: `signal` (declaration_block, metadata lowering), `source`
  (source_declaration, lowering class `signal_source` — this registration is
  the class's exerciser per constitution "Lowering classes"), and the
  `emit signal … to` effect_operation.
- **capabilities**: `["signal.emit"]`.
- **providers**: `std.ingress.cli` / `.stdio` / `.file` / `.http` rows, kind
  strings `cli`/`stdio`/`file`/`http` matching the bare source-header idents;
  `config_json` carries each provider's static "Provider Contract" descriptor
  (transport, auth modes, dedup source, correlation strategy,
  `can_start_instances: false` — see Deferred).
- **profiles**: none.

The clock provider row belongs to std.time's manifest, not this one.

### Catalog privilege additions this manifest requires (S6 obligation)

The manifest above is REJECTED by the shipped validation pipeline M5 requires
it to pass — roughly five hard errors today, so slice I2b cannot land on S6
as currently specified without the following catalog data, which is exactly
this design's deliverable (analogous to std.tracker's existing
claim/renew/release rows):

1. **Reserved-keyword privilege tuples.** `signal` and `emit` are reserved
   keywords (core/lib.rs:554-558) whose only privilege tuples today grant
   claim/renew/release to std.tracker (core/lib.rs:560-582), so the construct
   rows above trip `reserved_keyword_privilege_error`
   (cli/main.rs:13217, :17162). S6 must add tuples granting
   (`signal`, std.ingress, declaration_block, lowering `metadata`) and
   (`emit`, std.ingress, effect_operation, rule-body scope, lowering
   `signal_emit`). `source` is not reserved; it needs no tuple.
2. **Lowering-class authorability for catalog-privileged manifests.** All
   three lowering classes this manifest uses — `metadata`, `signal_source`,
   `signal_emit` — are `package_authorable: false` (core/lib.rs:340-433;
   only metadata_only/capability_call/typed_effect_call are authorable), so
   each construct row errors "platform-internal and cannot be used by
   package constructs" (cli/main.rs:13200-13204, :17082-17087). M5's
   "catalog-privileged" anticipated keyword tuples but says nothing about
   lowering authorability; S6 must close that gap with one of two
   mechanisms — extend the privilege tuple so a named (library, construct)
   pair may target a non-authorable lowering, or flip these classes'
   authorability scoped to embedded std manifests only. Which mechanism is
   S6's call (registered there, not decided here); this design requires only
   that the privilege stays a catalog fact and that the no-squatting property
   survives the std-construct-authorization.maude re-model already obligated
   on S6.

Both items are registered as explicit S6 obligations (see Dependencies and
slice I2b).

## Static checks

All tier-2 hand-coded core checks per M8 (no rule-of-three triplicate exists
for any of these), named here as this package's demands which core implements
(constitution E7):

1. **Q1 — emit names a declared signal, and the emit block materializes the
   declared payload type** from the `observe` binding and in-scope recorded
   values. Verified missing today: a source emitting undeclared
   `deploy.finished` with a bogus field passes `whip check` clean (state
   survey, verified live). This is the campaign's Q1 quick win.
2. **Provider kind is known**: the source header kind must be contributed by
   an embedded/locked manifest — hard error once S6 lands (today any ident is
   accepted, test parser/lib.rs:28035). The `use std.ingress` import itself is
   an ADVISORY lint per the M5 graduated ladder.
3. **Config-clause validation per provider kind** (closed sets): `http`
   requires `path` + `auth` + `correlate` (correlate mandatory in v1 because
   instance-starting is deferred), `auth` secret must be a reference, never a
   literal; `file` requires `watch`; `cli`/`stdio` accept no clauses; unknown
   clause anywhere is an error.
4. Existing clock checks (recurrence/missed/timezone,
   parser/lib.rs:18798-18807) remain std.time's demands; not restated here.

## Information-flow face

Posture per DR-0029 "Decision": std.ingress exports no `@tool`, so it carries
no `information_flow` contract — absent means no IFC claim, and consumers'
fail-closed defaults govern. What crosses this boundary:

- **Inbound labels.** Every admitted signal is a low-integrity external
  source unless internal-marked; H8 signal integrity carriage is shipped and
  modeled (ifc.rs:559, :1063; models/maude/infoflow-signal-carriage.maude):
  internal-governed signals refuse external injection. Because all new
  drivers route through the one admission core, they inherit this gate by
  construction — a driver that bypassed it would be a new unmodeled door and
  is forbidden by this design.
- **Cross-instance authority.** `signal.emit` peer injection validates the
  payload against the TARGET program's declaration and refuses cross-package
  internal targets (E2-DYN, test main.rs:45441). The emitting instance's
  provenance (origin instance + effect key) rides the admission record per
  "In-Workflow Signal Injection".
- **Secrets.** Source `auth` config admits only secret references resolved at
  the driver boundary; secret values never enter IR, facts, or evidence.

## Boundary with `std.time` (E2)

Stated here and mirrored in `std-time.md`; drift between the two is the E2
risk.

- **Shared family, split providers.** Both packages contribute to the
  `source_declaration` family: std.ingress owns external delivery kinds
  (cli/http/stdio/file, lowering `signal_source`); std.time owns the `clock`
  kind (lowering `clock_source`) plus ALL recurrence/timezone/missed grammar
  and ClockPolicy.
- **Core owns the source-resolving worker pass** and the admission core
  (validation, dedup index, H8, fact derivation) — per constitution E7 these
  are lifecycle invariants no package opts out of. Packages contribute
  provider kinds the pass drives. Slice I3 generalizes that pass from
  clock-only to clock+file; std.time's design must claim no ownership of the
  pass itself.
- **Neither package fires rules directly**; both emit typed signal facts
  through the same admission boundary ("Relationship To `std.time`").

## Build status (2026-07-15)

- **I1 — SHIPPED** (pre-dates this pass): emit-names-a-declared-signal +
  observation-field checks, parser-side
  (`validate_source_emit_signal_declared`, rule-side
  `validate_emit_signal_declarations`).
- **I2a — BUILT, reconciled to the shipped poller surface** (see "Reality
  reconciliation" under Surface): `watch` + `dedup` clauses in the hand
  parser (`source` is a hand-parsed structural exception; no grammar-table
  rows), closed-set validation, fmt idempotency, IR fields
  (`IrSource::watch`/`dedup_field`). The `auth`/`correlate`/http-`path`
  clauses defer with I4.
- **I2b — BUILT**: embedded `std/manifests/ingress.json` (signal.emit
  contract mirroring the parser-compiled shape; `signal`/`source`/`emit`
  construct rows; capability/provider/binding rows making the NATIVE
  `signal.emit` admission gate real — builtin exemption deleted from
  store `policy_block_on`, DO mirror keeps its exemption per M7); catalog
  privilege tuples for (`signal`, std.ingress, declaration_block,
  `metadata_only`) and (`emit`, std.ingress, effect_operation,
  `signal_emit`) in core/lib.rs (NOTE: the design sketch said lowering
  `metadata` for `signal`; `metadata` is not declaration_block-compatible —
  the shipped decl-block lowering is `metadata_only`, the
  std.tracker/std.coord precedent); operator-plane provider rows carry the
  static Provider Contract descriptors (cli/stdio/file/http; the clock row
  landed in std/manifests/time.json). Provider-kind-known hard check ON
  (`validate_source_provider_kinds`, kinds derived from manifest operator
  rows); import advisory `lint.missing_ingress_import` ON. The
  authorability-door mechanism S6 was obligated to provide already existed
  (privilege-tuple leg, modeled `[door-privileged]` in
  std-construct-authorization.maude) — no re-model needed, tuples are data.
- **I3 — BUILT**: shared admission core
  `kernel/src/ingress_pass.rs::admit_external_signal` (declared-signal check,
  H8 gate, IR-typed payload validation — `validate_json_for_object` moved
  kernel-side, CLI re-exports — delivery-key idempotency with duplicate
  ABSORPTION via the new `RuntimeStore::event_by_idempotency_key`, fact
  derivation); the file/http source passes relocated kernel-side generic over
  the store traits with native I/O seams (`IngressFileIo`, fetch closure
  carrying the SSRF screens; positional keys byte-compatible with the
  pre-lift cursor keys, and the cursor itself is gone — the key lookup also
  fixes its head-insert desync); `whip ingress serve --stdio` admits JSONL
  envelopes `{instance, signal, payload, delivery_id?}` (shell CLI-side).
  admission.maude extended with the delivery-id + file-occurrence key forms
  and bite fixtures.
- **I4 — DEFERRED** (moved to "Deferred with cause").
- **I5 — BUILT**: `whip signal --delivery-id` wins over the derived payload
  hash; the duplicate is absorbed once ACROSS process runs with an observable
  diagnostic (`"duplicate": true` under `--json`).

## v1 implementation slices

Each independently gateable under the per-piece review discipline; all assume
substrate S1 (rename) has landed. Declared dependencies beyond S1: I2b rides
S6 (manifests + the catalog privilege additions above); I3 and I4 need I2a's
clause grammar (`watch` for the file driver; `path`/`auth`/`correlate`/`dedup`
for http) — neither can pass its own e2e gate without it. I2a, I5, and I1
depend on nothing else.

- **I1 — Q1 static check.** Emit-names-a-declared-signal + payload
  materialization typing. Tests: the verified-live negative fixture (undeclared
  signal, bogus field) becomes a check error with a routes-to-fix span; all
  shipped examples (examples/clock-source.whip, event-bridge.whip) stay green.
  No model (pure static check).
- **I2a — provider config clause grammar.** Parse the closed clause sets; the
  "External Source" HTTP example in `event-ingress.md` parses, lowers,
  formats idempotently (`whip fmt`), and lands in an .ir snapshot; the
  config-clause-validation check (Static checks #3) turns on. No S6
  dependency. Tests: parser accept/reject per clause set; snapshot. No model
  (pure grammar + static check).
- **I2b — manifest identity (rides S6).** Embedded std.ingress manifest rows
  land, TOGETHER WITH the catalog privilege additions named above
  (reserved-keyword tuples for `signal`/`emit` → std.ingress; the
  lowering-authorability privilege mechanism for
  `metadata`/`signal_source`/`signal_emit`) — without them the manifest is
  rejected by the shipped pipeline. Provider-kind-known check (Static checks
  #2) turns on. Tests: manifest drift test; a privilege-acceptance test in
  the std-tracker.json pattern (cli/main.rs:37468); negative fixtures showing
  a NON-privileged manifest still cannot author these keywords/lowerings.
  Model: the S6 std-construct-authorization.maude re-model covers the
  registration re-key AND the extended privilege mechanism (registered S6
  obligations).
- **I3 — admission-core extraction + stdio + file drivers.** Extract the
  shared admission function from the `whip signal` path; `whip ingress serve
  --stdio` admits JSONL; the worker source pass drives file sources (drop the
  non-clock skip, main.rs:20831). Landing site: the generalized
  source-resolving pass and the extracted admission core land KERNEL-side,
  alongside kernel::rule_pass and generic over the store traits, per the
  in-flight instance-scheduler lift (which is relocating exactly this class
  of machinery out of cli/main.rs); the file poller's filesystem I/O (glob
  walk + content hash — the FileStore seam has no directory listing,
  store/files.rs:20-58) sits behind a native driver seam the pass calls, and
  the `whip ingress serve` command shell stays CLI-side. No new worker
  machinery accretes in cli/main.rs; coordinate the pass relocation with the
  DO/instance-scheduler tracker to avoid a mid-flight collision.
  Tests: stdio e2e (valid line fires rule;
  duplicate line absorbed once; malformed rejected before any fact); file e2e
  (dropped file admits once; unchanged file never re-admits; content-hash
  change re-admits). Model: extend models/maude/admission.maude coverage with
  the delivery-id and file-occurrence key forms + a NoSolution bite fixture
  (soup-variable gotcha applies).
- **I4 — IngressDeliveryLifecycle.tla, then the http driver.** Model-first
  (house discipline): Apalache model of deliver → authenticate → validate →
  admit/duplicate/reject with crash-retry reusing the delivery key —
  coverage AND bite — then `whip ingress serve` HTTP listener with
  path routing, hmac/bearer/shared auth fail-closed, `dedup`/`correlate`
  handling. Tests: in-process listener e2e (the std.telemetry live-collector
  test pattern); auth-failure admits nothing; duplicate delivery absorbed with
  an observable duplicate diagnostic; wrong-path 404s.
- **I5 — `--delivery-id` on `whip signal`.** Operator-supplied delivery id
  wins over the derived hash key, completing "CLI Admission". Tests: same id
  twice admits once across process runs.

## Deferred with cause

- **I4 — `whip ingress serve` HTTP listener (+ its `path`/`auth`/`correlate`
  clauses), deferred 2026-07-15.** Cause: (a) reality overtook the design —
  a POLLING http driver (`url`, worker-pass GET with SSRF screens) shipped
  ahead of this design's listener and covers the current pull-shaped demand,
  so the listener no longer gates any live user; (b) the slice is model-first
  by house discipline (IngressDeliveryLifecycle.tla, coverage AND bite,
  before the hand-rolled listener + hmac/bearer/shared auth), a full gate of
  its own that should not ride a multi-slice pass; (c) its clause set
  (`path`-as-endpoint, `auth <mode> secret <ident>`, `correlate`) is dead
  grammar without the listener, so the clauses defer WITH it (built now:
  `watch`/`dedup`, which the shipped pollers consume). Re-entry: the first
  push-delivery (webhook) demand; the slice enters exactly as specified in
  "v1 implementation slices" I4 — TLA+ model first, then the listener —
  plus the deferred clauses; the `dedup` machinery and the shared admission
  core it must route through are already in place. The stdio driver (also
  I3) was NOT deferred: it shipped as the dev/test reference path.
- **`std.ingress.grpc`.** Cause: `event-ingress.md` itself sequences it after
  cli/http harden the admission contract; HTTP/2 + protobuf pulls a heavy
  dependency tree against the threads-plus-ureq house minimalism; zero demand.
  Re-entry: a concrete typed-service demand after I4; it enters as one more
  provider row + clause set — no grammar or admission-core change, which is
  why deferral is safe.
- **Instance-starting deliveries** (openly registered question). `whip signal`
  targets existing instances only (main.rs:29430-29435) and the dedup index
  keys on instance id (migrations/0001:75-76), so a start-new-instance
  delivery has no admission identity, no program-revision selection rule, and
  no correlation grammar today. Cause: undesigned, and v1 http works with
  `correlate` mandatory. Re-entry: a dedicated design note when the first
  webhook-spawns-run demand appears; the `correlate` clause (I2a) and the
  provider `can_start_instances` descriptor field are the prepared hooks.
- **DO-plane source execution.** Clock/timer/source resolution lives in
  cli/main.rs and is already a DO-tracker obligation (chunk-4 tail / P6
  alarms); http-on-DO = DO fetch handler. Cause: M7 — one concern, one
  tracker. Re-entry: the DO tracker's package/compute phases.
- **Broker/topic + product-specific webhook adapters.** Per "Deferred" in
  `event-ingress.md` (delivery/offset/consumer-group semantics out of the
  base design). Re-entry: demand, as provider packages over the same
  admission core.
- **Probed provider capability reports.** v1 ships static descriptor data in
  manifest provider rows only. Cause: constitution E3 defers report machinery
  demand-side. Re-entry: first provider whose capabilities vary by deployment.
- **`source interaction`.** Owned by the std.messaging concrete design
  ("Relationship To `std.messaging`"); std.ingress only guarantees the family
  admits it.

## Spec amendments

1. **`event-ingress.md`, "Static Checks"**: "`source <provider> as <name>`
   requires an imported package that contributes the provider kind" — split
   per the M5 graduated ladder: provider-kind-known is a HARD check (once
   embedded manifests land); the import line itself is an advisory lint in
   v1, with lint→error escalation registered, not built.
2. **`event-ingress.md`, "CLI Admission" and "Modeling Notes"** (and
   `admission-and-idempotency.md`'s matching identity sections): align dedup
   vocabulary with the shipped index — the unique index is
   `events(instance_id, idempotency_key)`, not "(instance,
   fact_identity_key)". Same mechanism; the name in the specs is wrong.
3. **`std-time.md`**: must mirror the "Boundary with `std.time`" section above
   — clock provider + recurrence grammar in std.time, external delivery kinds
   in std.ingress, worker source pass + admission core in core. (Stated here
   per E2; the std.time design owns its own edit.)

No other substrate contradiction found: the target kind `signal.emit` this
design writes throughout matches what `api-reference.md`, `providers.md`, and
`language.md` already document; the code catches up via substrate slice S1.
