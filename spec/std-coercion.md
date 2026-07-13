# `std.coercion`: schema-coercion backend and operator config

Status: concrete package design 2026-07-04, under
[`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md) (all
mechanism/ecosystem decisions binding). Designed POST-rename: every name
assumes substrate slice S2 (`coerce` → `schema.coerce`, `std.coerce` →
`std.coercion`) has landed; this document carries the S2 key-commitment and
DR-0014 amendment content that ride that slice ("Renames" decision, slice B).
Substrate: [`coerce.md`](coerce.md), decision records 0014, 0029, 0032.

## Framing

**`std.coercion` is the operator-config package for the core `schema.coerce`
effect** (ecosystem shape "Authoring surfaces vs operator-config packages":
embedded manifest contributing capability/provider/profile rows + CLI surface
+ import advisory — no source constructs). Core owns the entire authoring
surface — `coerce`, `decide`, `prompt`, typed terminals,
case/exhaustiveness, typed-output admission, replay — exactly as decision
record 0014's conceptual stack draws it. The package owns what sits behind
the effect: which backend runs, under what credentials and config, with what
idempotency commitments, producing what evidence.

### Core functionality

- The `schema_coercer` provider kind, its two shipped providers (fixture,
  native structured-output), and registry-honest default selection.
- One canonical resolved-config record reconciling the two config planes
  (CLI env + `whip auth` vs DO secrets JSON), and a home for the shared
  model-credential layer ("Credential layer").
- The `schema.coerce` idempotency-key commitments (model/prompt/schema
  hashes) — the recorded [`coerce.md`](coerce.md) "Idempotency And Replay"
  violation, specified below, executed inside S2's one free rekey — and the
  evidence shape (`schema.coerce.provider`: schema/source hashes, redacted
  transcript; shipped at kernel/lib.rs:2007-2033, renamed).

### Why it belongs as a package

The backend is genuinely swappable (fixture / OpenAI Responses / Anthropic
Messages / Codex SSE all ship, kernel/coerce_native.rs), the authority is
operator not author authority, and the config surface is exactly what the
runtime registry exists to hold. Today that surface is CLI env plumbing
BYPASSING the seeded registry rows (coerce_runtime.rs:75-83; seeded
`effect_providers`/binding rows decorative). The package ends that dishonesty
for its own kind — the same move the "Provider execution seam" decision
makes for `capability.call`.

### What is NOT in the package

- `coerce`/`decide`/`prompt` grammar, lowering, typed terminals, `case` —
  core (decision record 0014 "should not own" list stands).
- Generic model-provider abstraction as a WORKFLOW-VISIBLE concept: no
  provider declarations in `.whip` source, no author-facing model routing
  (the operator credential plane is different — see "Credential layer").
- Agent-turn session harnesses (std.agent and its provider sub-packages);
  coerce-artifact generation / `.coerce` interop (deferred / obsolete);
  telemetry mapping of coercion runs (gen_ai attributes — std.telemetry).

## Surface

No `.whip` source surface is contributed. The names the package owns:

```text
effect kind:        schema.coerce  (sole kind)
capability id:      schema.coerce  (id == kind per capability-planes rule)
provider kind:      schema_coercer         provider ids: fixture | native
completion facts:   schema.coerce.succeeded | .failed | .timed_out
evidence kind:      schema.coerce.provider
library id:         std.coercion  (source_forms: coerce, decide, prompt)
```

`prompt` is confirmed as a third core source form lowering to the same
effect (shipped, parser/lib.rs:8603; body.rs:3586); its required capability
changes from never-enforced `model.invoke` to `schema.coerce`, dissolving
the recorded three-way capability drift (`model.invoke`/`coerce`/target).

Operator surface:

```text
whip auth set|status <openai|anthropic>   (shipped, main.rs:22553-22574)
whip coercion status [--json]             (new; see slice 4)
WHIPPLESCRIPT_COERCE_PROVIDER|_MODEL|_BASE_URL|_MAX_TOKENS|_TIMEOUT_SECS
per-effect `provider` override in source  (shipped)
DO create(coerce_config_json)             (shipped, do_wasm.rs:224-237)
```

## Providers

Provider-class seam (per "Provider execution seam"): **HTTP sans-IO step
machine** — that class's precedent IS the coercion code (CoerceStepMachine,
coerce_native.rs:663-700; the DO drives the same pure build/parse fns through
its own NeedsHttp suspend, do_instance.rs:253-292). No subprocess, no
native-adapter trait.

- `fixture` — the deterministic provider, NAMED per the
  fixture-as-named-provider rule: FakeCoerceClient + `stub coerce` +
  schema-based synthesis (kernel/coerce.rs:44-82). Default binding.
- `native` — provider-native structured outputs; config selects the backend
  (`openai` | `anthropic` | codex-OAuth routing) per [`coerce.md`](coerce.md)
  "Execution Modes". One provider id: the backends share
  request/parse/evidence shape, so they are config, not separate providers.

Provider expectations (the `schema_coercer` contract):

```text
input:   rendered prompt, output JSON Schema, resolved config record
output:  JSON conforming to the schema, or typed failure (DR-0032 base)
must:    be drivable sans-IO (pure build/parse; host supplies HTTP); honor
         the timeout; NO hidden retries; emit schema.coerce.provider evidence
         (schema/source hashes + REDACTED transcript; raw provider error out
         of fact values, kernel/lib.rs:2050-2075); fail loudly on misconfig
```

### Backend selection and config precedence

Selection becomes registry-honest with an explicit override ladder (the
"Provider execution seam" risk item, resolved here):

```text
1. per-effect `provider` in source   (author; must name a registered schema_coercer)
2. operator override                 (native: WHIPPLESCRIPT_COERCE_* env + whip-auth;
                                      DO: coerce_config_json secrets)
3. registry default                  (the schema.coerce binding row's provider + config_json)
4. fixture                           (when nothing selects native)
```

Env variables are thereby DEFINED as operator-override-over-registry-default,
not the selection mechanism; the registry consultation is one indexed SELECT
at claim time, mirroring the `capability_bound` promotion.

### Config-plane reconciliation

One canonical resolved-config record, two operator doors:

```text
ResolvedCoercionConfig { provider_id, backend, base_url, api_key,
                         model, max_tokens, timeout_secs, codex_account_id? }
```

Native door: env + `whip auth` (coerce_runtime.rs:71-135; codex path reads
model from `~/.codex/config.toml` when unset). DO door: `coerce_config_json`
on `create` (do_wasm.rs:158-180), same fields, fully explicit (no env on the
DO plane); its divergent `max_tokens` default 1024 is REMOVED — defaults
owned once by this package (`max_tokens 4096`, `timeout_secs 120`, matching
[`coerce.md`](coerce.md)'s table). The kernel sees only the record, never a
plane.

### Credential layer (the homeless shared layer gets its home here)

The model-credential resolution layer — `resolve_credential_with_source`
precedence (env → `whip auth` stored → codex OAuth `~/.codex/auth.json`),
`codex_account_id`, `codex_config_model`, the Anthropic OAuth-rejection
rule, `KNOWN_PROVIDERS` (auth.rs:18) — lives in coerce_runtime.rs and is
CONSUMED by both coercion and the owned agent harness's model backends
(harness_tools.rs:32, :2227). No DR assigns it an owner.

**Recommendation: std.coercion owns it.** (a) It is operator credential
config, and std.coercion is the operator-config package for model-backed
effects — its boundary IS config + CLI; (b) every consumer already reaches it
through the coercion module — this ratifies reality; (c) the owned harness's
model backends are model *clients* like coercion; the std.agent provider
packages (codex/claude session adapters) keep their own auth, NOT rehomed;
(d) the alternatives are worse: core would swallow provider-specific policy
(OAuth terms rulings), std.agent would couple every coercion to the agent
package. DR-0014's "provider configuration as library concepts" exclusion is
amended to "workflow-visible library concepts" (see "Spec amendments"). Seam
discipline: a named `model_auth` module in the CLI crate; no new crate until
a second binary needs it.

## Manifest

Embedded manifest `std.coercion` (per "std packages become embedded
manifests"), contributing:

```text
libraries[]:        std.coercion, standard:true
  effect_contracts: schema.coerce — source_forms [coerce, decide, prompt],
                    required_capabilities ["schema.coerce"],
                    provider_kinds ["schema_coercer"],
                    output "typed-provider-output" (metadata label, as today)
  constructs:       NONE (operator-config package; core owns the grammar)
capabilities[]:     schema.coerce
providers[]:        {id fixture, kind schema_coercer} + {id native, ...}
profiles[]:         default allowlist row containing schema.coerce
bindings[]:         schema.coerce -> fixture (default)
```

The lock-time subset check (capability-planes rule) holds by construction
under id==kind. Migration-0001 seed rows (`coerce`, `builtin-coerce`,
0001_runtime_store.sql:377-429) and DO bootstrap seeds (do_instance.rs:541-552)
are renamed in S2; the manifest supersedes them once S6 lands. Import bite:
advisory missing-import lint only (graduated ladder; not std.script).

## Idempotency key commitments (S2 design-note content)

The shipped key hashes rule/effect-id/kind/binding plus
program_version/revision_epoch (parser/lib.rs:9537-9548); a changed model,
prompt, or schema currently REUSES a recorded outcome, violating
[`coerce.md`](coerce.md) "Idempotency And Replay". S2's rekey (the kind is
in the hash, so the rename rekeys once) carries the fix. The commitments:

```text
key (admission-time) gains:
  coercion_name
  prompt_template_hash        H(declared prompt template source)
  output_schema_hash          H(synthesized output JSON Schema)
  coercion_config_fingerprint H(provider_kind, provider_id, backend, model)
execution fingerprint (run-time, recorded on the run) keeps:
  normalized named-args hash + upstream ids (shipped mechanism)
```

The first three are compile-time facts of the program.
`coercion_config_fingerprint` is runtime config: the HOST supplies it at
kernel construction (native: resolved-config record; DO: `coerce_config_json`;
fixture: literal `"fixture"`) and the kernel folds it into `schema.coerce`
keys only. Deliberate consequences: switching model/provider re-runs future
coercions (correct per spec); the fixture fingerprint is constant, so tests
stay deterministic; replay still re-reads recorded terminals without
re-invoking. The placeholder hash fields (`generated_coerce_source_hash=
'fixture'|'do'|'coerce'`, main.rs:22523) are replaced by the real hashes.
Model artifact: extend models/maude/effect-key.maude with a negative fixture
proving a changed fingerprint yields a different key (bite).

## Static checks

Per the two-tier rule ("Static checks: two-tier"):

- **Tier 1 (generic, catalog-flag-driven):** exhaustive coerce-branch
  handling — in the settled rule-of-three set. std.coercion DEMANDS the
  check; core implements it; the shipped hand-coded check becomes the
  generic engine's third exerciser.
- **Tier 2 (hand-coded core, named here as demands):** `decide` hygienic
  anonymous-class synthesis (shipped); prompt-template placeholder validation
  (`{{ arg }}` names must match declared params — NEW demand, unchecked
  today); config validation surfaced through `whip coercion status` /
  `whip doctor` (unknown provider id, provider-set-but-no-credential,
  OAuth-token-for-anthropic — shipped runtime errors, promoted to
  operator-visible posture).

## Information-flow face

Per decision record 0029's posture the package adds NO new membrane door:
`schema.coerce` is an ordinary effect-membrane egress/ingress pair, not an
`invoke:<package>/<tool>` door.

- **Egress:** the rendered prompt (workflow data) leaves to a model provider,
  governed by the `schema.coerce` capability + profile allowlist (the
  enforced runtime plane) and [`coerce.md`](coerce.md) "Policy".
- **Ingress:** the coerced value is model-derived, low-integrity content in a
  high-validity TYPE — typed admission is a shape guarantee, never a trust
  upgrade; downstream labels treat it as provider-sourced.
- **Evidence:** `schema.coerce.provider` records hashes + redacted
  transcript/summary; the DR-0032 failure payload carries a redacted reason,
  raw provider error out of fact values (kernel/lib.rs:2050-2075).
- **Credentials:** operator-plane secrets; never enter facts, evidence, or
  labels. `whip auth status` reports the SOURCE label only
  (coerce_runtime.rs:192-199); DO keys ride the secrets plane. Prompt-content
  export allowlists stay with [`std-telemetry.md`](std-telemetry.md)
  "Content Policy".

## Dependencies

Core: effect lifecycle, typed admission, DR-0032 failures, decide/case.
Substrate: S0 kind-map dedup; **S2 (hard prerequisite: this package's rename
+ rekey)**; S6 embedded manifests (slice 4); S5 is a PATTERN dependency only
(the ladder mirrors the `capability_bound` promotion; no shared code).
Packages: std.agent consumes the credential layer (module boundary, not a
manifest dependency); std.telemetry reads coercion runs read-side.

## v1 implementation slices

Each independently gateable per-piece; slice 1 ships INSIDE substrate S2.

1. **S2 rider — key commitments + DR-0014 amendment.** The rename itself is
   substrate work; this package contributes the key-commitment spec, the
   effect-key.maude extension (coverage AND bite), the loud store-open guard
   test for pending legacy `coerce` effects, and the "Spec amendments". Gate:
   rekey fixture proves model/prompt/schema changes rekey; fixture suites
   unchanged.
2. **`schema_coercer` provider kind + registry-honest selection.** Registry
   rows gain the kind; a claim-time SELECT establishes the four-rung ladder;
   fixture becomes the NAMED default provider. Gate: precedence tests for
   all four rungs; misconfig-hard-error regression; DO parity (secrets door
   beats registry default).
3. **Config-plane unification + credential-layer rehome.** One
   ResolvedCoercionConfig built by both doors; DO `max_tokens` drift removed;
   credential layer moves to `model_auth`, harness consumer re-pointed.
   Gate: golden precedence table (env/stored/codex × openai/anthropic);
   Anthropic OAuth-rejection regression; harness model smoke unchanged.
4. **Embedded manifest + operator CLI.** Manifest lands with S6 mechanics;
   `whip coercion status [--json]` reports resolved provider, model,
   credential source, selecting rung; import advisory lint fires on
   coerce-without-`use std.coercion`. Gate: manifest validated via the
   third-party pipeline; manifest-vs-seeded-rows drift test; status golden.

## Open naming-boundary questions

- `prompt`'s long-term owner: kept as a std.coercion source form (it lowers
  to `schema.coerce`), but it is agent-flavored authoring; if it grows
  session semantics it moves to std.agent. Decide on demand.
- `whip auth` scope: when std.agent provider packages need stored
  credentials, does `whip auth` stay coercion-owned with agent consumers, or
  become core CLI? Flagged for the std.agent concrete design.
- Provider id `native`: whether backend names surface in the id
  (`native-openai` …) is decided with the second distinct implementation.

## Deferred with cause

- **Generated-mode locked coerce artifacts** (DR-0014 "generated mode").
  Cause: BAML purged; native structured outputs synthesize the schema per
  call — no artifact plane to lock. Re-entry: a `schema_coercer` backend
  that genuinely compiles artifacts.
- **Interop mode (`.coerce` include/bind)** — not deferred, OBSOLETE; struck
  by amendment below. Re-entry: none (new interop demand = new design).
- **Repair loop / coercion diagnostic taxonomy** beyond the DR-0032 base:
  no failure corpus yet; re-enter from accumulated evidence.
- **Multi-model routing / fallback.** Cause: hidden-retry prohibition;
  single-model-per-key is what the fingerprint commits to. Re-entry: an
  explicit author-visible routing surface, never silent.
- **Content-allowlisted prompt logging/export.** Cause: policy tiers open
  (Jack); export belongs to std.telemetry. Re-entry: telemetry allowlists.
- **Kind versioning / migration tooling.** Cause: the "Renames" decision —
  pre-release one-way, no mechanism. Re-entry: first post-release rename.

## Spec amendments

1. **Decision record 0014, "Migration Target":** the "versioned contract
   compatibility or explicit migration tooling" clause is superseded by the
   "Renames" decision: one-way pre-release break, loud store-open guard on
   pending legacy-kind effects, no tooling. (Cross-cutting item 1; rides S2.)
2. **Decision record 0014, "coerce Backend Modes" + "Responsibilities":**
   interop mode / `.coerce` include-bind marked OBSOLETE (BAML-era; purged);
   artifact generation moves to the deferred list with its re-entry.
3. **Decision record 0014, "Naming" + "Core Surface":** required capability
   becomes `schema.coerce` (id == kind; `model.invoke` dies); `prompt`
   recorded as the third core source form; "provider configuration as
   library concepts" narrowed to "workflow-visible library concepts".
4. **[`coerce.md`](coerce.md), "Idempotency And Replay":** replace the
   informal commitment list with the key/fingerprint split specified here.
5. **[`coerce.md`](coerce.md), "Execution Modes":** env table reframed as
   operator-override-over-registry-default; DO secrets door + shared config
   record documented alongside; DO `max_tokens` 1024 drift corrected.

## Verdict

**Keep, as an operator-config package; build.** Thin by design and honest
about it: the backend machinery is shipped and DO-proven; the package adds
names that match the concept, a registry that actually selects, one config
record instead of two planes, a credential layer with an owner, and an
idempotency key that commits to what changes the answer. Slice 1 rides S2;
slices 2-4 are small and independently gateable.
