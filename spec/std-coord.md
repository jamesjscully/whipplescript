# `std.coord` package design: lease, ledger, counter

Status: concrete package design 2026-07-04 (std-package campaign, Process step 6).
Constitution: [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md) — decisions M1–M8/E1–E7 bind this design.
Substrate spec: [`coordination.md`](coordination.md) (semantics, safety model, modeling notes) — this document does not restate it; it packages it.

## Package contract

- **Core functionality.** Three closed shared-coordination resources — `lease`
  (reservable, TTL-bounded, 1..N slots), `ledger` (append-only, partitioned,
  retention-pruned), `counter` (consumable, cap+reset) — with atomic
  attempt-and-branch verbs and workspace-scoped durable state. See
  coordination.md "The three architectural principles" and "The reversible /
  irreversible split".
- **Why it belongs.** ~70% shipped and load-bearing today: declarations, verbs,
  typed effect kinds, the host-agnostic handler
  (kernel/effect_handlers.rs:1066-1271), the `Coordination` trait seam
  (store/coordination.rs:431-660, implemented native + DO), the three
  discipline checks (parser/lib.rs:12887-12962), terminal auto-release
  (main.rs:20635), CLI projections, and IFC carriage
  (models/maude/infoflow-coord-carriage.maude). What is missing is exactly
  package identity: lease/ledger/counter are core reserved keywords with
  builtin lowering, no manifest, no capability rows, and the catalog's
  `resource_effect` class has zero producers (constitution "Ground truth",
  item 5). std.coord is the designated first producer (E4).
- **NOT in this package.** Work-item claims (`queue`/std.tracker — a separate
  lease mechanism by standing decision, see "The four-mechanisms resolution");
  agent `capacity` (core scheduler); effect leases (core effect lifecycle);
  mailboxes/channels (std.messaging, per coordination.md "Coordination vs
  messaging"); instance-identity values (core, per the same section); release
  obligations and terminal auto-release enforcement (core lifecycle invariant
  per E7 — this package may not opt out and does not implement it).
- **Target feature set (v1).** Shipped surface + package identity (embedded
  manifest, capabilities, `resource_effect` attribution) + the counter
  timezone anchor + replay-determinism fix + formal models for the three
  shipped protocols + honest handler input validation. Bounded wait, lease
  order, renew, the full event vocabulary, and in-workflow ledger reads are
  deferred with cause below.
- **Dependencies.** Core: sum-type exhaustiveness, effect lifecycle +
  idempotency, terminal auto-release hook, the `Coordination` store trait,
  IFC engine. Packages: `std.time` only for the counter reset anchor
  vocabulary (shared with admission-and-idempotency.md "periodic reset
  anchor"); composes with std.ingress (signal injection) and std.tracker
  (sibling claim surface) without depending on them.
- **Provider expectations.** None external in v1 (see "Providers").
- **Open naming-boundary questions.** Listed at the end.
- **Verdict.** KEEP as its own semantic-domain package (E2/E6); v1 =
  packaging/namespacing + honesty slices over the shipped core, per
  TRACKERS.md ("shipped in core; package tracker is now a
  packaging/namespacing question").

## The four-mechanisms resolution (registered item)

spec/coordination.md says in three places ("Framing", the "lease" subsection,
"Dependencies") that `std.coord.lease` is a surface over a **single kernel
lease primitive** shared with `tracker.claim` and std.tracker claims. Reality is
the opposite, deliberately: the CoordinationStore keeps its own leases table
with its own TTL/expiry (store/coordination.rs:95-410), separate from
store.sqlite effect leases and items.sqlite claims (which carry no TTL at
all), per Jack's standing decision of 2026-06-17 — **four lease mechanisms
across three stores stay separate; merge queue+tracker first if ever
revisited** (memory: lease-mechanisms-kept-separate).

**Resolution: amend coordination.md to match the decision.** The unification
language is the fossil; the code is the intent. What the mechanisms genuinely
share is the *contract shape* (atomic attempt, branchable outcome,
holder-lifetime bound, terminal auto-release through the one core hook,
release_holder_resources_on_terminal — the cancel-leak lesson) — not an
implementation. The re-entry path for unification is exactly the one the
decision names: a queue+tracker merge (ADR-0002 rebuild campaign) would create
the second real customer; only then does a shared primitive have two users.
See "Spec amendments".

## Surface

All grammar is core-parsed per M1 (authorize-post-parse); the manifest
authorizes, it does not parse. The shipped surface is the v1 surface:

**Declarations** (family `declaration_block`, already shipped
parser/lib.rs:17657-17873):

```whip
lease   <name> { key <Type> slots N ttl <dur> [shared] }
ledger  <name> { entry <Schema> partition by <field> retain <dur> [shared] }
counter <name> { key <Type> cap N reset hourly|daily|weekly|monthly [timezone <tz>] [shared] }
```

(`timezone` on counter is NEW in v1 — slice 3; everything else shipped.)

**Effect operations** (family `effect_operation`, lowering class
`resource_effect` — see below) with EXACT effect-kind strings, all shipped
(parser/lib.rs:3490-3533):

| operation | effect kind | outcomes |
|---|---|---|
| `acquire <lease> for <key> [until ttl] as x` | `lease.acquire` | `held` / `contended{holders}` |
| `release <binding>` | `lease.release` | released |
| `append <Schema> {..} to <ledger> [as x]` | `ledger.append` | `appended{seq}` |
| `consume <counter> for <key> amount <n> as x` | `counter.consume` | `ok` / `over{remaining}` |

**Capability ids** per M3 (id == effect kind, matching the store's
default-required-capability rule store/lib.rs:6873-6877 by construction):
`lease.acquire`, `lease.release`, `ledger.append`, `counter.consume`.

**Lowering class: `resource_effect` — first producer (E4).** Today these
operations lower as builtin kinds with no class attribution; the catalog's
`resource_effect` class (core/lib.rs, `package_authorable: false`) has zero
producers. v1 attributes the four operations' construct registrations to
`resource_effect`, exactly as coordination.md "Surface" demands ("the
long-term package lowering should be a modeled `resource_effect` class, not
plain `capability_call`"). The class stays `package_authorable: false` for
the open ecosystem: the platform catalog owns the lowering semantics and the
reserved bare verbs; std.coord is privileged via reserved-keyword privilege
tuple rows (core/lib.rs:554-582 mechanism):
`lease`/`ledger`/`counter` → (std.coord, declaration_block,
top_level, metadata_only) and `acquire`/`append`/`consume`/`release` →
(std.coord, effect_operation, rule_body, resource_effect). The `release` row
coexists with std.tracker's — privilege rows are exact
(keyword, library_id, family, scope, lowering) tuples, so the shared bare verb
is two rows, not a collision.

Two mechanism gaps make this MORE than a rows-only change, and slice 4 owns
both explicitly:

- **Privilege must pierce authorability.** As shipped, manifest validation
  rejects ANY construct row whose lowering class is non-package-authorable
  (cli/main.rs:13200-13204), before and independently of the reserved-keyword
  privilege check (main.rs:13217-13228) — privilege tuples clear reserved-
  KEYWORD usage only; they do not override authorability. The std.tracker
  claim/renew/release precedent does not carry here: those rows lower as
  `capability_call`, which IS `package_authorable: true`. Without a mechanism
  change, "stays `package_authorable: false`" + "validated by the third-party
  pipeline" + four `resource_effect` construct rows are jointly unsatisfiable.
  **Chosen mechanism: extend the validator so a catalog privilege tuple whose
  `lowering_target` is a non-authorable class also authorizes that class for
  that exact (library, keyword, family, scope, lowering) tuple.** Privilege
  rows become the single, platform-catalog-owned door through the
  authorability wall; third parties remain fully locked out of
  `resource_effect` (no catalog tuple, no row). This is a validator change
  that rides/extends substrate slice S6, and it changes a Maude-modeled
  property: models/maude/std-construct-authorization.maude (and
  package-contract.maude as affected) must be re-modeled in the same slice,
  model-first, per the constitution's cross-cutting item 2.
- **Three of the seven verbs are not reserved yet.** Only
  `lease`/`ledger`/`counter`/`release` appear in
  PLATFORM_CONSTRUCT_CATALOG.reserved_keywords today (core/lib.rs:554-558);
  `acquire`/`append`/`consume` do NOT, and
  `reserved_keyword_privilege_error` returns None immediately for
  non-reserved keywords (main.rs:13278-13280) — privilege rows for unreserved
  words would be dead data, and the bare verbs would stay squattable by any
  future package construct path. Slice 4 therefore adds `acquire`, `append`,
  `consume` to `reserved_keywords` in the SAME change that adds their
  privilege rows — reserving the bare words is exactly the no-squatting
  property the privilege mechanism exists for — and reflects the new keywords
  in the re-modeled std-construct-authorization.maude.

**Operator surface** (shipped): `whip [--json] leases | ledger [--partition] |
counters` (main.rs:29668-29717); `WHIPPLESCRIPT_COORDINATION_STORE`;
`lint.unused_lease`.

## Providers

Per M2's three seams (HTTP sans-IO / subprocess / native adapter):
**std.coord uses none of them.** Its execution plane is the store plane: the
generic handler runs over the object-safe `Coordination` trait
(store/coordination.rs:431-660), implemented by the native workspace SQLite
store and by the DO host (host-do/do_instance.rs) — DO parity is already ahead
of the package design and MUST be preserved; any v1 change keeps the trait
seam. This is the same posture as std.tracker's `WorkItems` seam, and it is a
legitimate fourth position: coordination is state, not I/O, so "provider"
means "which store implements the trait", selected by deployment plane, not by
registry dispatch.

For admission honesty the manifest still names the provider: one
`effect_providers` row per kind, provider id `builtin-coordination` —
registry rows as admission gate (the enforced role, constitution "Ground
truth" item 1), not dispatch. External coordination backends (shared
cross-host lease service, etc.) are deferred; if one ever exists it enters as
a second `Coordination` implementation selected via the M2
`CapabilityProvider`-style promotion of these rows, and the trait is already
shaped for it.

## Manifest

Per M5, std.coord ships as an EMBEDDED manifest (`std-coord.json` compiled
into the binary), validated by the third-party pipeline, catalog-privileged.
It contributes:

- `libraries[]`: `std.coord` v0 (standard), owning four `effect_contracts[]` —
  kinds `lease.acquire` / `lease.release` / `ledger.append` /
  `counter.consume` with the existing typed input schemas
  (parser/lib.rs:3490-3533) and outcome vocabularies (`held|contended`,
  `released`, `appended`, `ok|over`), each with
  `required_capabilities = [<kind>]`.
- `constructs[]`: seven rows — `lease`/`ledger`/`counter` declaration blocks
  (metadata_only) and `acquire`/`release`/`append`/`consume` effect operations
  (resource_effect) — authorizing the core-parsed grammar post-parse (M1).
- `capabilities[]`: the four ids above (M3 id==kind).
- `providers[]`: `builtin-coordination` covering all four kinds.
- `profiles[]`: the four kinds added to the default runtime profile allowlist
  (the coerce-row precedent, migration 0001), so existing programs keep
  running with zero operator action — the admission gate becomes REAL for
  coordination kinds (today they are builtin-exempt) and passes by
  construction because the embedded manifest registers at store init.
  **Becoming real means deleting the builtin coordination exemption from the
  native policy gate** (store/lib.rs:6480-6489, the
  `lease.`/`ledger.`/`counter.` prefix lines) — that removal is explicit
  slice-4 work, not implied. The exemption is mirrored verbatim on the DO
  plane (host-do/do_store.rs:3293-3296), where the bootstrap seeds ONLY
  coerce rows (do_instance.rs:541-552, do_worker.rs:309-313) and M7 defers
  DO manifest registration to the DO tracker. Slice 4 therefore scopes the
  exemption removal **native-only**: the DO mirror keeps its exemption so
  shipped in-DO coordination parity is preserved (per "Providers"), and the
  DO-side removal + coordination row seeding is registered as a deferred
  item riding the DO tracker's package-registration row (see "Deferred with
  cause"). Until that lands the two policy-gate mirrors intentionally
  diverge on coordination kinds; the divergence is documented in a comment
  at both sites.
- Import posture: advisory missing-import lint for `use std.coord` (M5
  graduated ladder; hard requirement is the registered later escalation, not
  this design).

Lock-time invariant (constitution M3): contract required_capabilities ⊆ this
manifest's capabilities[] — true by construction here; the check still runs.

## Static checks

Per M8's two tiers, with **core as implementor of every check below** (E7's
explicit risk note: the package brings check DEMANDS as data; it never brings
check code):

- **Tier 1 (catalog-flag-driven generic, rule-of-three met):** exhaustive
  outcome handling. `held`/`contended` and `ok`/`over` join tracker claim
  outcomes and coerce branches as the third+ customer of the generic
  exhaustive-outcome checker; std.coord's contract rows (slice 4) carry the
  outcome vocabulary the generic checker will read. The generic checker
  itself and the migration of the shipped hand-coded check
  (parser/lib.rs:12887-12962, exhaustiveness half) onto it are NOT owned by
  any v1 slice here — they are campaign-level M8 Tier-1 work, deferred with
  cause below. Until then the hand-coded check stays authoritative.
- **Tier 2 (hand-coded core checks, named here as owner-spec):
  at-most-one-held-lease per progression** (multi-acquire flatly rejected —
  the `lease order` escape is deferred), **linear must-release** (held branch
  releases or reaches terminal, `until ttl` exempt), **typed-key/entry-schema
  declaration check** (parser/lib.rs:3880-3894), **mandatory bounds** (ttl /
  retain / cap+reset), and NEW in v1: **counter timezone default-UTC warning**
  (coordination.md "Static checks" already specifies it; slice 3).

## Information-flow face

DR-0029 posture (Cross-package information flow): std.coord exports no
`@tool`, so it carries no `information_flow` contract surface and no
producer-side `ifc_attested` obligation. Its IFC face is entirely the shipped
core treatment, which this package inherits unchanged:

- Coordination effects are **bidirectional membrane operations**
  (ifc.rs:683-838): payloads written into shared state are egress; outcomes
  read back (`contended{holders}`, `over{remaining}`, projected facts) are
  ingress.
- **`shared` resources are the only cross-workflow channel** the package
  creates, tracked as declared cross-workflow carriage (E-COORD), modeled in
  models/maude/infoflow-coord-carriage.maude with a fixture at main.rs:45258.
  Default owner partitioning (`local/<program>` / workflow principal,
  effect_handlers.rs:251-281) keeps non-`shared` flows intra-workflow by
  construction — no label crossing without the explicit `shared` keyword.
- No new authority crosses the boundary: capabilities are workflow-plane
  admission rows (M3), not ambient grants; importing std.coord grants nothing.

## v1 implementation slices

Each independently gateable under the per-piece review discipline; order is
the recommendation.

1. **Models first for the shipped protocols** (model-first discipline).
   **BUILT 2026-07-14**: `models/tla/CoordLease.tla` (`MutualExclusion`,
   ≤ N holders per key, attempt-and-branch deny + release + TTL expiry),
   `CoordCounter.tla` (`CapInvariant` + `NoLostConsume` via a granted-sum
   history variable + epoch-advancing reset), `CoordLedger.tla`
   (`NoLostEntry` — the checkable residue of `AppendLinearizable` over an
   inherently ordered sequence — + `PartitionIsolation` over projection
   snapshots; retention pruning out: prefix-removal never reorders). All
   three in the Apalache gate loop, each with a mutation bite in
   `scripts/check-tla-models.sh` (slot guard / cap guard / idempotency
   guard stripped ⇒ invariant violation). `NoDeadlock`/`BoundedWait`
   explicitly out (their objects — lease order, wait queue — are deferred).
2. **Handler honesty. BUILT 2026-07-14.** The smuggled defaults for
   malformed input (slots=1/ttl=600s/retain=86400s/cap=0) are gone —
   `run_coordination_effect_generic` fails the effect with the DR-0032
   typed base (`fail_coordination_effect` derives the `{kind}.failed` fact)
   when a numeric field well-formed lowering always emits is missing or
   mistyped; the pre-partitioning `lease.release` shared-owner fallback is
   dropped, and `lease.renew`'s mirror DEFAULT-owner retry with it (a
   renew/release only ever touches its own acquire's owner-scoped lease).
   Renew's fall-back to the ACQUIRE's declared TTL stays: that is the renew
   contract, not a default. Pre-release one-way break per M4 posture. Gate:
   `e2e_malformed_coordination_input_fails_typed_instead_of_defaulting`
   (per-field negatives over forged inputs); kernel/store/DO suites green.
3. **Counter timezone anchor + replay determinism. BUILT 2026-07-14.**
   `timezone "<IANA zone>"` clause on counter (grammar + CounterDecl/IrCounter
   + default-UTC warning when omitted); the period is computed KERNEL-side
   (`effect_handlers::counter_period`) from the pass's INJECTED instant in
   the declared timezone via the clock-source chrono-tz machinery — the
   store's wall-clock `current_period` is deleted from the Coordination
   trait and all three impls (it read `strftime('now')`, the recorded
   replay-determinism violation) — and the consume outcome RECORDS the
   period it resolved against (`"period"` on Ok/Over) so replay re-reads
   rather than re-derives. Unknown zone / unparseable instant fails typed
   (slice 2's path). Spec amendment 5 applied: coordination.md's
   `counter.period_reset` sentence now describes this mechanism, the
   distinct fact joining the deferred event vocabulary. Gate: DST
   spring-forward boundary test + west-of-UTC date test + injected-now
   determinism + runtime `ledger.append` e2e
   (`e2e_counter_period_is_timezone_anchored_and_replay_deterministic`);
   `counter_timezone_clause_parses_and_default_utc_warns`.
4. **Package identity** (rides/follows substrate slice S6): the embedded
   `std-coord.json` manifest of "Manifest" above; the
   privilege-authorizes-non-authorable-class validator extension plus the
   std-construct-authorization.maude (and package-contract.maude) re-model it
   requires, model-first (see "Surface"); `acquire`/`append`/`consume` added
   to PLATFORM_CONSTRUCT_CATALOG.reserved_keywords in the same change as the
   seven privilege tuple rows; `resource_effect` attribution on the four
   operation registrations (E4 first producer); capability + provider +
   profile rows seeded from the manifest; removal of the native builtin
   coordination exemption (store/lib.rs:6480-6489) — native-only, DO mirror
   deferred per "Manifest"; advisory import lint. Gate: Maude re-model
   coverage AND bite; manifest passes the (extended) third-party validation
   pipeline; a negative fixture proving a non-privileged manifest still
   cannot author a `resource_effect` construct; drift test
   manifest-vs-compiled registrations; privilege acceptance test
   (std-tracker.json precedent, main.rs:37468); e2e proof that the NATIVE
   admission gate now holds for a coordination kind (unbound kind blocks,
   seeded kind claimable); DO coordination suites stay green with the DO
   exemption intact.
5. **Release disambiguation off string-matching.** Replace the textual
   body-scan rewrite of `release <binding>` (tracker.release → lease.release
   when the binding matches an `acquire ... as <binding>` line,
   kernel/rule_lowering.rs:2141-2163) with binding-typed resolution over the
   lowered binding table — the platform-catalog reserved-word design this
   scan was a placeholder for. Gate: multi-line/aliased-binding regression
   fixtures that defeat the old scan; both queue and lease release suites
   green.
6. **Spec amendments + docs** (below), plus the docs/manual.md coordination
   section (currently near-absent; api-reference is the only good surface).
   Gate: mkdocs --strict; check-trackers.

## Deferred with cause

| item | cause | re-entry path |
|---|---|---|
| `acquire ... wait timeout <dur>` (FIFO wait queue, `times out` outcome, `lease.wait_enqueued/wait_timed_out`) | zero shipped demand; immediate attempt-and-branch covers the corpus (examples/gastown-lite.whip); it is the one slice that changes protocol semantics | model-first: extend the lease TLA+ with the wait queue and prove `BoundedWait`, then store + surface; spec text already final (coordination.md "lease") |
| `lease order` multi-lease | multi-acquire is flatly rejected today — safe over-restriction with no escape-hatch demand | first real multi-lease program; check design already written (coordination.md "Safety model" item 2) |
| `lease.renew` verb — **row STALE: the verb SHIPPED** (2026-07-04 tail work: `renew <acquire-binding> [until <dur>] as b`, IrEffectKind::LeaseRenew, Coordination::renew_lease_for_owner across all 3 store impls, examples/coord-lease-renew.whip). What remains deferred is only its CONTRACT surface | the effect contract / feature-report shape should not be invented separately from std.tracker's renewal vocabulary; the embedded manifest seeds the capability/provider/binding trio the live verb needs, without a contract row | the std.tracker renew design (ADR-0002 noun work) supplies the shared renewal contract; add the manifest contract row then |
| Full event vocabulary + rebuildable projections (coordination.md "Event And Projection Vocabulary") | runtime records `{kind}.completed` + live tables; no consumer needs the event stream; building it now duplicates the ADR-0002 event-sourcing question | the deferred ADR-0002 event-sourced rebuild campaign — coord joins tracker in one event-sourcing design, not two |
| In-workflow ledger read (`when <ledger> has entry ...`) / aggregation (`recent N`) | `projection_view` is deferred campaign-wide (E4); ledger is CLI-readable meanwhile | un-deferral of the package-projection-noun-vocabulary decision record |
| `consume amount` as full expression | number-literal/dotted-path covers shipped uses | demand-gated ergonomics; parser slot exists |
| Coordination snapshot/checkpoint | recorded in code (store/coordination.rs:427-430): waits for the experimentation-subsystem checkpoint consumer | that consumer materializing |
| External coordination backend | no second `Coordination` implementor exists or is demanded | M2 registry promotion + a second trait impl; rows already shaped for it |
| DO-plane admission gate for coordination kinds (remove the host-do/do_store.rs:3293-3296 exemption mirror + seed the four provider/capability/profile rows in the DO bootstrap, do_instance.rs:541-552 coerce-row pattern) | M7 defers DO manifest registration to the DO tracker; the DO bootstrap seeds only coerce rows today, so removing the mirror without seeded rows would turn every in-DO coordination effect into blocked_by_capability and break shipped DO parity | the DO tracker's package-registration row (Phase-8-adjacent); until then the native/DO policy-gate mirrors intentionally diverge on coordination kinds (documented at both sites) |
| Generic Tier-1 exhaustive-outcome checker + migration of the hand-coded exhaustiveness half (parser/lib.rs:12887-12962) onto it | the checker is campaign-level M8 Tier-1 work; no coord slice owns it, and building it here would silently expand v1 scope — slice 4 lands only the contract outcome-vocabulary rows it will read | the M8 close-out honesty audit / campaign-level Tier-1 build, with coord + tracker + coerce as the rule-of-three customers; gate there = generic checker driven by contract rows, all three green, hand-coded exhaustiveness half deleted |

## Spec amendments

1. **spec/coordination.md, "Framing"** — the parenthetical citing "the kernel
   lease primitive" as superseding items.sqlite: reword to cite the
   four-mechanisms-separate decision; the architectural template claim stands.
2. **spec/coordination.md, "lease"** — the paragraph "`std.coord.lease` does
   not implement leasing itself... surface over the single kernel lease
   primitive": replace with the decided posture — std.coord.lease owns its
   lease state behind the `Coordination` trait; queue claims, effect leases,
   and agent capacity are sibling *disciplines* sharing the contract shape and
   the single core terminal-auto-release hook (E7), not an implementation;
   unification is re-openable only after a queue+tracker merge.
3. **spec/coordination.md, "Dependencies"** — same replacement for the
   "single kernel lease primitive (`acquire`/`renew`/`expire`/`recover`)"
   sentence.
4. **spec/coordination.md, "Event And Projection Vocabulary"** — mark the
   event list as the deferred event-sourcing target (re-entry: ADR-0002
   campaign); current recorded truth is `{kind}.completed` facts + run
   metadata + live-table CLI projections.
5. **spec/coordination.md, counter section** (the replay-determinism
   sentence, ~:222-226) — the spec specifies replay determinism via a
   recorded `counter.period_reset` fact that replay re-reads; slice 3
   delivers the same property by a different mechanism: the `consume`
   OUTCOME records the period it resolved against, and replay re-reads the
   outcome. Reword the sentence to the outcome-recorded mechanism;
   `counter.period_reset` as a distinct fact moves to the deferred event
   vocabulary (amendment 4 / the ADR-0002 bucket), so the substrate spec no
   longer contradicts the built mechanism.

## Open naming-boundary questions

- `release` is a bare verb shared across std.coord and std.tracker privilege
  rows; slice 5 removes the string-scan hazard, but source-facing docs must
  teach the one-verb-two-resources story deliberately.
- CLI nouns `whip leases|ledger|counters` are three top-level commands; a
  future `whip coord <noun>` grouping is open (decide once, with the
  `whip issue` naming in the tracker design — not here).
- Counter reset-anchor vocabulary must stay word-for-word shared with
  admission-and-idempotency.md "periodic reset anchor" and std.time's calendar
  forms; slice 3 cites both rather than inventing fields.
