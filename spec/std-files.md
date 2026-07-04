# `std.files`: concrete package design (std-package campaign)

Status: concrete design 2026-07-04, under the settled ecosystem shape
(spec/std-package-ecosystem-shape.md; every M1–M8/E1–E7 decision binds this
document). Substrate spec: spec/files.md — this design does NOT restate its
surface; it packages what shipped, closes the catalog fossil (shape note
"Lowering classes"), designs the `file.*` capability layer, fixes the recorded
turn-grant spec violation, settles dynamic-path security posture, and states
the export kernel-lift gap honestly. Where it contradicts the substrate, see
"Spec amendments".

## Design-tracker checklist

- **Core functionality.** Named `file store` resources as a path/policy
  boundary; deterministic read/write of text bodies and import/export of typed
  rows; turn-scoped agent file grants. All four effect verticals plus the
  declaration and turn grants are SHIPPED end-to-end (spec/files.md
  "Implementation Status"; kernel handlers at
  crates/whipplescript-kernel/src/effect_handlers.rs:494, :621, :853; export at
  crates/whipplescript-cli/src/main.rs:23185).
- **Why it belongs as a package.** Files are an external resource boundary
  (spec/files.md "Framing") with its own capability vocabulary, provider seam
  (FileStore), and IFC face — a semantic domain per shape note "Semantic
  domains vs provider catalogs". It is core grammar today with no package
  identity; this design gives it one without moving the grammar (M1:
  authorize-post-parse).
- **What is NOT in the package.** File-arrival observation (`source file`
  watch = std.ingress; boundary stated in spec/files.md "Relationship To Other
  Packages"); model-backed document understanding (coerce); process execution
  (std.script — explicitly not a hidden fallback, spec/files.md "Non-Goals");
  the typed fact-batch admission primitive (core platform,
  `SqliteStore::admit_fact_batch`); the workspace-versioning plane
  (spec/versioned-workspace-research-note.md owns file edges as evidence-grade
  boundaries).
- **Target feature set (v1).** Everything already shipped, plus: live `file.*`
  capability layer; turn-grant ∩ store-policy intersection; dynamic-path
  hardening; export handler relocated into the kernel (already generic; see
  "Export kernel-lift status"); embedded manifest + typed_effect_call
  attribution.
- **Verdict.** KEEP as its own package (per shape note "Merge/split/defer/drop
  verdicts": no merges). Furthest-along authoring-surface package; v1 is
  hardening + packaging plus one small declaration clause (the `provider`
  clause, F5) — no new effect surface.

## Surface

No new effect grammar; the one v1 addition is the `provider` declaration
clause noted below. The shipped surface is normative (spec/files.md "Source
Surface"); this section fixes its package-contract identities.

Declarations:

- `file store <name> { root "…" allow read [globs] allow write [globs] }` —
  family declaration_block, lowering metadata_only, provides
  Resource\<FileStore\> (parser/lib.rs:17984). One clause addition in v1:
  optional `provider <ident>` (default `local`; unknown providers rejected at
  check time), aligning with the channel declaration's block-internal provider
  clause (state-note inconsistency, "Block-internal `provider` clause").
  `formats [...]` stays deferred (no consumer).

Effect operations, with EXACT effect-kind strings (unchanged — see E4 below):

| Operation | Effect kind | Family / lowering class | Capability id (M3: id == kind) |
| --- | --- | --- | --- |
| `read text\|markdown from <store> at <path> as b` | `file.read` | effect_operation / typed_effect_call | `file.read` |
| `write text\|markdown to <store> at <path> {…} as b` | `file.write` | effect_operation / typed_effect_call | `file.write` |
| `import jsonl\|json\|csv <Schema> from <store> at <path> as b` | `file.import` | effect_operation / typed_effect_call | `file.import` |
| `export jsonl\|json\|csv <Schema> to <store> at <path> {…} as b` | `file.export` | effect_operation / typed_effect_call | `file.export` |

Terminal facts stay `file.<op>.completed` / `file.<op>.failed` (DR-0032
EffectError base on failures). Contracts are already attributed to library_id
`std.files` (parser/lib.rs:3551–3581) but carry EMPTY required_capabilities;
v1 fills each with its own kind string, satisfying the store's
default-required-capability rule by construction (store/lib.rs:6873-6877).

Turn grants: `with access to <store> { read [globs] write [globs] }` on
tell/invoke is authority-narrowing metadata on `agent.tell` (spec/files.md
"Turn Access Grant"), NOT an effect operation and NOT a capability id. The
substrate's `files.turn_access` capability is dropped: turn grants are the
third enforcement plane of M3 (harness), governed by the monotone-narrowing
invariant `contract caps ⊆ store-bound caps ⊇ turn grants`, and recorded as
evidence-metadata (per the construct-lowering decisions record). See "Spec
amendments".

## Providers (M2 seam classification)

The `local` provider executes through the **FileStore host-projection trait**
(store/files.rs:20-58) — std.files' counterpart of M2's CapabilityProvider
seam, and it PRECEDES it: NativeFileStore and DoFileStore both implement it,
so read/write/import already run on native and the DO/wasm plane (kernel
handlers cited above; host-do DoFileStore, commit aa2c076). v1 makes the
registry honest about this: the embedded manifest contributes
effect_providers rows naming provider `local` for all four kinds, and the
provider expectation is:

- deterministic codecs only (spec/files.md "Format Scope");
- path authorization repeated at runtime before any disk access — compiled
  source is not authority (spec/files.md "Security And Policy");
- content hashes on reads and writes; explicit write modes, no silent
  overwrite;
- both planes: native and DO (export joins after slice F4 below).

Future non-filesystem providers (S3/GitHub/Drive) classify as **HTTP sans-IO
step machines** (M2 class 1) when un-deferred; their path-namespace contract
is the re-entry design (see "Deferred with cause").

## Manifest (M5 contributions)

The embedded `std.files` manifest contributes:

- **libraries**: `std.files` (standard, catalog-privileged). Today the library
  registers only when a rule uses a file effect (register_effect_contract →
  register_standard_library, parser/lib.rs:3333); a bare `file store`
  declaration registers nothing — the manifest closes that.
- **effect_contracts**: the four `file.*` contracts with
  required_capabilities = [own kind], typed outputs
  FileReadResult/FileWriteResult/FileImportResult/FileExportResult
  (RuntimeBoundary validation, parser/lib.rs:3549-3588).
- **constructs**: `file store` (declaration_block/metadata_only) and
  read/write/import/export (effect_operation/**typed_effect_call**) —
  authorize-post-parse per M1; grammar stays in the core parser.
- **capabilities**: `file.read`, `file.write`, `file.import`, `file.export`.
- **providers**: `local` over the FileStore seam, for all four kinds.
- **profiles**: a `files` profile row allowlisting the four capabilities, the
  operator's coarse on/off per workspace (same enforcement points as `coerce`:
  store/lib.rs:6951, harness_tools.rs:2315-2318).
- **import posture**: advisory missing-import lint for `use std.files` (M5
  graduated ladder; hard-off is std.script's alone).

## Closing the catalog fossil (shape note "Lowering classes")

The catalog claims typed_effect_call was "promoted for std.files
DR-0019/0020" (core/lib.rs:382, target_capability Forbidden), but the code
took Route B: builtin `file.*` kinds resolved directly, no manifest
(spec/files.md "Implementation Status"). Decision E4 closes this by making
the claim TRUE while KEEPING the `file.*` kind strings: the manifest rows
above declare typed_effect_call, lowering keeps emitting the same
IrEffectKind::File* kinds, and — because idempotency keys hash the kind
string (parser/lib.rs:9537-9548) — durable history is untouched. The
"promotion" is attribution + admission, not a rekey: with the manifest rows
registered, the builtin-kind bypass of the admission gate is removed for
`file.*`, so `policy_block_on` (store/lib.rs:6423) governs file effects
exactly as it governs any package kind. Fail-closed check: a workspace whose
store lacks the std.files rows (stale store) gets a loud
blocked_by_capability, not a hang.

## Turn-grant ∩ store-policy intersection (quick win Q3)

**The violation.** spec/files.md "Agent File Access" defines the effective
grant as a NARROWING of the store policy. The shipped harness ignores the
store: turn_tool_access_from_input (harness_tools.rs:2062-2132) turns grant
globs directly into tool policy against the single workspace root — store
root and `allow` globs never consulted — and collapses multi-store grants
into one merged TurnFileAccess whose store_name degrades to `"turn_access"`
(harness_tools.rs:2118-2122).

**The fix.** Lowering already validates the grant against the declared store
(parser/lib.rs:4026); the fix has lowering additionally embed a per-grant
policy snapshot — the store's `root` and `allow read/write` globs — into the
access_grants payload on agent.tell (landing next to the existing
access_grants emission at rule_lowering.rs:3084). The harness computes, per
store: effective read = grant read ∩ store allow-read, effective write =
grant write ∩ store allow-write, all paths resolved against the store root
(not the workspace root), empty intersection = deny. TurnFileAccess becomes
per-store (a Vec, not one merged struct); the `"turn_access"` fallback name
is deleted. Precedent: workflow_invoke start grants already use intersection
semantics (main.rs:19311, :23971) — this brings agent turns to parity. The
governance-envelope check (enforce_turn_access_governance,
harness_tools.rs:2135) is unchanged: it governs resource NAMES; this fix
governs their extent.

## Dynamic `at <Expr>` path security

**Reality.** spec/files.md says v0 paths are literal ("v0 Scope", "Security
And Policy", "Non-Goals") — but the parser accepts value expressions after
`at`, the substrate's own composition example uses `at file.path`
("Relationship To Other Packages"), and the runtime path policy (root
containment, `..`/absolute denial, allow-glob match) already runs per-value
before any disk access (effect_handlers.rs:536; denial fixtures
control_plane.rs:588, :985). Dynamic paths are shipped and runtime-checked;
the literal-only claim is fiction.

**Posture (decided here).** Runtime authorization is the authority; the spec
is amended to say so. v1 hardens the two real holes:

1. **Symlink escape.** Root containment is checked on the requested path;
   a symlink inside the root pointing outside it defeats containment.
   Fix: canonicalize after policy match and re-check containment against the
   canonicalized store root before the operation; failure settles
   `file.<op>.failed` (fail-closed, no disk content touched). FileStore gains
   a `canonicalize` method with native + DO implementations (DO plane:
   DoFileStore's virtual namespace has no symlinks — implement as identity
   and state that).
2. **Literal-path static check.** When the `at` argument IS a literal, `whip
   check` validates it against the store's allow globs at compile time
   (M8 tier-2 hand-coded check) — restoring the static-auditability the spec
   wanted, without banning the dynamic form that ships.

Full canonicalization/TOCTOU/glob-intersection design for hostile providers
stays deferred (see below); the local provider's threat model is closed by
1+2.

## Export kernel-lift status (honest)

`file.read`/`file.write`/`file.import` are host-agnostic kernel handlers over
the FileStore seam and run on the DO/wasm plane. `file.export` is ALREADY
generic over the same seams: `run_file_export_effect_generic<S: RuntimeStore>`
takes a held `RuntimeKernel<S>` plus `&dyn FileStore` (cli/main.rs:23185,
DO-tracker chunk 3c) and the generic instance-step executor already
dispatches it (main.rs:20116). What remains is **crate location, not shape**:
the fn lives in the cli crate, which is not built for wasm32, so exports
still cannot execute on the DO plane. Slice F4 is therefore a relocation —
move the already-generic core into kernel::effect_handlers and re-point the
dispatch arms — not a genericization lift. Until it lands, every std.files
claim of DO-parity must carry the export exception.

## Static checks (M8 tier assignment)

All std.files checks are **tier 2: hand-coded core checks named here** — the
files domain has no triplicate pattern qualifying for the catalog-driven
generic tier. The set: declared-store requirement; format-vs-verb routing
diagnostics (structured formats rejected from read/write); mandatory write
`mode`; grant-op vocabulary on file stores (read/write/import/export only,
empty grants rejected, parser/lib.rs:4026); NEW: literal-path-vs-allow-glob
check; NEW: unknown `provider` clause rejection. Honesty-audit note for
close-out: the catalog's static-guarantee flags on typed_effect_call must map
to these named checks or be removed.

## Information-flow face (DR-0029 posture)

std.files opens no new doors. File stores are governed resources under the
envelope grammar `grant file_store <name> -> file:<address> …`
(ifc.rs:2422); `file.read`/`file.import` classify as read sources and
`file.write`/`file.export` as write sinks in the static checker (ifc.rs:690,
:873, :1476-1480). Per DR-0029's producer/consumer split: the package's
declared surface is exactly the four kinds against Resource\<FileStore\>
handles; a consumer binds stores to governed `file:` addresses and the
surface-refines-envelope check applies unchanged. Import is an admission
boundary: rows enter as typed facts whose labels derive from the store's
grant — no laundering past the envelope. Turn grants are checked against the
verified governance envelope (harness_tools.rs:2135); the Q3 fix narrows
that door's extent but does not change its label semantics. Nothing in this
design touches FlowAwait_* namespaces or adds egress.

## Dependencies

Core: effect lifecycle + admission gate; typed fact-batch admission
(shipped); FileStore seam; IFC engine + envelope; DR-0022 collection
projections (export). Campaign substrate: S6 embedded manifests (slices F2/F5
ride it); S4 capability-namespace invariant checks. Other packages: none at
build time. Boundary-only: std.ingress (`source file` watch), std.script
(escape hatch, never fallback), std.agent/owned harness (turn-grant
enforcement point).

## v1 implementation slices

Each independently gateable under the per-piece review discipline.

- **F1 — Q3 intersection fix** (quick win, no substrate dependency).
  Lowering embeds store-policy snapshots; harness computes per-store
  intersections rooted at store roots; delete the `"turn_access"` collapse.
  Tests: grant-wider-than-store is clipped; path outside store root denied;
  two-store grant exposes two distinct scopes; existing grant e2e
  (main.rs:43825, harness_tools.rs:2581) stay green. Model: none new (no
  lifecycle/protocol change); the fix is glob-set intersection, covered by
  table-driven unit tests.
- **F2 — capability layer live** (after S6). Contracts carry
  required_capabilities = [kind]; embedded manifest registers
  capability/provider/profile rows; remove the builtin-kind admission bypass
  for `file.*`. Tests: unbound `file.read` blocks as blocked_by_capability;
  bound path unchanged byte-identical; stale-store loud-failure fixture.
  Model: extend models/maude/package-contract.maude with a file.* capability
  fixture (coverage + one negative).
- **F3 — dynamic-path hardening.** Symlink canonicalize-and-recheck in the
  FileStore seam; literal-path static check. Tests: symlink-escape negative
  fixture fails closed pre-disk; literal path outside allow-globs is a check
  error; dynamic in-policy path still succeeds. Model: none (single-op path
  authorization; negative fixtures carry the bite).
- **F4 — export handler relocation.** Move the already-generic
  run_file_export_effect_generic (cli/main.rs:23185, chunk 3c) into
  kernel::effect_handlers and re-point the two dispatch arms (main.rs:20116,
  :20542); no signature or behavior change. Tests: existing export e2e
  (control_plane.rs:906) green; wasm32 target builds; native/relocated parity
  on a golden export.
- **F5 — typed_effect_call attribution + provider clause** (with S6).
  Manifest constructs rows declare typed_effect_call; `provider local`
  clause parses (default local, unknown rejected); manifest-vs-compiled
  contract drift test. Tests: catalog honesty assertion; .ir snapshots
  unchanged (zero kind churn is the acceptance criterion).

## Deferred with cause

- **bytes/docx/xlsx/pdf/OCR codecs** — cause: decoder version drift breaks
  replay determinism (spec/files.md "Deferred Scope"); re-entry: dedicated
  codec-pinning design with replay-against-recorded-output semantics.
- **Non-filesystem providers (S3/GitHub/Drive)** — cause: path-namespace
  contract undesigned, zero demand; re-entry: first remote-store demand,
  built on the M2 HTTP sans-IO seam class.
- **`formats [...]` store clause** — cause: no consumer until a second codec
  family exists; re-entry: rides the codec design above.
- **Full dynamic-path security engine** (canonicalization contract for
  hostile providers, per-value glob-intersection formalization) — cause:
  local provider's threat model is closed by F3; re-entry: first non-fs
  provider, whose namespace makes it load-bearing.
- **`rows <named-collection>` general export source** — cause: DR-0022
  exposed collections only in the export clause for v0; re-entry: general
  collection-value design.
- **Hard `use std.files` import requirement** — cause: M5 graduated ladder
  (advisory lint in v1, repo-wide example churn deferred); re-entry: the
  registered lint→error escalation cleanup.
- **DO package bootstrap for these rows** — cause: M7 (one concern, one
  tracker); re-entry: DO tracker Phase 8-adjacent package layer.

## Spec amendments

1. **spec/files.md "Capabilities"** — replace
   `files.read/files.import/files.write/files.export/files.turn_access` with
   capability ids EQUAL to effect kinds:
   `file.read`/`file.write`/`file.import`/`file.export` (M3 id==kind);
   delete `files.turn_access` (turn grants are harness-plane metadata, not a
   capability id).
2. **spec/files.md "v0 Scope", "Security And Policy", "Non-Goals"** — remove
   the literal-paths-only claims; state the decided posture: dynamic `at
   <Expr>` paths are accepted, runtime authorization (containment + globs +
   canonicalized symlink re-check) is authoritative, literal paths
   additionally get the compile-time policy check.
3. **spec/files.md header ("Reserved-class prerequisites") and
   "Implementation Status"** — record that the typed_effect_call promotion
   executes as attribution-keeping-`file.*`-kinds (E4): no rekey, no Route-B
   re-home of the kind strings; the Route B note becomes historical.

## Open naming-boundary questions

- Bare `file` is both a queue verb (`file item into Q`, body.rs:1083) and the
  store declaration keyword; rename slice C (`queue.*` → `tracker.*`) touches
  the former — sequencing note for whoever lands C, not a files decision.
- Whether a future `read bytes` rides `file.read` (a codec) or a new kind (a
  new contract) — decided in the codec design pass, biased to codec-of-
  `file.read` to avoid a rekey.
- Store `provider` clause vocabulary must stay consistent with channel/source
  provider clauses when provider capability reports arrive (state-note
  inconsistency); owned by whichever package first ships a capability-report
  grammar.
