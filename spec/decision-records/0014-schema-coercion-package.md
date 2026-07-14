# 0014: Schema Coercion Package

Status: accepted (amended 2026-07-13 — see Amendment)

## Amendment (2026-07-13, std.coercion concrete design + substrate S2)

The target conceptual names below are now the SHIPPED names: the effect
kind/capability is `schema.coerce` (S2 rename landed; `coerce` sits in the
retired-kind store guard), the provider kind enum carries `schema_coercer`,
and the standard package is `std.coercion`. `coerce` remains the AUTHORING
keyword (core grammar) and the concrete backend/provider id — per this
record's own naming table — and is no longer implementation debt.

Amended per spec/std-coercion.md ("Idempotency And Replay"): the
`schema.coerce` idempotency key additionally commits to
`coercion_name`, `prompt_template_hash`, `output_schema_hash` (compile-time)
and `coercion_config_fingerprint` (host-supplied at kernel construction), so
a changed prompt/schema/backend re-runs future coercions instead of replaying
a stale terminal. The "provider configuration as library concepts" exclusion
is superseded for the OPERATOR plane only — i.e. narrowed to
"workflow-visible library concepts": `std.coercion` is an operator-config
package (registry rows + CLI surface), never a source construct — the core
"should not own" list stands unchanged.

Further amendments per spec/std-coercion.md ("Spec amendments"):

- **"coerce Backend Modes" + "Responsibilities":** interop mode (`.coerce`
  include/bind for existing backend artifacts) is OBSOLETE — it was BAML-era
  surface and the backend was purged; a new interop demand is a new design,
  not a re-entry. Artifact generation ("generated mode") moves to the
  deferred list with its re-entry condition: a `schema_coercer` backend
  that genuinely compiles artifacts (native structured outputs synthesize
  the schema per call, so there is no artifact plane to lock today).
- **"Naming" + "Core Surface":** the required capability is `schema.coerce`
  (id == kind per the capability-planes rule; the never-enforced
  `model.invoke` dies — the kernel and the seeded registry rows already
  said `schema.coerce`), and `prompt` is recorded as the third core source
  form lowering to the same effect (source_forms: coerce, decide, prompt).

## Decision

Core WhippleScript owns `coerce` and `decide` as typed schema-coercion effects.
The official standard package for backend/toolchain support is `std.coercion`,
not a coerce-named library.

The conceptual stack is:

```text
core workflow semantics
  durable effects, after branches, terminal status, typed success bindings,
  case/exhaustiveness, replay, revision, runtime validation

core coerce/decide surface
  unstructured or semi-structured input -> declared WhippleScript type
  durable, branchable, typed

std.coercion
  schema-coercion backend/toolchain contract for core coerce/decide effects
  artifact generation/binding, schema compatibility, provider dispatch metadata,
  deterministic fixtures, coercion evidence

coerce backend/provider
  one implementation of std.coercion's schema-coercion contract
  coerce source generation or coerce include/bind interop
  coerce SAP/parse diagnostics, client/runtime, coerce-specific evidence

model providers
  lower-level LLM clients/config used by coerce or another future coercion engine
```

coerce is fundamentally about schema alignment/coercion of model text into typed
values. It is not workflow control flow, not a decision-record system, and not a
generic model-provider abstraction. Workflow decisions remain ordinary
WhippleScript rules, `after` branches, `case`, and recorded facts.

## Naming

Target conceptual names:

```text
standard package/library: std.coercion
effect kind / capability: schema.coerce
provider kind: schema_coercer
concrete backend/provider id: coerce or coerce-schema-coercer
```

Current implementation names such as `std.coerce`, `coerce`, and
`Coerce*` are implementation debt. Specs may mention them only as current
compatibility names or implementation notes, not as the target conceptual model.

## Core Surface

Named reusable coercion:

```whip
class BugReport {
  title string
  severity "low" | "medium" | "high"
  repro_steps string[]
  likely_area string
}

coerce extractBugReport(raw string) -> BugReport {
  "Extract a normalized bug report from the text."
}

rule normalize
  when fact inbox.message as msg
=> {
  coerce extractBugReport(msg.body) as parsed

  after parsed succeeds as report {
    record report
  }

  after parsed fails as error {
    record NeedsHumanReview { source msg.id reason error.message }
  }
}
```

Inline one-off coercion:

```whip
decide "Is this safe to ship? Explain." -> {
  safe bool
  reason string
  risk "low" | "medium" | "high"
} as verdict
```

`decide` is prompt-shaped authoring sugar for an anonymous generated `coerce`
shape. It is not a separate semantic primitive and not coerce-owned control flow.

## `std.coercion` Responsibilities

`std.coercion` should own:

```text
schema-coercion backend registration for schema.coerce
coercion provider kind and capability metadata
artifact generation from WhippleScript schemas/coerce declarations
include/bind interop for existing backend artifacts such as .coerce files
schema/hash compatibility checks
backend runtime/client invocation metadata
coercion failure diagnostics and repair surfaces
evidence shape: arguments, schema hashes, generated source hash, backend/model metadata
fixture support for deterministic tests
```

The package should not own:

```text
rule scheduling
after dependency semantics
terminal status taxonomy
case/exhaustiveness
runtime typed-output admission
generic model-provider abstraction
OpenAI/Anthropic/etc provider configuration as library concepts
hidden retries or hidden context injection
direct fact writes
```

## coerce Backend Modes

The coerce backend can support two modes:

```text
generated mode
  WhippleScript types and coerce declarations are the source of truth.
  The toolchain emits locked coerce artifacts as build output.

interop mode
  Existing .coerce files are included or bound explicitly.
  The checker cross-validates coerce functions/types against locked
  WhippleScript schemas and records compatibility hashes.
```

Generated mode should be the default mental model. Interop mode is for projects
that already have coerce assets or need coerce-specific constructs that WhippleScript
does not author directly.

## Migration Target

The implementation may continue to expose `coerce` while the package system
is still being built. The target migration is:

```text
coerce effect kind       -> schema.coerce effect kind
std.coerce library row          -> std.coercion library row
builtin-coerce provider binding -> coerce backend implementing provider kind schema_coercer
```

This migration should preserve existing durable history either by versioned
contract compatibility or by explicit migration tooling. It should not be mixed
with unrelated package extractions.

## Coverage And Implementation Follow-Ups

The formal model now has an explicit `std.coercion`-style package-contract
fixture: it registers a `schemaCoerce` capability, lowers to the ordinary
`coerceEff` graph template, and still requires the kernel-owned provider-run
boundary before typed output can become a fact. This confirms that the
schema-coercion package does not need special workflow semantics.

Remaining implementation rename work:

```text
parser registry: std.coerce -> std.coercion
IR enum/type names: Coerce -> SchemaCoerce where they name the core effect
effect kind strings: coerce -> schema.coerce
required capabilities: coerce -> schema.coerce
completion facts: coerce.* -> schema.coerce.*
provider evidence kind: coerce.provider -> schema.coerce.provider
report schemas and acceptance fixtures: update exact effect/evidence names
checked examples: update assertions from coerce to schema.coerce
kernel/CLI helpers: reserve coerce names for the concrete backend/client only
real-provider scripts: keep coerce endpoint env vars as backend-specific config
durable history: define compatibility or migration for old coerce records
```

This should be a single implementation migration pass, not piecemeal drift:
exact effect names are authority, report, and fixture keys, not just comments.

## Amendment (proposed 2026-07-05): pre-release rename + idempotency-key commitments

The precursor the `schema.coerce` rename (campaign S2) was gated on — the
"idempotency-key commitments + DR-0014 amendment" — resolved/framed here so the
rename is decision-ready. Items marked **(DECIDE)** are the author's call.

### 1. The durable-history clause is moot pre-release — one-way rename

The Migration Target's "preserve existing durable history ... versioned contract
compatibility or explicit migration tooling" and "durable history: define
compatibility or migration for old coerce records" both assumed shipped stores.
The repo is **pre-release**: no persisted production store carries `coerce`-kind
effects (tests use fresh stores; migration 0001 is the single baseline). So the
rename is a **one-way, no-back-compat change** — no compatibility shim, no
migration tooling. The store-open legacy-kind guard (`RETIRED_EFFECT_KINDS`,
introduced for the signal.emit rename) gains a `coerce` row so any stray
old-kind store fails loud rather than silently deduping. This **deletes** the
migration-tooling scope from the sections above.

### 2. Effect identity and the kind-string change

The effect execution fingerprint is `H(input_json | sorted upstream effect ids)`
(store `execution_fingerprint_on`), and the coerce `input_json` already folds the
**prompt and schema name** (the coerce input builder in `rule_lowering`), so a
prompt or schema change already re-runs rather than deduping a stale result. The
rename changes these authority strings (one pass):
- effect **kind string** `coerce` -> `schema.coerce` (~85 sites incl. IR enum
  `Coerce` -> `SchemaCoerce`, `as_str`, every exhaustive match, and the
  `flow_expand` re-serialize arm the compiler will flag);
- **completion facts** `coerce.completed|failed` -> `schema.coerce.*` (the
  after-branch predicates, 6 sites);
- **capability name** `coerce` -> `schema.coerce`;
- **migration-0001 seeds** — provider `provider_coerce_builtin`, binding
  `binding_coerce_builtin`, and the `coerce` capability inside three profile
  capability lists (repo-reader / repo-writer / internet-research);
- report schemas, acceptance fixtures, checked-example assertions, IR goldens.

**(DECIDE) Model identity in the effect key.** `input_json` binds prompt + schema
but not the resolved model id, so a coerce whose only change is the model (same
prompt/schema/input) dedups to the prior result on replay. Recommendation: fold
the resolved model id (and the schema hash) into `input_json` in this same pass —
a model swap is a semantically different coercion and should re-run. If
model-stability-on-replay is instead preferred, leave as-is. Low blast radius
either way; decide it here rather than discover it later.

### Execution note

One migration pass (as the section above already instructs), model-first per
house rules: rename the kind in the effect-key / std-construct-authorization
Maude models first, then the code pass, then regen goldens + report schemas; gate
on full readiness. DR-0014 can move `proposed` -> `accepted` when this lands.
