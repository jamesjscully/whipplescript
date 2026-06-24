# 0007: Core, Standard Libraries, and Providers

Status: proposed

## Decision

WhippleScript should keep a small kernel and move domain-specific functionality
into first-party standard libraries backed by providers. "Move out of core" does
not mean optional or unbundled by default; it means the syntax and behavior are
owned by a named library surface rather than by the irreducible rule/effect
kernel.

The formal package/library/provider boundary is defined in
[`0010-package-library-provider-boundary.md`](0010-package-library-provider-boundary.md).

The core should be:

```text
typed facts + rules + durable effects + event log + package/provider ABI
```

Standard libraries should provide the batteries:

```text
std.tracker    issue/work tracker and ready-work projections
std.agent      agent provider bindings and profiles
std.memory     explicit agent memory pools and recall context bundles
std.time       clock sources, recurring occurrence policy, and signal emission
std.ingress    external sources, typed signals, and admission providers
std.files      file stores, deterministic file/document I/O, and format codecs
std.script     pinned script capabilities
std.coord      shared coordination resources
std.messaging  communication channels and generic message envelopes
std.telemetry  event-log exporters
std.coercion   schema-coercion backends and toolchain support
```

## Shared Package Abstractions

Standard packages should reuse a small vocabulary of package-facing abstractions
instead of each package inventing its own shape.

```text
declared resource
  a named durable or external resource surface, usually declared as
  <noun> [qualifier] <name> { provider <provider>; ... }
  Examples: tracker, memory pool, file store, channel, lease, ledger, counter.

source declaration
  a named outside-observation source that maps provider observations into
  declared typed signals through observe/emit clauses.
  Examples: source clock, source http, source file, source interaction.

effect operation
  a durable rule-body operation that enqueues work, records evidence, and returns
  an effect handle with ordinary after-branch lifecycle.
  Examples: claim, recall, learn, read, write, import, export, send, exec.

projection
  a typed view over durable records or accepted external observations, usable in
  rule conditions without mutating state.
  Examples: ready issue, proposed memory, ledger entries, inbound Message.

turn access grant
  a declarative tell-clause grant that narrows one agent turn's access to a
  declared resource. The effective authority is the intersection of resource
  policy, turn grant, provider capability report, capability binding, and agent
  profile/policy.

provider capability report
  machine-readable feature and authority metadata used by the checker, runtime,
  doctor/status commands, and traces to explain whether a provider can satisfy a
  package construct.

typed signal admission
  the core boundary where an outside or cross-instance observation becomes a
  durable typed signal fact. Packages may contribute source providers, but core
  owns signal declarations, payload typing, validation, replay, and the rule that
  no invalid signal fact enters the log.
```

Declaration headers should be boring and predictable. A package resource should
prefer a block-internal `provider` clause over a one-off header word:

```whip
channel release_room {
  provider local
  destination "release"
}
```

The package import, resource declaration, and source declaration are not runtime
authority. They describe what the source means. Actual authority still comes from
provider bindings, credentials, capability grants, and profile policy. There is
no ambient fallback: provider-native tools, filesystem access, model memory,
script execution, and messaging channels do not satisfy package access unless the
accepted package contract explicitly grants that access.

## Construct Graph Classification

The standard-library list is not a list of parser exceptions. Each library must
fit the construct graph and lowering-class model from
[`../construct-grammar.md`](../construct-grammar.md),
[`../construct-graph-calculus.md`](../construct-graph-calculus.md), and
[`../construct-lowering-preservation.md`](../construct-lowering-preservation.md).

The design contract for a standard library should name:

```text
construct families it instantiates
resources and projections it provides
operation input/output ports
capabilities required by lowered effects
lowering class and runtime entrypoint class
whether the class is implemented today or reserved for later platform work
```

Current classification:

| Library | Primary construct roles | Lowering posture |
| --- | --- | --- |
| `std.tracker` | `declaration_block` tracker resources, resource-qualified ready projections, tracker lifecycle operations | needs `projection_view` and `resource_effect` / `typed_effect_call`; generic `capability_call` only fits simple request/response tracker calls |
| `std.agent` | provider/profile/skill metadata around core agent constructs | `agent`/`tell` remain core; package catalog entries are mostly `metadata`; provider execution is runtime-owned |
| `std.memory` | memory-pool resource declarations, recall/learn/curate lifecycle operations, and turn-scoped memory access grants | `recall from` is the first accepted package `capability_call`; `learn`, `curate`, `keep`/`forget`, and `with access to` need typed effect and turn-grant lowering classes |
| `std.time` | `source clock as <name>` declarations, recurrence policy, clock observation mappings | shared `source_declaration` family with `clock_source` lowering; admits typed durable signal facts only |
| `std.ingress` | `source <provider> as <name>` declarations for `cli`/`http`/`stdio`/`file`/`grpc`, typed signal admission | shared `source_declaration` family with `signal_source` lowering; delivery admits typed durable signal facts only |
| `std.files` | `file store` resource declarations, read/write/import/export operations, and turn-scoped file-store access grants | needs `resource_declaration`, `typed_effect_call`, typed fact-stream import, and `agent_turn_grant`; no ambient filesystem access |
| `std.script` | named content-pinned `exec` capabilities, typed stdin/stdout, manifest metadata, and local script provider binding | narrow `capability_call` / future `typed_effect_call`; no parser/runtime extension and no hidden process execution |
| `std.coord` | lease/ledger/counter resource declarations and operations | needs `resource_effect` with static release/exhaustiveness checks preserved in graph/lowering evidence |
| `std.messaging` | `channel` declarations, outbound `send`, generic inbound `Message` observations, explicit `source interaction` callback mappings | `send` is a typed effect operation; inbound messages are durable observations; interaction callbacks use typed signal admission |
| `std.telemetry` | event/evidence-log exporter configuration, cursor state, structural telemetry mapping, and provider targets | read-only operator/provider surface over core logs; no workflow syntax and no effect scheduling semantics |
| `std.coercion` | schema-coercion backend/toolchain metadata for core `coerce`/`decide` typed effects | `coerce`/`decide` remain core; package owns backend registration, artifacts, fixtures, and evidence |

Standard-package privilege means these libraries may get concise words in the
core-owned construct grammar. It does not mean they can invent control flow,
terminal states, provider runs, hidden context, direct fact writes, or
package-owned scheduler/lifecycle state. If a listed library requires a lowering
class that is still reserved, the spec is a design target, not an implemented
package contract.

## Kernel Surface

These surfaces remain core because the compiler or runtime must understand them
to preserve determinism, replay, static checking, or terminal correctness:

```text
workflow / input / output / failure
class / enum / sum types / literal unions / optionals
rule / when fact / guards
record / done
complete / fail
effect outbox
after branches and effect dependency edges
terminal unions and case over finite domains
flow lowering
pattern / apply
include / use package machinery
signal declarations and typed signal admission boundary
agent declaration, tell effect, and canonical agent turn lifecycle
coerce / decide typed schema-coercion effects
timer / timeout / cancel effect lifecycle semantics
event log / facts / effects / evidence / projections
capability registry and provider-binding mechanism
```

This is the part third-party libraries may not change. They can register typed
surfaces and providers, but they cannot add new control-flow semantics, mutate
facts directly, bypass the event log, or execute effects inline.

## Standard Library Surfaces

### `std.tracker`

Owns durable issue/work records:

```text
tracker declarations
ready issue projections
append-only issue and claim events
create / claim / release / finish / close / cancel / reopen
comments / notes
evidence attachments
dependencies / blockers
issue-domain claim handles backed by local leases where available
semantic conflicts
schema/capability discovery
tracker CLI and provider sync
```

Current queue/item implementation details should be replaced by this package.
A queue is a ready-work view over tracker records, not the primary product
concept.

The tracker semantic contract is append-only accepted changes plus projections:
commands request changes, events record accepted changes, and projections answer
queries such as ready work, current state, history, conflicts, and active
claims. External providers can be mutable internally, but adapters must
normalize observations into this contract with honest sync/conflict metadata.

The tracker package owns issue claims, not generic coordination leases. Generic
leases belong to `std.coord`, and provider-run leases remain core runtime
internals. Words such as `claim`, `renew`, `release`, and `lease` are reserved
by the platform construct catalog; packages may use those bare words only through
an explicit catalog privilege for the exact construct shape they lower through.

New CLI design should prefer `whip issue` or an equally direct issue namespace.
`whip items` is not part of the future package surface.

Provider examples:

```text
builtin local tracker
GitHub Issues
Linear
Jira
```

### `std.agent`

Owns agent provider bindings, profile presets, skill/context resolution, and
provider capability discovery:

```text
Codex provider
Claude provider
Pi provider
fixture provider
command harness provider
enterprise broker provider
profile presets
skill/context bundle resolution
native provider policy translation
provider capability reports
provider health checks
```

The core still owns `agent`, `tell`, `AgentRef`, capacity/readiness, provider
run state, capability/profile enforcement, and the canonical `agent.turn.*`
lifecycle. `std.agent` supplies concrete providers and package-level defaults.
`harness` remains an advanced endpoint-routing escape hatch; `provider` should
be the ordinary authoring vocabulary.

### `std.memory`

Owns explicit agent memory:

```text
memory pool declarations
recall from / learn from into / curate / keep / forget
turn-scoped memory access grants for agent turns
BM25 / vector / hybrid retrieval providers
memory entry provenance
context bundle artifacts
recall evidence and explanation
pool curation policy
optional future knowledge-engine projections
```

The core owns `agent.tell`, `agent.turn.*`, durable effects, evidence, and
artifacts. `std.memory` owns the memory-specific effect schemas, indexes,
providers, and context bundle construction. Memory retrieval may be heuristic
inside a pool, but moving memory into an agent turn must remain explicit and
auditable. Agents receive memory only through explicit `with context` values or
turn-scoped `with access to <memory-pool> { ... }` grants.

### `std.time`

Owns clock sources and recurring occurrence policy:

```text
source clock as <name>
clock observation schema
recurrence forms
timezone policy
missed occurrence policy
clock source status
```

The core owns replay-safe time values, one-shot `timer` effects, effect
timeouts, and the rule that the current clock is read only at worker/provider
boundaries. `std.time` owns recurrence and operational scheduling policy as
a source provider. It emits ordinary typed signal facts that workflows react to;
it never fires rules directly.

### `std.ingress`

Owns external sources and delivery into declared workflow signals:

```text
source <provider> as <name>
observe as <binding>
signal declarations and ingress manifests
hosted HTTP/HTTPS webhook server
gRPC / file-watch / stdin-style adapters
endpoint configuration
HMAC / bearer / shared-secret auth
delivery deduplication
payload validation and normalization
payload-to-instance correlation
signal replay and import tools
```

The core owns the runtime event log and signal admission boundary. `std.ingress`
owns provider-backed delivery mechanisms that receive outside observations and
turn them into typed durable signal facts. Source should use `signal`, not
`event`, for author-declared outside inputs. Source providers must expose an
observation schema and workflows must explicitly map observations into emitted
signals.

### `std.files`

Owns deliberate file and document I/O:

```text
file store declarations
read / write single-file operations
import / export typed structured formats
text / markdown / json / jsonl / csv / xlsx / docx / bytes codecs
path policy and provider-backed store bindings
turn-scoped file-store grants for agent turns
file-operation evidence and content hashes
```

The core owns artifacts, evidence, effect lifecycle, capability/profile
enforcement, and the generic turn-grant composition point. `std.files` owns the
file-store resource, path policy, format codecs, file-operation effect schemas,
and agent file-access grant schema.

`std.ingress.file` observes outside file arrivals or changes and emits typed
signals. `std.files` reads and writes file content deliberately. File I/O must
never be ambient: all reads and writes are effects, dynamic paths are normalized
and checked at runtime, and agent file access requires an explicit
`with access to <file-store> { ... }` grant.

### `std.script`

Owns a deliberately small process-execution surface:

```text
hosted exec manifests
script capability registration
typed stdin/stdout conventions
hash verification and evidence policy
local process provider binding
hard-off policy when scripts are disabled
```

Raw dev-profile `exec "cmd"` remains a local convenience, not the standard
package surface. Hosted `exec capability with record -> Type` is the default
custom-provider path for small request/response integrations.

The main invariant is exploit resistance: when `std.script` is not imported,
not installed, or disabled by policy, no workflow source, agent prompt,
provider output, messaging payload, or generic capability call may cause process
execution through this package. When enabled, process execution must go through a
named manifest entry with pinned bytes, typed stdin, typed output validation,
capability authorization, and runtime evidence. `std.script` is not a plugin
system and must not become an alternate way to add syntax, mutate facts, run the
control plane, schedule work, or bypass provider policy.

### `std.coord`

Owns shared coordination resources:

```text
lease
ledger
counter
lease slots 1 as mutex
lease slots N as semaphore
```

This package is standard and privileged: its operations need atomic stores and
static safety checks such as must-release and exhaustive outcome handling. The
coordination concepts should still be package-owned rather than treated as the
irreducible kernel.

The coordination state model is append-only accepted changes plus projections:
commands request changes, events record accepted changes, and projections answer
active holders, wait queues, ledger entries, counter usage, and audit queries.
`std.coord` is not an arbitrary shared-cell or database surface; it is a closed
family with fixed resource kinds and tiny verb vocabularies.

`std.coord` should use a modeled `resource_effect`-style lowering rather than
plain `capability_call`, because the checker/runtime must enforce held-resource
lifetime, release obligations, bounded waits, lease ordering, counter caps, and
ledger retention.

### `std.messaging`

Owns communication-platform messaging:

```text
channel declarations
outbound send effects
generic inbound Message envelopes
local mailbox provider
stdio provider
desktop notification provider
explicit source interaction mappings through typed signal admission
```

Messaging is for talking, not for typing arbitrary outside facts. Its native
inbound value is a generic `Message` envelope. Strongly typed domain input
belongs to `std.ingress` signals or to explicit schema coercion over message
text. Human review is a messaging/ingress use case, not a separate `std.human`
or `std.inbox` abstraction.

### `std.telemetry`

Owns read-only export tooling over the event/evidence log:

```text
OTLP/OpenTelemetry export provider
cursor-tracked log tailing
exporter configuration
attribute/redaction policy
export cursor/checkpoint state
status/report renderers
```

The event log, evidence records, local trace shape, source spans, causal ids,
effect ids, run ids, and provider-run lifecycle are core substrate.
`std.telemetry` owns the read-side projection from that substrate into operator
observability systems. It should surface through operator config, environment
variables, provider bindings, and CLI commands such as `whip otel-export`, not
workflow rule syntax.

The default policy is structural telemetry only: ids, kinds, statuses, timings,
counts, and source metadata. Fact values, prompt bodies, model responses, and
artifact contents require explicit operator allowlists. Exporters must be
cursor-tracked, failure-isolated, and replay-safe: a down collector cannot block
workflow execution, and replay/recovery cannot double-export telemetry.

### `std.coercion`

Owns schema-coercion backend/toolchain support for core `coerce` and `decide`:

```text
schema.coerce backend registration
provider kind schema_coercer
artifact generation from WhippleScript schemas and coerce declarations
include/bind interop for existing backend artifacts such as .coerce files
schema/hash compatibility checks
coercion runtime/client invocation metadata
coercion diagnostics, fixtures, and evidence shape
```

`coerce` and `decide` remain core WhippleScript typed schema-coercion effects.
`std.coercion` does not own workflow control flow. It supplies backend and
toolchain contracts for turning unstructured or semi-structured output into
locked WhippleScript types.

coerce is one concrete backend/provider under `std.coercion`. Model providers such
as OpenAI or Anthropic are lower-level execution/client configuration under coerce
or another future schema-coercion backend. They should not be modeled as the
same library as schema coercion, and coerce should not be described as "the model
provider layer" or as workflow decision semantics.

## Syntax Guidance

First-party standard libraries may justify concise syntax when the construct is
central to orchestration and statically analyzable:

```whip
use std.tracker
use std.agent
```

Third-party libraries should primarily compose through:

```text
types
events
patterns
workflows
effect schemas
provider capabilities
ordinary call/exec effects
```

They should not get arbitrary new grammar. The goal is open extension without
turning WhippleScript into a macro language or a general host-language runtime.

## Package Naming Preferences

Use names that describe the durable abstraction:

```text
std.tracker   not std.work
std.memory    not std.rag
std.time      not std.cron
std.ingress   not std.webhook
std.script    not std.exec
std.coord     not std.lease
std.messaging not std.notify, std.channel, or std.human
std.telemetry not std.otel
std.coercion  not std.coerce or std.decision
```

`std.agent` is acceptable because "agent turn" is already the core execution
concept; provider packages underneath it should name the concrete provider.

## Testing And Evaluation Tooling

Testing and evals are important, but they should not be a standard runtime
package in this design pass.

There are at least two distinct future tracks:

```text
workflow author testing: assert, dev, accept, fixtures, report expectations
package/provider author testing: package conformance, provider fixtures, lowering checks
```

These are CLI/tooling and validation surfaces. They should not require
`use std.test`, should not extend workflow runtime semantics, and should not
enter the package construct graph unless a later eval design identifies a real
workflow-level abstraction.

## Consequences

- Existing documentation that says "plugin" should be revised toward
  package/library/provider terminology.
- Existing `queue` language should be treated as a compatibility surface or
  ergonomic alias while `std.tracker` becomes the conceptual home.
- Capability checks must distinguish library import from provider authority.
  Importing `std.tracker` should not grant GitHub credentials, process
  execution, or network access.
- Standard libraries can be bundled and enabled by default in local builds while
  still having clear conceptual ownership.
- Artifact and evidence records remain kernel substrate. Domain packages may
  attach, render, retain, or export their own evidence, but there is no
  separate `std.evidence` or `std.artifact` package unless future workflows need
  first-class evidence bundles as ordinary orchestration material.

## Open Questions

- Which standard libraries are implicitly available in v0 versus explicitly
  imported?
- Should package imports use `use`, `import`, or a split form that distinguishes
  source libraries from provider bindings?
- How should the checker report missing library versus missing provider versus
  denied capability?
- Which currently implemented syntax should be preserved as compatibility
  aliases during the transition?
