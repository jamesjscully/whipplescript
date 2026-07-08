# Decl-family migration — design note (D2–D5)

Status: pre-build, design ratified in outline (Jack 2026-07-08 "full migration");
this note is the hardened implementation design produced by the
`decl-migration-design` workflow (3 competing designs → synthesis → 3 adversarial
verifiers, 2026-07-08). It supersedes the outline where they conflict.

Goal: migrate declaration-family std packages so their **grammar** lives in
manifests and top-level `declaration_block` parsing is data-driven, mirroring the
shipped `effect_operation` (Shape 2) pipeline. DR-0011 Shape 1 was amended
2026-07-08 (flag kind, multi-word clause names, `list: true`; commit 6e17135).

## Winning spine

Faithful mirror of the Shape-2 pipeline, with per-decl **typed-node builders**
localizing the one irreducible asymmetry (effect_operation lowers to a uniform
node; the decls lower to 7 distinct typed AST nodes). Code geography correction:
all new decl spec infra lands in **`parser/src/lib.rs`** (the top-level Item
parser), NOT `body.rs` (which owns the rule-body effect_operation pipeline).

- `ClauseKind { Identifier, Expression, Duration, Glob, Schema, Scalar, Flag }`;
  `ClauseSpec { name, words, kind, required, list, unknown_hint, missing_summary }`
  (order-free analog of `EffectSlotSpec`; `words` = build-time-split multi-word
  clause-name tokens); `DeclarationBlockSpec { keyword, keyword_words, ast_kind,
  clauses }`.
- `declaration_block_spec_at()` matches on the **head word only**
  (`keyword_words[0]`) + `include!(OUT_DIR/declaration_block_grammar.rs)`.
  `build.rs` relaxes BOTH its family reject (build.rs:111) and shape reject
  (build.rs:132) and emits the decl table from the grammar objects.
- Dispatch hook is the FIRST check in `parse_declaration_item` (lib.rs:18216),
  exact analog of body.rs:1148; the 5 exceptions (harness/agent/signal/source/
  coerce) are simply absent from the table, so table membership is the partition.
- Multi-word KEYWORD dispatch is head-word + tail-validation-in-parser (NOT a
  2-token peek — that mis-routes malformed `file <x>` to the wrong diagnostic).

## Decisions (from synthesis)

1. **Hold version `v0`** through D2–D5a. `.ir` is version-invisible, but
   `contract_registry_to_json` + `package_contract_digest` serialize/hash
   `libraries[].version`, so `v0→0.1.0` churns those goldens. Flip is a separate
   deferrable **D5b**, sourced from a generated identity table, bundled with the
   stale-id fixes.
2. **coord stays builtin-no-manifest** (also telemetry). Identity manifests for
   agent/ingress/coercion/workflow/human/files are DEFERRED — they're never
   `use`d, so `merge_embedded_std_manifests` never merges them; an embedded copy
   is inert. Only **source** buys something (the door).
3. **Registration stays hand-written usage-driven code at v0** for the migration
   (keys on AST vectors that persist); generated `DECLARATION_LIBRARY_TRIGGERS`
   is an optional D5b end-state.
4. **Head-word dispatch** (not 2-token peek).
5. **Diagnostics split**: `.ir` (success) is the hard byte-identical gate via
   typed-node builders; carry the two high-frequency per-decl strings
   (unknown-field hint, missing-required summary) in the spec; accept a small
   explicitly-gated regen for genuinely-bespoke control-flow diagnostics
   (confined to parser negative fixtures, never the `.ir` example corpus).
6. **Manifest placement**: grammar-only manifests are read by `build.rs` only,
   NOT embedded (a declaration_block construct in embedded memory/messaging.json
   would panic `embedded_std_manifests()` until the CLI arm lands, and churn
   those digests). See blocker B1 for the directory decision.
7. **Code geography**: decl infra in lib.rs, not body.rs.

## Blockers found by verification (both CONFIRMED) + fixes

- **B1 — Python door mirror fails open.** `scripts/artifact_admission.py`
  `embedded_std_construct_identities()` globs `std/manifests/*.json`
  indiscriminately; the Rust authority `registry_construct_is_embedded_std_copy`
  reads only hardcoded `EMBEDDED_STD_MANIFESTS`. Grammar-only manifests carrying
  declaration_block constructs would be treated as embedded-std by Python
  (fail-open) but not Rust — defeating the authorability door for the migrated
  families. **Fix (part of D2.0):** put grammar-only manifests in a SEPARATE
  directory `std/grammars/` that `build.rs` reads and the door glob does not; add
  a conformance test asserting Python's embedded set == Rust `EMBEDDED_STD_MANIFESTS`
  construct identities.
- **B2 — `.ir` gate blind to lease/ledger/counter.** `to_snapshot` emits no
  coord-decl sections, so "byte-identical .ir" cannot witness 3 of 7 decls; a
  builder that drops a decl or corrupts fields passes every `.ir` checkpoint
  while silently dropping `std.coord` from the registry and changing the digest.
  **Fix:** D2.1–D2.3 gate on an explicit contract-registry + `package_contract_digest`
  diff AND direct assertions on `ir.leases/ledgers/counters` fields (slots
  default 1, ttl seconds, cap i64, reset enum, partition_field, retain). `.ir`
  stays the witness only for tracker/channel/file-store/memory-pool.

## Structural exceptions — RESOLVED (Jack 2026-07-08: M2, all 7 migrate)

Byte-identical preservation is OFF (Jack: "I don't mind changing behavior/ADR to
make the language consistent"). That dissolved the fork — the two decls were
"exceptions" only against preserving current behavior.

- **ledger `partition by`** — `by` is a *connective*, the same concept Shape 2
  slots already have (`recall <pool> for <query>`). **M2 CHOSEN**: add an
  optional clause connective to Shape 1, sharing Shape 2's vocabulary + `by`;
  model `partition by` as clause `partition` connective `by`, **mandatory**
  (drops the old soft-`by` leniency — behavior change accepted). Unifies the
  connective concept across both shapes; falsifier intact (bounded, shares an
  existing idea). Modeled: `byConn` on `clauseVal` + missing-connective bite.
- **file-store `allow read`/`allow write`** — compound-name clauses sharing the
  `allow` head word; the generic clause matcher reads clause-name words greedily
  against the known set. Unknown direction (`allow sideways`) is now a clean
  "unknown clause" (drops the consume-then-complain quirk — behavior change
  accepted). No new primitive: multi-word names + `list` already cover it.

Result: **all 7 decls migrate data-driven; no declaration-family construct is a
hand-parsed exception.** The 5 real exceptions (harness/agent/signal/source/
coerce) stay. **source door = OPTION (b) DEFER** (D5a dropped — door already
test-covered; a real consumer is un-tie substrate work).

Behavior regularizations accepted (negative-fixture churn only; `.ir` corpus
untouched): mandatory `by`; clean `allow` unknown-direction; memory-pool
`provider` → generic "unknown field"; counter `reset` missing-value recovers.

## Other confirmed findings folded into slices

- Clause span-capture invariant: `root_span`/`read_span`/`write_span`/
  `context_limit_span` are the FIRST-word token span, not the value or joined
  span — assert against file-store-demo before flipping.
- `parse_string_list` hardcodes the "skill string" diagnostic label; route
  glob-lists through it unchanged to avoid negative-fixture churn.
- memory-pool `provider` is a named-but-rejected clause (bespoke-3 regen, or a
  `rejected_clauses` escape hatch — deferred).
- counter `reset` uses a `?` whole-parser bail (minor diagnostic delta on the
  missing-value path; bespoke-3 regen or a per-clause fatal flag).
- **D5a "source door customer" is a demonstration, not a customer**: `parse_source`
  stays native and source.json is never `use`d, so the merged registry construct
  is never consumed on real programs. Rescope D5a's gate to "manifest-validation
  admits signal_source via byte-identity privilege"; do not claim a live consumer.
  Also pin source.json's library version to the usage-driven `v0` or defer to D5b,
  else the door identity 8-tuple can never match.

## Slices (revised)

- **D2.0** — parser scaffolding (structs + include! + head-word `declaration_block_spec_at`)
  + `build.rs` decl-table codegen + grammar-only manifests in `std/grammars/` +
  **B1 door conformance test**. Nothing dispatches yet → trivially byte-identical.
- **D2.1–D2.5** — migrate the 5 clean decls one per subslice (lease, counter,
  tracker, channel, memory-pool), each with the B2 registry+digest gate for coord
  decls and `.ir` gate for the rendered ones.
- **ledger + file-store** — per the fork: hand-parsed exceptions (recommended) or
  a further DR-0011 amendment.
- **D3** — core `ConstructGrammar` declaration_block validation types.
- **D4** — CLI `package_construct_grammar` declaration_block arm (must land AFTER
  the concurrent main.rs work; digests unchanged).
- **D5a** — source authorability-door demonstration (rescoped per finding).
- **D5b** — version/id alignment end-state (v0→0.1.0 + std.exec→std.script +
  std.schedule→std.time), ONE reviewed digest regen across ALL digest-bearing
  artifacts (contract_registry, package_contract, construct_graph,
  model_search_obligations); `.ir` still byte-identical.
