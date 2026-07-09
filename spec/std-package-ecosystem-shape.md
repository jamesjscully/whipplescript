# Standard-Package Ecosystem Shape

Status: feeding-ADR (ecosystem shape settled 2026-07-04; both ⚑ forks RESOLVED
by Jack 2026-07-04 — M5 embedded manifests ratified, M1 build-DR-0011-now
reverses the recommended defer)

This note settles the standard-package design tracker's "overall ecosystem
shape" gate (its "Process", step 5) so the per-package concrete designs
(step 6) can proceed. It answers the tracker's "Meta Questions" plus the
eight cross-cutting mechanism questions those answers depend on. It is
design intent, not reality and not a commitment; the tracker's "Current
Rule" still gates implementation on concrete per-package designs.

## How this was settled

An understand pass (11 agents) produced evidence-cited state documents for
all 14 inventory rows and the package mechanism layer. Three targeted
fact-checks resolved the load-bearing unknowns — is the runtime provider
registry decorative, which catalog lowering classes are exercised, and the
true blast radius of a pre-release effect-kind rename — followed by a
three-proposal judge panel (pragmatic / spec-faithful / mechanism-first
lenses; judges scoring grounding, engineering economics, and owner fit)
whose syntheses converged on most decisions, and a campaign-lead synthesis.
Jack was away; per the goal directive these are decided-on-recommendation,
and the two genuinely contested forks (M1 meta-grammar, M5 std-as-manifest)
took the cheapest-to-reverse branch and carry ⚑ FOR JACK'S REVIEW callouts.

## Ground truth that shaped everything

Five fact-checked findings underlie every decision here.

1. **The runtime provider registry is an admission gate, not a dispatcher —
   and the gate is real.** No handler is ever selected through the seeded
   rows: dispatch is a hardcoded `match effect.kind.as_str()` in both
   executors (cli/main.rs:20515, cli/main.rs:20093), and the
   `provider`/`config_json` columns are never read anywhere. But
   `policy_block_on` (store/lib.rs:6423) enforces row *existence* at two
   choke points (claimable_effects at lib.rs:2508, start_run at
   lib.rs:4243): a non-builtin effect kind only becomes claimable if an
   effect_providers row exists and each required capability — defaulting to
   the kind string itself (store/lib.rs:6873-6877) — has schema + binding
   rows. `profiles` content is genuinely enforced twice: the policy gate
   (store/lib.rs:6951) and the owned-harness tool surface
   (harness_tools.rs:2315-2318). `package_registrations` is pure audit
   record.
2. **The real-provider plug-point is precisely located.** `capability.call`
   settles as a fixture: the output value is fabricated at the else-branch
   in kernel/effect_handlers.rs:1883-1887, then validated against the lock's
   capability contract via the `CapabilityContract` host projection
   (effect_handlers.rs:1797-1801) and completed. A real provider slots in
   exactly there, and registry-honest selection means extending
   `capability_bound` (store/lib.rs:6928) from `SELECT 1` to returning the
   binding's provider + config_json.
3. **Pre-release one-way renames are safe.** There is no store back-compat
   promise anywhere: the migration chain is a single baseline
   (store/lib.rs:914-918), spec/package-management.md "Non-Goals" defers
   package migrations, and no whip.lock is checked in anywhere. A rename's
   entire user-facing blast radius is a developer re-initializing
   `.whipplescript/`. The dangerous mode is *silent*: old-kind effects fall
   through the executor's `_ => Ok(None)` (cli/main.rs:20515-20543) and hang
   forever; idempotency keys hash the kind string
   (parser/lib.rs:9537-9548), so renamed kinds rekey all effect ids; the
   `coerce` seeds live in migration 0001 (0001_runtime_store.sql:377, 401,
   421-423, 429) and never re-seed an existing store.
4. **recall/send are the grammar precedent, and they are hardcoded.** The
   only shipped package-construct parses (`recall`, `send`) are dedicated
   core-parser branches with hardcoded target capabilities
   (parser/body.rs:1853-1985); catalog/lock rows authorize the construct
   *after* parse. No package can register a construct grammar end-to-end
   today.
5. **The catalog's dormant lowering classes are now mapped.**
   `rule_template` is live (every rule/when node carries it,
   cli/main.rs:4038-4066; DR-0023 action templates reach it only by
   parse-time inlining). `schedule_emitter` is live (timer-wait effects,
   cli/main.rs:4414-4439). `projection_view` is registered-but-never-produced
   (nothing emits it; `emit milestone` is a synchronous fact projection in
   rule lowering, kernel/rule_lowering.rs:1074-1112, and correctly bypasses
   it). `resource_effect` has zero producers despite being coord's declared
   target.

## Mechanism decisions

### M1. Construct grammar: BUILD the two-shape DR-0011 meta-grammar now (Jack 2026-07-04) ⚑ RESOLVED

**Decision.** For v1 the core parser owns ALL construct grammar. Catalog and
manifest rows AUTHORIZE constructs post-parse — the shipped recall/send
precedent, generalized: a new std construct is a core parser arm whose
construct-use is validated against the built-in/lock registry, exactly as
`recall` and `send` are today. DR-0011's constrained two-shape meta-grammar
is recorded verbatim as the re-entry design sketch, built when a third-party
construct demand exists: shape 1 = `declaration_block` (keyword + typed
clause-schema: `provider <ident>`, scalar/duration/glob/schema fields);
shape 2 = `effect_operation` (slot-template over a fixed slot vocabulary —
keyword, expr slots with connective words `from`/`for`/`into`/`to`/`via`,
one payload record block, mandatory `as <binding>`, optional `fails as`),
with target_capability, output schema, and outcome vocabulary drawn from the
manifest. The sketch carries its named falsifier (a std construct the two
shapes cannot express forces a shape addition, not a hardcode) and its
validation-by-deletion criterion (re-register `recall` and `send` through it
and delete parse_recall/parse_send).

**Why.** Every package in scope is std and we control the parser. The
authorize-post-parse mechanism exists, is Maude-modeled
(models/maude/std-construct-authorization.maude, package-contract.maude),
and has two shipped exercisers. The meta-grammar is the single largest novel
mechanism on the table with zero external demand — deferral is the
reversible branch, and the catalog + reserved-word privilege tuple grants
(core/lib.rs:554-582) remain the future seam so nothing is foreclosed.

**The road not taken.** Building the meta-grammar now (two of three
proposals) has real force: DR-0011 is an accepted baseline ("core parses via
a fixed extension meta-grammar"); spec/construct-grammar.md's own next steps
name generalizing the recall parser; and hand-coded std grammar keeps std
"semantically magical" against that document's own contract that third-party
packages use the same families — guaranteeing a porting campaign the day the
first external author appears.

**Risks/mitigations.** std packages keep needing core parser changes, so
"package-authorable" stays partially aspirational; the Package Author
Contract's acceptance rule (a package needing a class that does not exist is
not accepted) covers the interim. The judges split on this fork — hence the
flag.

> **⚑ RESOLVED 2026-07-04 (Jack): BUILD DR-0011 now — reverses the
> recommended defer.** Build the constrained two-shape extension meta-grammar
> now (declaration_block + effect_operation shapes), model-first
> (construct-grammar.maude with coverage + bite), with validation-by-deletion
> of parse_recall/parse_send as the greenfield check. The manifest
> `constructs[]` carries machine-readable grammar specs for the two shapes;
> std constructs register their grammar through it rather than through
> hardcoded parser arms. This becomes a load-bearing S6 sub-slice (see build
> order) and reshapes the per-package designs' M1 assumption from
> "core parser owns grammar / authorize-post-parse" to
> "manifest declares grammar in one of the two shapes"; the designs' concrete
> constructs already fit the two shapes, so the surfaces are unchanged — only
> the registration path differs. The authorize-post-parse mechanism remains
> the fallback for a construct the two shapes cannot express (a core-grammar
> exception, not a shape addition).
>
> *Original recommendation was defer* (zero external demand,
> avoid-premature-abstraction); Jack chose build-now because DR-0011 is
> accepted baseline and every std construct added under deferral would be a
> future re-registration.

### M2. Provider execution seam: three designated seams + the CapabilityProvider host projection now

**Decision.** Providers execute through three designated seams selected by
provider class — no unification: (1) **HTTP sans-IO step machine** for
network providers (the CoerceStepMachine/BrokeredTurnMachine precedent;
DO-compatible by construction); (2) **subprocess adapter** for local tooling
(native-only, DO counterpart = DO tracker Phase 8);
(3) **native adapter trait behind a cargo feature** for deep session
protocols (codex/claude/pi). One NEW seam lands now: a `CapabilityProvider`
host-projection trait replacing the fixture else-branch at
kernel/effect_handlers.rs:1883-1887, keeping the existing
validate→complete_run→derive-fact tail unchanged; and `capability_bound`
(store/lib.rs:6928) is extended from `SELECT 1` to returning the binding's
provider + config_json. The registry becomes dispatch-honest; the fixture
becomes a provider NAMED `fixture`. std.memory is the forcing case: fixture
+ local backend is a genuine two-provider demand, dissolving the
wait-for-two-providers objection.

**Why.** All the patterns already exist, are tested, and map onto deployment
reality (store + HTTP run on the DO plane; subprocess does not). Designating
per class rather than unifying honors keep-genuinely-different-mechanisms-
separate, and the fact-check located both the exact plug-point and the exact
one-query promotion that ends the decorative-rows dishonesty.

**The road not taken.** Full deferral of registry dispatch until a second
real provider exists (the pragmatic proposal) is cheaper and honest about
the rows staying admission-only — but it leaves the state brief's flagged
"dead rows" hole governing all 14 package designs, and
fixture-as-named-provider removes its premise.

**Risks/mitigations.** One indexed SELECT on dispatch is trivial. Env
overrides (`WHIPPLESCRIPT_COERCE_PROVIDER`, coerce_runtime.rs:71-80) must be
defined as operator-override-over-registry-default in the coercion concrete
design or config drift returns.

### M3. Capability planes: one namespace, three enforcement points, monotone narrowing

**Decision.** ONE dotted capability namespace across all three planes, with
the mechanisms kept separate and a monotone-narrowing invariant:
compile-time contract caps ⊆ store-bound caps ⊇ turn-grants. Manifest
`capabilities[]` feeds the runtime plane (capability_schemas/bindings — the
already-enforced admission gate) as it does today via
register_locked_packages (cli/main.rs:18953 → store/lib.rs:2552). For 1:1
effect operations the capability id EQUALS the effect kind string, matching
the store's default-required-capability rule by construction
(store/lib.rs:6873-6877): `tracker.claim`, `lease.acquire`, `file.read`,
`schema.coerce`, `signal.emit`, `messaging.send`. Parameterized caps use
pattern FAMILIES (`script.<name>`). New lock-time check: contract
required_capabilities ⊆ the owning manifest's capabilities[].

**S4 status (verified 2026-07-04): the lock-time subset check is ALREADY
SHIPPED — do not rebuild it.** `validate_declared_capability`, driven by
`validate_package_manifest_consistency` (invoked on every manifest load,
cli/main.rs:16897), rejects any effect contract whose `required_capabilities`
(or provider capability, or contract id) is not in the manifest's declared
`capabilities[]` — exactly this invariant. Tested: a manifest whose contract
requires `memory.missing` is rejected (cli/main.rs:36942). The id==kind leg is
true by construction (the store defaults an effect's required capability to its
kind string, store/lib.rs:6873-6877), so no check enforces it — it is
structural. The remaining leg, **`bound ⊇ turn-grants`** (a turn grant must be
within the bound capabilities), is harness-side and belongs to the std.files /
std.script package tails that own file/script turn grants — tracked there, not
as standalone S4 work. S4 therefore adds no new substrate code; it is verified
already-satisfied for its enforceable content.

**Why.** The runtime plane is the only one with teeth today; feeding it is
zero new mechanism. id==kind makes the manifest and the store agree by
construction. Unifying the three enforcement engines (admission gate,
contract metadata, harness tool surface) would be premature abstraction
across genuinely different checkpoints. The pattern-family treatment closes
the one hole the simpler versions glossed: without it, `script.<name>`
either over-blocks or goes decorative.

**The road not taken.** A plane-mapping layer between distinct namespaces —
strictly more mechanism for the same convergence.

**Risks/mitigations.** `model.invoke` (coercion contract) and `coerce`
(runtime rows) both die in the schema.coerce rename; pre-release,
acceptable. The subset check may flush latent manifest drift; fix as found.

### M4. Renames: one-way, pre-release, no migration mechanism — three gated slices behind a map dedup

**Decision.** All three decided renames execute as one-way pre-release
breaks with NO migration mechanism (fact-checked: no back-compat promise
exists; blast radius = local store re-init). PRE-SLICE first: dedup the
three duplicated IrEffectKind↔string maps (parser/lib.rs:3613,
kernel/lib.rs:3010, cli/main.rs:28201) into the kernel map — a half-day that
cuts each rename's regeneration surface by roughly two-thirds. Then three
gated slices:

- **(A)** `event.notify` → `signal.emit`. **LANDED 2026-07-04** (S1a
  commit; `IrEffectKind::EventNotify` → `SignalEmit`, the kind string,
  derived facts `signal.emit.completed/.failed`, the builtin capability-gate
  exemption in store + DO mirror, dispatch arms, and the regenerated
  `examples/event-bridge.ir` — 1115 tests green). Proved the regeneration
  pipeline. **The `event.emit` purge originally bundled here is DEFERRED with
  cause:** an investigation before touching it found `event.emit` is *not*
  dead legacy — `emit milestone` currently produces a phantom `event.emit`
  effect through the bare-`emit` branch of the text-based rule-body lowering
  (kernel/rule_lowering.rs:2965), and `event.emit` still carries full
  capability/provider/binding/profile seed rows in migration 0001. Purging it
  is therefore a change to milestone/projection lowering (Family C; the
  package-projection-noun-vocabulary decision record's territory), not a
  clean legacy removal. Re-entry: a projection-owned slice that removes the
  phantom `event.emit` from milestone lowering (the real `workflow.milestone:*`
  fact is derived independently at rule_lowering.rs:1078 and is unaffected),
  then retires the `event.emit` kind/handler/seeds and reconciles the
  `parse_effect_line` diagnostic classifier. Sequence it with the projection
  work, not the rename substrate.
- **(B)** `coerce` → `schema.coerce` and `std.coerce` → `std.coercion` —
  AND, in the same rekey, the coerce idempotency key gains
  model/prompt/schema-hash commitments (the recorded spec/coerce.md
  violation rides the one free rekey; a small key-commitment design note is
  authored BEFORE the slice, not bundled half-designed into it). This slice
  also AMENDS the schema-coercion package decision record's
  migration-tooling clause (pre-release posture supersedes it) — all three
  judges flagged the amendment as mandatory.
- **(C)** `queue.*` → `tracker.*` nouns: queue/item → tracker/issue;
  statuses done/cancelled → closed/canceled/archived — the FULL decided set
  including `archived`.

EVERY slice adds a store-open guard that errors loudly on pending
retired-kind effects, converting the fact-checked silent-hang mode
(cli/main.rs:20515-20543 unknown-kind fallthrough) into a visible failure.
**LANDED 2026-07-04** (S1b commit) as a shared, once-built mechanism:
`run_worker_once` → `guard_no_stale_effect_kinds`, driven by a
`RETIRED_EFFECT_KINDS` **denylist** (each rename slice appends its old kind).
A denylist, not an allowlist — because not every live runtime kind has an
`IrEffectKind` variant (e.g. `lease.release` is dispatched but unmodeled in
the enum), so an allowlist false-positives on legitimate effects. Later
rename slices reuse this guard by adding one string, not rebuilding it.

**Why.** Fact-check 3 is definitive: single-baseline migration
(store/lib.rs:914-918), no store-stability promise in any spec, and
`rm -rf .whipplescript/` fully cures old stores. The BAML purge and notify
removal set the house precedent; spec/execution-contract.md already labels
`coerce.*` facts "current implementation compatibility names" with
`schema.coerce.*` as target. Doing renames first prevents fourteen packages
of fresh references to dead names.

**The road not taken.** Building a kind-versioning/migration mechanism first
(the schema-coercion DR's original clause) buys safety for stores that do
not exist — a mechanism with zero users, delaying every downstream slice.

**Risks/mitigations.** In-flight dev instances die (accepted; one announced
re-init). Users running compiled `.ir` directly must recompile. The
regeneration surface — 6 checked-in .ir snapshots, the accept fixture, Maude
models, ~10 spec files — is our own release work, bounded per slice by the
map dedup and the per-piece gate.

### M5. std packages become embedded manifests, with a graduated import ladder ⚑

**Decision.** std packages become EMBEDDED manifests — real manifest files
shipped in the binary, validated by the same pipeline as third-party
manifests, catalog-privileged — replacing usage-driven auto-registration.
Import bite is GRADUATED: `use std.script` is HARD-OFF now (not imported ⇒
any exec form is a check error — the tracker's prompt-injection-resistance
demand for std.script); every other std package gets an advisory
missing-import lint in v1; escalation of lint→hard requirement is registered
as a later one-way cleanup, avoiding an all-at-once `use`-line churn of
every example. The std lock-exemption re-key (from "compiled-in standard
registration" to "manifest embedded in this binary", same no-squatting
property) REQUIRES re-modeling models/maude/std-construct-authorization.maude
in the same slice — model-first; all three judges flagged this.

**Authorability door (added 2026-07-04, concrete-design coherence pass).** The
concrete designs for std.coord, std.ingress, std.messaging, and std.time each
independently hit a wall this decision did not originally name: their embedded
manifests must author constructs whose lowering class is
`package_authorable: false` (`resource_effect` for coord; `signal_source` /
`signal_emit` / `metadata` for the source family; `clock_source` for time),
and the shipped validator rejects any non-authorable construct row
(cli/main.rs:13200-13204) *before and independently of* the reserved-keyword
privilege check (cli/main.rs:13217-13228). Reserved-keyword privilege
authorizes a bare *word*; it does not authorize a *class*. So S6 gains a
second, adjacent mechanism: a platform-catalog privilege tuple whose
`lowering_target` is a non-authorable class also authorizes *that class* for
exactly that `(library, keyword, family, scope, lowering)` tuple — the classes
stay `package_authorable: false` for unprivileged third parties. This is one
door built once in S6, re-modeled once in std-construct-authorization.maude
(coverage: privileged std manifest authors the class; bite: an unprivileged
manifest with the same construct row is still rejected). std.ingress (owner of
the shared source-declaration family) registers the source-family obligations;
std.coord owns the `resource_effect` obligation. std.script deliberately avoids
the door by choosing the already-authorable `typed_effect_call` class for
`exec` (the std.files DR-0020 precedent). Rationale: convergent across four
independent designs, mechanically identical to the keyword-privilege door
already in S6, and cheaper than promoting four classes to universally
authorable (which would let any third party emit them). Cheapest-to-reverse:
if the tuple mechanism proves wrong, the fallback is per-class authorability
promotion, a strictly larger grant — so starting narrow forecloses nothing.

**Why.** Embedded manifests exercise the exact validation machinery third
parties will use, give `use` semantic bite (today inert except
workflow_tools), and give the lock exemption a principled shape. The
std-tracker.json privilege-acceptance test (cli/main.rs:37468) anticipated
exactly this. The graduated ladder takes the cheap part now (script's
security-relevant gate) and defers the expensive part (example churn).

**The road not taken.** Keeping std compiled-in permanently (the pragmatic
proposal) is honest and smaller: with the meta-grammar deferred per M1, the
embedded manifests cannot carry grammar, so they partially mirror
compiled-in truth — a two-sources-of-truth liability held together by drift
tests. That is the strongest argument against this decision and the reason
it carries the flag.

**Risks/mitigations.** Manifest JSON must stay in lockstep with the core
code owning the actual lowering; guarded by schema-vs-catalog drift tests.
The std.script hard-off's enforcement depth beyond the check error (a
runtime backstop via per-program seeding, or an IR-uses gate at exec
dispatch) is specified in the std.script concrete design — the panel was
unanimous that a check-time gate alone does not resist forged-IR paths.

> **⚑ RESOLVED 2026-07-04 (Jack): EMBEDDED MANIFESTS** — ratifies the
> recommendation. std packages ship as embedded manifests; with M1 now
> build-now (above), the manifests carry real grammar specs, so the
> "two sources of truth" objection to compiled-in is moot. Below is the
> decision record kept for provenance.
>
> *Embedded manifests (chosen):* std becomes the first customer of the
> third-party path, so "a package costs a manifest" is verified by
> construction on every std slice; `use` stops being a lie; the lock
> exemption gets a principled re-key. *Compiled-in:* with M1 deferred, the
> manifests mirror compiled truth — two sources of truth is pure liability,
> and a lint-only `use` achieves the same v1 discipline with zero new
> representation. *Why the chosen branch is cheapest to reverse:* the
> hard-to-reverse element (hard import requirement everywhere, with its
> repo-wide example churn) is NOT taken — only std.script goes hard-off;
> the manifest representation itself is a data move over the existing
> validator and folds back into Rust constants mechanically if reversed.
> *What re-opens it:* manifest/code drift biting in practice, or Jack
> preferring compiled-in in review.

### M6. Package versions: deferred

**Decision.** Defer entirely, per spec/package-management.md "Non-Goals":
`version: "unlocked"` plus the sha256 manifest pin stays the whole story. No
comparison semantics, no semver, no resolution.

**Why/risks.** Nothing consumes version fields today; sha256 pins fail
closed on any change, which is correct v0 behavior. Contract-evolution
questions resurface with the first second version of anything — registered
deferral, not an oversight.

### M7. DO plane: bounded kernel lift now; full DO package layer stays in the DO tracker

**Decision.** This campaign performs a bounded kernel lift of the package
registry/validation core (manifest parse/validate, ContractRegistry
derivation, catalog) so the DO plane CAN host it — the same lift pattern as
rule_lowering/rule_pass. The full DO package layer (register_locked_packages
at DO bootstrap, manifest delivery, package rows over DoSql — the gate
mirror already exists at host-do/do_store.rs:3094-3134) stays registered in
spec/durable-object-runtime-tracker.md (Phase 8-adjacent), not this
campaign. Hard rule: NO new package machinery lands in cli/main.rs — the
instance-scheduler lift campaign is literally paying down that debt.

**The road not taken.** Doing DO package bootstrap here — rejected: one
concern, one tracker; the DO tracker owns live-Cloudflare sequencing.

**Risks/mitigations.** Until that phase lands, a locked package with runtime
providers cannot function on the DO — acceptable, none exist. The lift is
bounded to the registry/validation core; CLI-only surfaces (lock discovery,
`whip package` commands) stay in main.rs.

### M8. Static checks: two-tier, with an honesty audit at close-out

**Decision.** Two tiers, packages bring data never code: (1)
catalog-flag-driven GENERIC checks are built ONLY where the rule of three is
already satisfied — where triplicate hand-coded implementations exist today
(binding-required, exhaustive outcome handling across coord held/contended,
tracker claim outcomes, coerce branches); (2) everything else stays
hand-coded core checks named in the owning package's spec — precisely why
std.coord is "privileged". Close-out HONESTY AUDIT slice: every catalog
static-guarantee flag either maps to a real checker or is removed — a
decorative flag is the same dishonesty as a decorative registry row. Per the
lowering-class fact-check, `rule_template` and `schedule_emitter` are
actively exercised core-internal classes and the audit documents them as
such (no removal).

**Why.** The Package Author Contract already forbids parser callbacks and
arbitrary lowering code; a declarative check engine beyond the rule-of-three
set is the second-largest premature abstraction available.

**Risks/mitigations.** The hosted-exec line-scan lint stays fragile until
rewritten inside the script slice (onto the AST), not via a framework.

## Ecosystem-shape decisions

### E1. Names

std.coord, std.tracker, std.agent (with std.agent.codex and std.agent.claude
as thin provider packages; std.agent.pi DEFERRED name-reserved — pi_variant
is pure spec), std.messaging, std.memory (manifest id `memory` →
`std.memory`; KEEP capability ids `memory.query`/`memory.write` — the
panel's `memory.recall`/`memory.learn` was an undecided invention,
rejected), std.time, std.ingress, std.files, std.script, std.telemetry,
std.coercion. std.test stays dropped. CLI collision: the `whip agent` IFC
REPL is renamed → `whip infoflow` (one-line decision, recorded here;
whichever name, it is picked ONCE).

*Road not taken:* none of substance — every rename executes an
already-settled tracker note or decision record; this row closes the memory
naming drift, which predates the convention.

### E2. Semantic domains vs provider catalogs

Semantic domains (own nouns/verbs/typed surfaces): coord, tracker,
messaging, memory, time, ingress, files, script — plus coercion-the-concept
(the backend is a provider inside it). Provider catalogs: std.agent.codex /
.claude (/.pi when un-deferred), plus provider sections WITHIN each domain
package. std.agent itself is a boundary/taxonomy package — the portable
feature-report contract its provider sub-packages fill in — NOT a harness
bundle. *Risk:* std.time's boundary with std.ingress's source family must be
stated in both package specs.

### E3. Authoring surfaces vs operator-config packages

Authoring surfaces: coord, tracker, messaging, memory, time, ingress, files,
script. Operator-config packages: telemetry and coercion. "Package" for the
operator-config pair means, machine-checkably: an embedded manifest
contributing capabilities/providers/profiles rows + a CLI surface + the
import advisory — no source constructs — and that is a legitimate package
shape, defined by the runtime's actually-enforced primitives rather than a
second package concept. *Road not taken:* thickening the definition with
probed capability reports and config schemas now — deferred demand-side; the
thin definition is the honest current truth.

### E4. Lowering classes

std.files → typed_effect_call promotion, KEEPING the `file.*` effect kinds —
this closes the catalog fossil exactly as the state brief's hardest files
question asks (the catalog claims a promotion the code never did) with zero
durable-history churn. resource_effect gets its first producer via coord's
registration. signal_source and clock_source are exercised by the ingress
and time registrations. projection_view stays DEFERRED — the
package-projection-noun-vocabulary decision record itself recommends
deferral — and is documented core-internal/unproduced: the honesty rule is
satisfied by documentation, not a manufactured producer (milestones are
Family C synchronous fact projections, kernel/rule_lowering.rs:1074-1112,
and correctly bypass the class).

### E5. Bundled but imported

All std packages ship in the binary and are imported via `use` — bite per
the M5 ladder: std.script hard-off now; advisory lint for the rest in v1;
lint→error escalation registered, not built. *Risk:* two-tier import
semantics must be documented crisply or authors will assume other `use`
lines gate authority.

### E6. Merge/split/defer/drop verdicts

Keep all rows except: std.agent.pi deferred (name reserved; pi_variant has
no lock/provenance/discovery design); std.test already dropped. No new
merges or splits — time stays separate from ingress, messaging separate from
ingress, and coord's four lease mechanisms stay separate per Jack's standing
decision. The ADR-0002 event-sourced tracker rebuild is split IN TIME, not
in package: identity/rename now on the shipped row-store, rebuild deferred
(see build order). *Road not taken:* the spec-faithful proposal's
rebuild-now — the ADR is status:proposed, no conflict/relations demand
exists, and it was the campaign's largest unforced slice.

### E7. Package feature vs core lifecycle invariant

Core keeps the lifecycle invariants no package can opt out of: coord release
obligations + terminal auto-release of ALL held state
(release_holder_resources_on_terminal spans queue claims and coordination
leases from one code path — the cancel-leak lesson says this stays one core
hook), one-shot timers/timeouts + no-clock-in-guards, the IFC engine and
membrane, and the effect lifecycle (claim/run/settle, admission,
idempotency). Packages own vocabulary, providers, CLI projections, and
check DEMANDS — which core implements and enforces (per M8). *Risk:* the
coord discipline checks sit right on the line; their spec must name core as
implementor to prevent a future "package brings a check" misreading.

## Cross-cutting registered items

Five findings where the judges held all three proposals wrong; each becomes
a tracked item rather than prose:

1. **DR-0014 amendment rides rename slice B** — else the spec corpus
   contradicts reality the day the rename lands.
2. **std-construct-authorization.maude re-model rides the M5 slice** — the
   lock-exemption re-key changes a Maude-modeled property; model-first.
3. **spec/coordination.md one-kernel-lease-primitive language vs the decided
   4-mechanisms-separate** — explicit resolution item in the coord concrete
   design (recommend amending the spec to match the decided separation,
   citing Jack's lease-mechanisms decision).
4. **human-review (askHuman/inbox) migration onto messaging** — registered
   as an OWNED design question inside the messaging concrete design, not
   silently deferred; the live surface stays untouched meanwhile.
5. **Build order below = CANDIDATE slices only** — per the design tracker's
   "Current Rule", implementation commitments wait for the per-package
   concrete designs (Process step 6). This note honors that gate.

## Candidate build order

Candidates pending per-package concrete designs (see item 5 above); the
substrate is ordered, the quick wins are not.

Substrate: **S0** kind-map dedup → **S1** rename A (signal.emit) → **S2**
rename B (schema.coerce/std.coercion + key commitments + DR-0014 amendment)
→ **S3** rename C (tracker.* nouns) → **S4** capability-namespace invariants
(contract ⊆ bound ⊇ grant checks) → **S5** CapabilityProvider seam +
capability_bound promotion (fixture-as-named-provider) → **S6** embedded std
manifests + import lint + script hard-off + the lock-exemption re-key AND the
authorability door (privilege tuple authorizes a non-authorable lowering class;
see M5 "Authorability door") + Maude re-model of both doors → **S7** bounded
kernel lift of the registry core.

Parallel quick wins (any time): **Q1** ingress `emit`-names-a-declared-signal
static check (verified missing today); **Q2** telemetry auth headers +
cursor scoping; **Q3** files turn-grant ∩ store-policy intersection fix (the
recorded spec violation at harness_tools.rs:2062-2132).

Per-package tails from the phase-3 concrete designs: coord; tracker (nouns +
contracts' tracker.* caps + `renew` parses + claim/renew/release privilege
rows exercised + minimal `whip issue`); messaging (3 local providers +
capability reports + MessageSendReceipt + interaction sources); ingress
(source-provider execution for cli/http/stdio/file + provider config
grammar); time (package identity; DO alarms stay in the DO tracker); files
(typed_effect_call + files.* caps); memory (pool decls +
learn/curate/keep/forget + real provider over the new seam + harness grant
wiring, replacing the inert grant arm at harness_tools.rs:2093-2112); script
(hard-off + identity + model-first Maude; DO-plane exec explicitly out of
scope = DO tracker Phase 8); telemetry (identity + allowlist); coercion
(rides S2 + schema_coercer provider kind + a config-plane reconciliation
note including the homeless shared codex credential layer); agent (open
provider registry from manifests + minimal DR-0015 capability-report surface
+ centralized 7-preset profile table as package data + codex/claude thin
packages).

ADR-0002's event-sourced tracker rebuild: DEFERRED to its own later
model-first campaign behind the surviving WorkItems seam + the package
contract — no forcing demand; the ADR is status:proposed.

## What "finished" means for this campaign

- [ ] All 14 inventory rows have concrete per-package designs (design
      tracker Process step 6), each recording its deferred set.
- [ ] Substrate slices S0-S7 landed, each gates-green under the per-piece
      review discipline, plus quick wins Q1-Q3.
- [ ] Per-package v1 slices from those concrete designs landed gates-green.
- [ ] Every deferred item carries a cause and a re-entry path (the M1
      meta-grammar sketch, the capability_bound second-provider extension,
      the ADR-0002 rebuild, the DO package bootstrap row, lint→error import
      escalation, version semantics).
- [ ] The standard-package design tracker updated: inventory rows moved to
      designed/built status, "Current Rule" satisfied in order, and the two
      ⚑ forks either ratified or reversed by Jack.
