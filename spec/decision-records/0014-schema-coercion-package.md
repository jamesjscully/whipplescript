# 0014: Schema Coercion Package

Status: proposed

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
