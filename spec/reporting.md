# Reporting Contract

Status: draft v0 contract

WhippleScript reports should make source intent, durable runtime state, and
validation results inspectable without introducing a second execution model.
This is the reporting lesson to borrow from Cucumber Messages: stable
machine-readable envelopes, source metadata, and explicit result records rather
than free-text step matching.

## Scope

Current report surfaces:

- `whip --json check <workflow.whip>...`
- `whip --json compile <workflow.whip>`
- `whip --json dev <workflow.whip>`
- `whip dev <workflow.whip> --stream ndjson`
- `whip --json accept <fixture.json>`
- JSON inspection commands such as `facts`, `effects`, `evidence`,
  `diagnostics`, and `trace`

Draft JSON Schema files:

- [`report-schemas/check_report_v0.schema.json`](report-schemas/check_report_v0.schema.json)
- [`report-schemas/compile_report_v0.schema.json`](report-schemas/compile_report_v0.schema.json)
- [`report-schemas/dev_report_v0.schema.json`](report-schemas/dev_report_v0.schema.json)
- [`report-schemas/dev_stream_v0.schema.json`](report-schemas/dev_stream_v0.schema.json)
- [`report-schemas/local_trace_v0.schema.json`](report-schemas/local_trace_v0.schema.json)
- [`report-schemas/acceptance_fixture_v0.schema.json`](report-schemas/acceptance_fixture_v0.schema.json)
- [`report-schemas/acceptance_report_v0.schema.json`](report-schemas/acceptance_report_v0.schema.json)
- [`report-schemas/construct_graph_v0.schema.json`](report-schemas/construct_graph_v0.schema.json)
- [`report-schemas/lowered_ir_report_v0.schema.json`](report-schemas/lowered_ir_report_v0.schema.json)
- [`report-schemas/ir_model_search_obligations_v0.schema.json`](report-schemas/ir_model_search_obligations_v0.schema.json)
- [`report-schemas/artifact_model_search_obligations_v0.schema.json`](report-schemas/artifact_model_search_obligations_v0.schema.json)
- [`report-schemas/platform_construct_catalog_v0.schema.json`](report-schemas/platform_construct_catalog_v0.schema.json)
- [`report-schemas/package_set_v0.schema.json`](report-schemas/package_set_v0.schema.json)
- [`report-schemas/package_manifest_v0.schema.json`](report-schemas/package_manifest_v0.schema.json)
- [`report-schemas/package_lock_v0.schema.json`](report-schemas/package_lock_v0.schema.json)
- [`report-schemas/package_sync_v0.schema.json`](report-schemas/package_sync_v0.schema.json)
- [`report-schemas/package_check_v0.schema.json`](report-schemas/package_check_v0.schema.json)
- [`report-schemas/package_contract_v0.schema.json`](report-schemas/package_contract_v0.schema.json)
- [`report-schemas/test_report_v0.schema.json`](report-schemas/test_report_v0.schema.json)
- [`report-schemas/verified_artifacts_v0.schema.json`](report-schemas/verified_artifacts_v0.schema.json)

Run [`../scripts/check-report-schemas.sh`](../scripts/check-report-schemas.sh)
to generate provider-language `check`, `compile`, `dev`, `trace --check`,
`dev --stream ndjson`, and `accept` reports and validate them against these
schemas. Run
[`../scripts/check-artifact-admission-differential.sh`](../scripts/check-artifact-admission-differential.sh)
to compare Python schema/artifact admission against native `whip verify-report`
on a generated matrix of malformed construct-graph and lowered-IR artifacts
across full check reports, compile reports, graph-only verified artifact
bundles, lowered-IR verified bundles, and full verified artifact bundles.
The same matrix mutates the shared report spine, including source/IR hashes,
snapshots, contract registries, package-contract digests, and platform
catalogs, and includes schema-valid semantic duplicate IDs for graph nodes,
ports, edge refs, dependency refs, and lowered core objects.

Reports are descriptive. They must not change rule readiness, effect creation,
provider routing, table seeding, retries, authorization, or assertion
semantics.

## Shared Diagnostics

Diagnostics are report data, not free-form text. New report surfaces should
follow [`error-handling.md`](error-handling.md): stable code, severity, source
span when available, provenance, related refs, suggestions or fixits when safe,
and redaction metadata for provider/runtime boundaries.

During migration, some schemas still expose smaller diagnostic shapes such as
`message`, `suggestion`, and `source_span`. Those are compatibility surfaces,
not the long-term target. The long-term target is one shared diagnostic object
rendered consistently by `check`, `compile`, `dev`, `accept`, package commands,
construct graph reports, lowered IR reports, verified artifact admission, and
runtime inspection commands.

`whip lint --json` should emit a report using the same diagnostic object model.
`whip test --json` should emit a scenario-oriented test report using the same
diagnostic object model and linking detailed dev/trace reports only for
debugging.
`whip lsp` does not need its own report schema, but protocol tests should show
that LSP diagnostics preserve the same codes, spans, provenance, suggestions,
fixits, and package refs as CLI reports. See
[`editor-tooling.md`](editor-tooling.md) and
[`workflow-testing.md`](workflow-testing.md).

## Shared Source Metadata

`check`, `compile`, and `dev` include `source_metadata`:

```json
{
  "tags": [
    {
      "name": "acceptance",
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "source_span": {"start": 1, "end": 12}
    }
  ],
  "descriptions": [
    {
      "value": "Codex completes both assigned language tasks",
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "source_span": {"start": 13, "end": 72}
    }
  ],
  "targets": {
    "assertion:a219774c2ee6f69f": {
      "target_kind": "assertion",
      "target": "a219774c2ee6f69f",
      "tags": ["acceptance"],
      "description": "Codex completes both assigned language tasks"
    }
  }
}
```

Target keys use `<kind>:<target>`. Current target kinds are `workflow`,
`table`, `rule`, and `assertion`.

## `check --json`

`check --json` emits an array with one report per input path. Successful entries
use:

```json
{
  "schema": "whipplescript.check_report.v0",
  "path": "examples/provider-language-e2e.whip",
  "status": "ok",
  "workflow": "ProviderLanguageE2E",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {},
  "contract_registry": {},
  "construct_graph": {
    "schema": "whipplescript.construct_graph.v0",
    "graph_id": "graph_...",
    "platform_version": "whipplescript-...",
    "package_lock_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "source_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "nodes": [],
    "ports": [],
    "edges": [],
    "effect_dependencies": [],
    "derived_facts": [],
    "diagnostics": []
  },
  "lowered_ir_report": {
    "schema": "whipplescript.lowered_ir_report.v0",
    "graph_id": "graph_...",
    "accepted_program_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "lowerer_version": "whipplescript-...",
    "package_lock_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "source_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "node_lowerings": [],
    "edge_lowerings": [],
    "dependency_lowerings": [],
    "core_objects": [],
    "derived_facts": [],
    "diagnostics": []
  },
  "model_search": {
    "status": "ok",
    "searches": 0,
    "solutions": 0,
    "no_solutions": 0,
    "ir_searches": 0,
    "artifact_searches": 0,
    "obligations": []
  }
}
```

Diagnostic entries use `"status": "error"` and include an `error.kind` such as
`"io"`, `"diagnostics"`, `"package_lock"`, or `"construct_graph"`. Parser
diagnostics include message, suggestion, and source span offsets. Construct
graph diagnostics include stable codes, severity, structural refs, and source
span offsets.

`contract_registry` is the normalized library/effect contract view used for
the check. `construct_graph` is the normalized static construct graph derived
from that registry and the parsed IR. The emitted slice records package-backed
capability-call effects, their package effect-contract nodes, compiler-owned
built-in effect-operation nodes, resolved package capability edges, and source
effect dependencies. `timer.wait` nodes advertise `schedule` plus
`schedule_template`, while ordinary effect-operation nodes advertise `effect`
plus `effect_graph_template`. It also records typed `event` declarations as
compiler-owned event-source nodes that lower to admission templates, not
durable event occurrences, and top-level assertions as compiler-owned
assertion-check nodes. Rule triggers are emitted as compiler-owned
rule-template nodes; rules that can record facts advertise rule-owned
`fact_record` output templates for those fact schemas. Guard and assertion
projection queries are emitted as checker-owned projection-read metadata nodes.
Compiler-owned `model_search.guard_source` and
`model_search.terminal_branch_source` derived facts anchor generated IR search
source spans without turning those checks into runtime constructs. Native
`whip verify-report` additionally treats a readable source file as authoritative
for check and compile reports, including package-locked reports: it recompiles
the source, checks the report `source_hash`, `ir_hash`, snapshot, and
construct-graph `source_digest`, re-emits the construct graph and lowered IR
report from the admitted contract registry and package digests, and compares
those canonical artifacts to the report. When `model_search` is present, the
same source-backed path regenerates IR model-search obligations, compares
their source spans to the ledger, and checks the compiler-emitted
artifact-search obligation artifact against the non-IR ledger rows.
Its `derived_facts` include package/lowering facts plus checker-owned
`construct_graph_validator` facts for
accepted graph predicates covered by the current validator, including node
profile, declared-interface, capability, output, node/port consistency, port
profile, edge-compatibility, required-port cardinality, effect-dependency, and graph
acceptance predicates used by generated formal checks. The graph-wide
acceptance fact cites the concrete graph ID, node IDs, port IDs, edge refs, and
effect dependency refs in the emitted artifact. Node output declarations use
the same object-kind and runtime-entrypoint vocabulary as `lowered_ir_report`:
for example, ordinary effect nodes advertise `effect` plus
`effect_graph_template`, not older implementation terms such as
`effect_object` or `kernel.graph_commit`.
Port profile facts cite the port ID, owner node, direction, kind, type, phase,
contract version, cardinality, and resource identity, so generated formal
`port(...)` facts are backed by validator-owned trace evidence rather than raw
artifact inventory alone.
Edge compatibility facts cite labeled metadata witnesses for both endpoint
ports, including owner, direction, kind, type, phase, contract version,
cardinality, resource identity, resolution reason, and resolution evidence.
This keeps generated formal checks tied to the concrete port metadata that the
checker accepted rather than to endpoint IDs alone.
The artifact bridge recomputes construct graph endpoint inventory directly:
node port refs must be owned by the node with the declared direction, declared
interfaces must be satisfied by concrete same-direction ports with compatible
phase coverage and cardinality, edge provider nodes and endpoint ports must
exist, providers must own the provided ports, endpoint
kind/type/phase/version/resource compatibility must hold, and effect
dependencies must name known endpoints and supported predicates.
Rewritten validator-owned edge facts cannot make a spoofed provider node or
incompatible endpoint pair admissible.
Native `verify-report` also rejects bad embedded construct-graph schema IDs and
missing, wrong-typed, or unknown semantic fields on construct graph roots,
collection fields, string-ref fields, declared-interface collections, declared
interfaces, ports, edges, and effect dependencies, and validates construct graph
source-span objects, required scalar ID fields, declared-interface scalar and
nullable string fields, optional node metadata object shape, validator-owned
derived-fact metadata, validator-owned diagnostic objects, and validator-owned
input-ref sets, so schema-forbidden stale graph data, malformed diagnostic
metadata, or malformed provenance anchors, including exact duplicate
derived-fact entries from any subsystem, are not silently ignored when the
Python JSON Schema gate is not the only verifier in use.
Unknown, missing, or unmatched output declarations are construct graph
diagnostics and do not receive validator-owned facts.
Required-port cardinality facts cite the required port and selected edge refs;
aggregate cardinalities also cite concrete `order_index` witnesses, and
`named-many` cites concrete `resource_key` witnesses. The artifact validator
and generated bridge reject schema-valid reports whose aggregate ordering or
resource-key metadata is missing, duplicated, non-contiguous, or attached to a
scalar cardinality. The generated bridge also rejects schema-valid output
vocabulary outside the current runtime handoff vocabulary, even if the
validator-owned `node.output` evidence was rewritten to match the stale graph.
It also rejects schema-valid lowering class/lifecycle-profile mismatches even
when the validator-owned `node.profile` evidence was rewritten to match the
stale graph.

`lowered_ir_report` is the deterministic lowering inventory for the accepted
construct-graph slice. It records each graph node and edge exactly once, lowers
effect-operation nodes to `effect` core objects entering through
`effect_graph_template`, except `timer.wait`, which lowers to a `schedule`
core object entering through `schedule_template`. It lowers source effect
dependencies to `dependency` objects entering through
`effect_dependency_template`, and preserves capability-resolution edges as
static relations that emit no runtime objects. Event-source nodes lower to
`signal_source` core objects entering through
`signal_source_template`; assertion-check nodes lower to `assertion` core
objects entering through `assertion_check`; rule-template nodes lower to
`rule` core objects entering through `rule_template` plus one `fact` core
object entering through `fact_record` for each fact schema the rule body can
record. These fact objects are static rule-owned templates, not precommitted
fact occurrences. Projection-read metadata nodes and package effect-contract
metadata nodes emit no runtime objects.
Its `derived_facts` are owned by `lowered_ir_validator` and record the accepted
lowering coverage, aggregate and per-field node lifecycle inputs, aggregate and
per-field node output compatibility evidence against the construct graph's
allowed object-kind/runtime-entrypoint vocabulary, aggregate and per-field
node/edge/dependency preservation inputs, core object
owner/entrypoint checks, deterministic lowering identity, report completeness,
ownership uniqueness, no-runtime-input evidence, and runtime-boundary inventory
available to generated formal checks.
Node lifecycle input facts cite the graph/node identity, source lowering class,
accepted construct family, accepted lifecycle profile, produced core-object
IDs, and each produced object's object kind and runtime entrypoint. This is the
trace the generated formal bridge uses to derive lowering-class lifecycle
acceptance rather than bridge-local defaults. Core
object metadata arrays such as `resource_refs`, `capability_refs`, and
`version_refs` are unique ref sets when present, and graph-wide owner evidence
cites each object's owner kind as well as owner ref so generated checks do not
erase the distinction between node, edge, and dependency ownership.

The `check_report_v0`, `compile_report_v0`, and `verified_artifacts_v0` schemas
require an embedded `package_contract` artifact plus the selected artifact
surface: construct graph evidence is always present for successful artifact
entries, while lowered IR evidence is present for full reports, full verified
bundles, and lowered-IR verified bundles. The package contract carries a digest
over the canonical sorted-object JSON for the locked package summaries, platform
construct catalog, normalized contract registry, and diagnostics; the construct
graph cites that `package_contract_digest`. Derived fact `input_refs` must be
unique, so duplicated trace witnesses are rejected at schema validation before
generated formal bridge admission. The artifact bridge also recomputes exact
lowered-node, lowered-edge, and lowered-dependency coverage from the accepted
construct graph, rebuilds core-object ownership from the lowering inventories,
and rejects spoofed owner refs even when validator-owned owner facts have been
rewritten to match the spoofed artifact. It also validates the required
`entrypoint_refs` payloads for lowered runtime entrypoints such as `fact_record`,
`signal_source_template`, `schedule_template`, `rule_template`,
`effect_dependency_template`, and `assertion_check` against the accepted graph
and lowered core-object identity where those values are determined, so missing
or spoofed entrypoint payloads cannot be hidden behind validator facts: core
object entrypoint facts cite the concrete `entrypoint_refs` key/value
witnesses they validate. Native `verify-report` also rejects missing or unknown
semantic fields on lowered IR report roots, lowering entries, and core objects,
validates lowered core-object source-span plus validator-owned diagnostic
objects, validates the embedded lowered-IR schema ID, lowered IR collection
fields, required scalar ID fields such as `lowerer_version` and
`lowering_class`, lowered dependency `preserved_predicate` shape,
validator-owned derived-fact metadata and input-ref sets, rejects exact
duplicate derived-fact entries, and rejects unknown entrypoint-ref keys for the
exact runtime entrypoints above.
The release schema gate validates that
the embedded artifact schemas in `check_report_v0`,
`compile_report_v0`, and `verified_artifacts_v0` remain aligned with each other
and with the standalone artifact schemas for top-level required fields,
properties, schema constants, inventory uniqueness flags, diagnostics arrays,
and derived-fact trace shape. It also rejects schema-valid-looking verified
bundles whose embedded artifact objects have been erased to empty objects.
The dedicated
`construct_graph_v0` and `lowered_ir_report_v0` schemas remain the deeper
artifact contracts for nested entry shapes and validator-owned trace details.
`construct_graph_v0` treats graph string-array inventories, declared interface
entries, and evidence arrays as unique ref sets; `lowered_ir_report_v0` does
the same for node, edge, and dependency lowering witness arrays plus core
object metadata ref arrays. Both artifact schemas also reject exact duplicate
top-level inventory entries and exact
duplicate derived-fact entries. Semantic identity uniqueness, such as two
different objects with the same `node_id` or `object_id`, is enforced by the
artifact validators and executable artifact-report checks rather than by JSON
Schema alone. `whip verify-report <report.json>` re-validates successful
`check --json` reports, successful `compile --json` reports, graph-only
`whipplescript.verified_artifacts.v0` bundles emitted with
`--emit construct-graph`, and lowered/full verified bundles emitted with
`--emit lowered-ir` or `--emit artifacts` by first checking the report schema
identity and report envelope shape it depends on. Verified bundle envelopes and
entries are closed shapes, and the native verifier enforces the emit-specific
artifact surface: graph-only bundles must not carry `lowered_ir_report`, while
lowered/full bundles must carry both `construct_graph` and `lowered_ir_report`.
For `check --json` arrays,
the CLI validates every entry as either an `ok` entry or a shaped `error` entry
before it filters to the successful entries, so malformed failed entries cannot
be hidden behind a valid artifact in the same array. It also recomputes the
embedded `ir_hash` from `snapshot`, recomputes
`package_contract.package_contract_digest`, checks that the embedded package
contract registry matches the report `contract_registry`, requires the
contract-registry spine (`libraries`, `constructs`, `effect_contracts`, and
`diagnostics`) at both locations, requires `package_contract.manifests` and a
verifier platform catalog match, and, when the source path is readable,
re-emits the construct graph and lowered IR report from source plus admitted
registry context before accepting the artifacts.
The contract registry may also include `fixture_outcomes` metadata for
user-facing workflow tests. Those entries map runtime-facing package surfaces to
deterministic fixture outcomes and stable diagnostics for `whip test` and LSP
completion. They are tooling metadata only: they do not contribute runtime graph
nodes, change accepted program identity, or grant provider authority.
64-character lowercase-hex `package_lock_digest`, rejects non-empty
`package_contract.diagnostics` or `contract_registry.diagnostics`, validates
locked package-library `input_schema` and `output_schema` fragments against the
supported package `Value<T>` subset, checks that package `capability_call`
construct registrations provide required top-level fields for their target
`input_schema`, checks that locked package construct/effect vocabulary is
supported by the verifier's platform construct catalog, checks that package
construct `target_capability` references and package effect
`required_capabilities` resolve to package `capability.call` effect contracts,
checks that locked package construct keywords are unambiguous within their
scope, checks that
`package_contract.platform_version` and `platform_construct_catalog` match the
verifier's current platform contract, checks that
`construct_graph.package_contract_digest` and `construct_graph.package_lock_digest`
match the package contract, checks that `construct_graph.graph_id` is derived
from `construct_graph.source_digest`, and checks that
`lowered_ir_report.accepted_program_digest` is derived from `graph_id` plus the
same snapshot before recomputing the Rust construct-graph and lowered-IR
validator facts, checking graph/lowered identity, rejecting validator
diagnostics, and comparing the exact validator-owned `derived_facts` traces
without requiring Maude. Full JSON Schema validation remains part of the Python
schema/report gates. The generated lowered-IR bridge reuses the generated
construct-graph bridge admission checks for the embedded construct graph before
checking lowered-IR trace evidence, so direct lowered-IR bridge runs reject
stale construct graph evidence and spoofed graph inventory rather than relying
on callers to run the construct-graph bridge first. The generated bridge
scripts require an explicit compiler-emitted verifier catalog. The CLI
`--model-search` path writes the
current compiler catalog to a temporary file and passes it to bridge
subprocesses with `--platform-catalog`, so inherited verifier-catalog
environment cannot steer generated checks. Standalone bridge scripts accept the
same `--platform-catalog <path>` binding, or the
`WHIPPLESCRIPT_PLATFORM_CATALOG_PATH` fallback. The release schema and formal
gates generate that file with `whip package catalog` for standalone bridge
runs. Missing verifier-catalog binding is an admission error. The verifier
catalog is admitted as a closed vocabulary: scalar vocabulary arrays, family
IDs, and lowering IDs must be unique, lowering families and scopes must resolve
inside the catalog, and lowering interface requirements must cite declared
interface kinds before the bridge will use the catalog to admit package
constructs. When a report includes `model_search`, the same command also
validates the ledger counters and re-derives the artifact-search obligations
from the admitted construct graph, lowered IR inventory, and compiler-emitted
platform catalog. The platform-catalog obligation proves lowering-class
static-safety and authority-profile acceptance; concrete output object and
runtime-entrypoint preservation still comes from the lowered IR obligations.
The companion
[`../scripts/validate-artifact-reports.py`](../scripts/validate-artifact-reports.py)
script applies the same report-admission contract while also exercising the
Python bridge/schema path: construct-only bundles receive construct-graph
admission, and lowered/full bundles additionally receive lowered-IR identity
and trace admission. The standalone
[`../scripts/validate-model-search-report.py`](../scripts/validate-model-search-report.py)
script first validates the outer `check_report_v0` or `compile_report_v0`
envelope, then requires model-search artifacts to pass that admission path
before deriving expected artifact obligations, so an invalid report envelope or
stale validator trace cannot be hidden behind a self-consistent ledger. The
release schema gate runs these validators over generated core and
package-backed reports and includes schema-valid stale-evidence fixtures to
prove the trace comparison is active.
`whip verify-report --emit construct-graph|lowered-ir|artifacts <report.json>`
uses the same admission path and then writes a
`whipplescript.verified_artifacts.v0` JSON bundle containing the selected
validated artifact surface. Each bundle entry carries the admitted `snapshot`,
`ir_hash`, and artifact identities; lowered/full bundles also carry enough
context for consumers to recompute the lowered program digest instead of relying
on the original report envelope. This is the executable handoff for generated
checker bridges and later compiler stages: consumers should read artifacts from
this post-admission bundle instead of trusting unchecked report JSON. `--emit
lowered-ir` carries the construct graph as an explicit dependency because
lowered IR is only meaningful relative to the accepted graph it preserves. Full
`--emit artifacts` bundles and `--emit lowered-ir` bundles can be fed back
through `whip verify-report` for native revalidation. `--emit construct-graph`
remains a targeted partial input for construct-graph bridge, Python
artifact-admission checks, and native graph-level revalidation rather than a
complete lowered-program envelope; asking native `whip verify-report` to emit
`lowered-ir` or `artifacts` from a graph-only bundle is rejected because no
`lowered_ir_report` has been admitted.
`check --model-search` and `compile --model-search` use the same bundle shape
internally for construct-graph and lowered-IR artifact bridge searches, and run
the platform-catalog bridge over the same explicit verifier catalog, before
recording the `model_search` ledger in the final report.
The generated Maude bridge scripts accept a successful check-report entry, a
successful compile report, or a verified artifact bundle entry and then validate
the digest chain around the embedded artifacts before emitting obligations:
`ir_hash` must match `snapshot`, the package contract digest must match the
embedded package contract, construct-graph package references must match the
package contract, and lowered-IR accepted-program digest must match
`graph_id + snapshot` when the lowered bridge runs. The construct graph bridge
emits node, edge, graph aggregation, and accepted-program Maude obligations
from the admitted graph; the accepted-program obligation uses
`construct_graph_validator`-owned adequacy predicates for source/lock
determinism, closed registries, validator fact accountability/consistency,
namespace stability, authority scoping, phase/cardinality and version checks,
diagnostics, and declared lowering/lifecycle boundaries. The
construct graph bridge also rejects duplicate logical graph members, including
node IDs, port IDs, implicit edge refs, and effect dependency refs, before
derived-fact trace comparison. The lowered IR bridge likewise rejects duplicate
logical lowered members, including node lowering refs, implicit edge lowering
refs, dependency lowering refs, and core object IDs, before trace comparison. A
check report array with exactly one entry is selected automatically.
Multi-entry check reports and verified artifact bundles must pass
`--entry-index <n>` so a standalone bridge run cannot silently verify only the
first artifact in a larger report.

With `--model-search`, successful reports include `model_search`. `ir_searches`
counts the typed-IR Maude searches for guards, dependencies, revisions,
terminal branches, and assertions. `artifact_searches` counts generated Maude
searches produced from the emitted `construct_graph`, `lowered_ir_report`, and
compiler platform catalog, including construct graph acceptance, lowering
preservation, runtime lifecycle handoff for the covered artifact slice, and
platform lowering-class lifecycle acceptance. The `obligations` ledger has one
entry per generated search with its category (`ir`, `artifact.construct_graph`,
`artifact.lowered_ir`, or `artifact.platform_catalog`), source span, human
description, formal predicate edge, and expected/actual outcome. Successful
entries must have matching `expected` and `actual` values, so CI can audit the
specific generated obligations rather than only aggregate counters. The
validator derives the expected artifact ledger from the emitted
`construct_graph`, `lowered_ir_report`, and admitted platform catalog:
construct nodes, construct edges, graph aggregation, accepted-program evidence,
lowered edges, lowered dependencies, lowered nodes, lowering boundary, graph
preservation, per-core-object runtime handoff, aggregate runtime lifecycle
handoff, and catalog lowering-class lifecycle rows must appear in that order
with their concrete node, port, dependency, core object, graph, and catalog
lowering identifiers. Per-core-object runtime handoff rows use the lowered core
object's own `source_span`, not the graph's aggregate span, so stale
object-level provenance is visible in the model-search ledger.
The report schemas reject empty model-search formal endpoints before semantic
admission. For IR obligations, both native `whip verify-report` and the
standalone report validator currently validate counters, status, category
indexes, non-empty formal endpoints, and the known compiler-generated predicate
vocabulary (`guard-*`, dependency predicates, terminal-branch predicates,
assertion read-only checks, and revision checks). They also require
`ir_model_search_obligations` when `ir_searches > 0`, bind it to the report
`source_hash` and `ir_hash`, and compare each IR ledger row against that
compiler-emitted obligation artifact. The Python report validator validates the
artifact against the standalone `ir_model_search_obligations_v0` schema; native
`whip verify-report` enforces the same admission-critical fields directly
(schema id, source/IR hashes, non-empty generator, positive row indexes,
non-empty endpoints, expected outcome, and source-span identity). Each IR row
must also be supported by the
embedded `snapshot`: guard predicates require a guarded rule `when`, dependency
predicates require a matching lowered dependency, terminal-branch predicates
require a terminal branch on the rule, assertion predicates require an existing
assertion index, and revision predicates require the matching rule/effect or
dependency structure. The validators also derive the exact ordered IR-search
sequence from the embedded `snapshot`, including the generated description,
expected number, per-predicate distribution,
`(upstream, predicate, downstream)` endpoint multiset, per-endpoint outcome
distribution, and edge/outcome order. A ledger and obligation artifact that agree
with each other but omit an entire generated IR obligation family, swap one
generated family for another, duplicate one supported endpoint while dropping
another, flip a generated expected outcome, reorder otherwise valid generated
rows, or carry a stale generated description are rejected. The validators now
compare IR obligation source spans against construct-graph anchors when the
obligation has a unique graph-backed source: guard obligations compare against
compiler-owned `model_search.guard_source` facts, terminal-branch obligations
compare against compiler-owned `model_search.terminal_branch_source` facts,
dependency obligations compare against the matching `effect_dependencies` span,
assertion read-only obligations compare against the ordered assertion node span,
revision rule obligations compare against the rule `when0` node span, and
revision effect-attribution obligations compare against the unique matching
effect node span. The dependency coverage includes `succeeds`, `fails`,
`completes`, and `revision-completes-cancelled` obligations. They still do not
fully re-derive ambiguous guard, terminal-branch, or dependency spans for
portable report bundles when the original source file is not readable and the
construct graph does not provide a unique anchor; that stronger check requires
an embedded source/IR obligation witness or a richer parseable IR obligation
source.
For artifact-search obligations, successful reports with `artifact_searches > 0`
also include `artifact_model_search_obligations`. This artifact is bound to the
report `source_hash`, `ir_hash`, package-contract digest, construct-graph ID,
and lowered accepted-program digest, and records one non-IR obligation row per
artifact search. Native `whip verify-report` and
`scripts/validate-model-search-report.py` compare every non-IR ledger row
against that artifact by category-local row index, category, description,
formal endpoints, expected outcome, and source span. The Python validator also
validates the artifact against
`artifact_model_search_obligations_v0.schema.json` and re-derives the expected
artifact obligations from the admitted graph, lowered report, and platform
catalog, so a stale durable artifact, a stale ledger, or a self-consistent pair
that no longer follows the admitted artifacts is rejected.
[`../scripts/validate-model-search-report.py`](../scripts/validate-model-search-report.py)
and `whip verify-report` both validate the ledger only after validating the
report envelope and admitting the embedded construct graph and lowered IR
evidence through the verifier catalog, artifact identity checks, and
validator-owned trace comparison. The release gate exercises that path for both
a package-backed memory workflow and core
event-source, schedule, rule, assertion, fact, effect, and dependency compile
reports; it also feeds those compiler-emitted reports directly to the
construct-graph and lowered-IR bridge scripts. The same gate includes
schema-valid tampered construct graphs for scalar aggregate metadata, duplicate
scalar resolution, reserved graph output vocabulary with matching
validator-owned `node.output` evidence, lowering class/lifecycle-profile
mismatch with matching validator-owned `node.profile` evidence, missing
catalog-required lowering interfaces with matching validator-owned
`node.interfaces` evidence, `many` without ordering evidence, `named-many`
without resource-key evidence, direct lowered-IR bridge inputs with stale
construct graph evidence or spoofed construct graph providers, lowered reports
whose accepted graph output vocabulary no longer admits the emitted core
objects, and stale model-search construct evidence, plus a model-search report whose
artifacts are intact but whose outer compile-report schema is invalid, plus a
model-search report whose artifact ledger and durable artifact-search
obligation artifact disagree, plus an IR model-search report whose ledger and
obligation artifact agree on a
nonexistent snapshot endpoint, plus IR model-search reports whose ledger and
obligation artifact agree while omitting a snapshot-implied generated search,
preserving the total count with the wrong predicate distribution, or preserving
the total and predicate counts with the wrong endpoint distribution, or
preserving the endpoint distribution with the wrong expected outcome mix, or
preserving all aggregate distributions with the wrong row order or a stale
generated IR description, or carrying a stale guard, terminal-branch,
dependency, assertion, or revision source span while the ledger and obligation
artifact agree. It also rejects duplicate
platform-catalog vocabulary and duplicate verifier-catalog lowering IDs before
direct bridge generation.
It also checks that the embedded check/compile
IR-obligation schema stays aligned with the standalone schema. It requires the artifact validator, generated
construct-graph bridge, model-search validator, and `whip verify-report` to
reject the relevant fixture. The same gate also feeds a construct-only
`whip verify-report --emit construct-graph` bundle through the Python artifact
validator and directly to the generated construct-graph bridge, proving that
post-admission bundle output is a real graph-level handoff rather than only a
reporting convenience. It also confirms native `whip verify-report` rejects that
construct-only bundle as incomplete for full lowered-program revalidation.
Release readiness additionally runs
[`../scripts/check-artifact-admission-differential.sh`](../scripts/check-artifact-admission-differential.sh),
which mutates compiler-emitted package-backed artifacts across root fields,
node fields, declared interfaces, ports, edges, dependencies, lowered entries,
core objects, entrypoint refs, verified-bundle envelope fields, and emit-mode
artifact presence. It also mutates lowered-IR core objects through the
`event_record`, `event_projection`, and `diagnostic_record` entrypoints with
explicit handoff refs so native and Python validators prove those supported
runtime handoff shapes remain executable. It also mutates locked package
effect-contract `input_schema` and `output_schema` fragments after rehashing
the package contract, and mutates matching construct input fields after
rehashing, and mutates construct field vocabulary after rehashing, so
unsupported package schema vocabulary, impossible construct/input pairings, and
package construct vocabulary outside the platform catalog, missing construct
targets, missing effect required-capability targets, or ambiguous package
construct keywords cannot pass report admission behind a valid digest. It also
mutates package constructs to target catalog-known but platform-internal
lowerings, proving that verifier catalog knowledge does not make a lowering
package-authorable. It also mutates schema-valid graph and lowered-IR artifacts
to duplicate logical IDs
while changing a non-identity field, proving semantic identity uniqueness is
checked by artifact admission rather than only by exact JSON array uniqueness.
The gate requires schema/artifact admission and native `whip verify-report` to
agree on every malformed report.

## `compile --json`

`compile --json` emits one report object. Successful reports contain the
compiled snapshot plus the same contract registry, construct graph, and lowered
IR report artifacts used by `check --json`. With `--model-search`, successful
compile reports also include the same `model_search` object used by
`check --model-search`; non-JSON `compile` rejects `--model-search` because
stdout remains the raw IR snapshot:

```json
{
  "schema": "whipplescript.compile_report.v0",
  "path": "examples/provider-language-e2e.whip",
  "workflow": "ProviderLanguageE2E",
  "source_hash": "...",
  "ir_hash": "...",
  "snapshot": "...",
  "source_metadata": {},
  "contract_registry": {},
  "construct_graph": {
    "schema": "whipplescript.construct_graph.v0",
    "graph_id": "graph_...",
    "platform_version": "whipplescript-...",
    "package_lock_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "source_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "nodes": [],
    "ports": [],
    "edges": [],
    "effect_dependencies": [],
    "derived_facts": [],
    "diagnostics": []
  },
  "lowered_ir_report": {
    "schema": "whipplescript.lowered_ir_report.v0",
    "graph_id": "graph_...",
    "accepted_program_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "lowerer_version": "whipplescript-...",
    "package_lock_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "source_digest": "0000000000000000000000000000000000000000000000000000000000000000",
    "node_lowerings": [],
    "edge_lowerings": [],
    "dependency_lowerings": [],
    "core_objects": [],
    "derived_facts": [],
    "diagnostics": []
  }
}
```

Compile failures emit a structured error report instead of plain stderr:

```json
{
  "schema": "whipplescript.compile_report.v0",
  "status": "error",
  "path": "examples/package-memory.whip",
  "error": {
    "kind": "package_lock",
    "message": "package lock `missing.json` could not be read: ..."
  }
}
```

For construct graph and lowered IR failures, the report includes the partial
`construct_graph` and/or `lowered_ir_report` artifact that produced the
diagnostics.

## `dev --json`

`dev --json` emits one validation report after the local run loop:

```json
{
  "schema": "whipplescript.dev_report.v0",
  "instance_id": "inst_...",
  "workflow": "ProviderLanguageE2E",
  "source_metadata": {},
  "steps": [],
  "workers": [],
  "diagnostics": [],
  "provider_runs": {
    "summary": {"total": 12, "artifact_count": 12},
    "groups": [
      {"provider": "fixture", "status": "completed", "count": 12, "artifact_count": 12}
    ]
  },
  "provider_artifacts": {
    "summary": {"total": 12},
    "groups": [
      {"kind": "stdout_ref", "mime_type": "text/plain", "count": 6},
      {"kind": "transcript_ref", "mime_type": "text/plain", "count": 6}
    ],
    "items": [
      {
        "artifact_id": "art_...",
        "run_id": "run_...",
        "kind": "transcript_ref",
        "mime_type": "text/plain",
        "content_hash": "sha256:..."
      }
    ]
  },
  "provider_evidence": {
    "summary": {"total": 37},
    "groups": [
      {"kind": "agent.turn.provider", "subject_type": "run", "count": 6},
      {"kind": "coerce.provider", "subject_type": "run", "count": 6},
      {"kind": "skills.injected", "subject_type": "run", "count": 6},
      {"kind": "rule.committed", "subject_type": "rule_commit", "count": 19}
    ],
    "items": [
      {
        "evidence_id": "ev_...",
        "kind": "agent.turn.provider",
        "subject_type": "run",
        "subject_id": "run_...",
        "causation_id": "eff_...",
        "correlation_id": "key_...",
        "summary": "Fixture provider completed agent turn"
      }
    ]
  },
  "assertion_filter": {
    "include_tags": ["acceptance"],
    "exclude_tags": [],
    "total": 6,
    "selected": 6
  },
  "executable_spec": {
    "status": "passed",
    "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0},
    "tags": []
  },
  "assertions": []
}
```

`diagnostics` is the durable diagnostic list for the dev instance at the end of
the run. It uses the same object shape as `diagnostics <instance> --json`, so
provider boundary failures, policy denials, assertion failures, guard errors,
and terminal diagnostics can be inspected from the single dev report.

`provider_runs` summarizes durable provider runs by provider/status, including
artifact counts for each group. `provider_artifacts` summarizes artifact
metadata by kind and MIME type and includes compact artifact item links:
artifact id, run id, kind, MIME type, and content hash. It does not expose
artifact paths or content.
`provider_evidence` summarizes evidence metadata by kind and subject type and
includes compact evidence item links: evidence id, kind, subject type/id,
causation id, correlation id, and summary. It does not expose evidence metadata
payloads. Use the `runs`, `artifacts`, and `evidence` inspection commands when
full individual run ids, lifecycle details, artifact metadata, or evidence
metadata payloads are needed.

Assertion tag filters select which source assertions are evaluated and reported.
They do not skip rules, effects, providers, or table seeding. Exclusion wins
when both include and exclude filters match an assertion.

Assertion reports include `target_id`, `event_id`, `expr`, `reads`, `tags`,
`status`, `passed`, `actual`, `actual_values`, and `expected`. They may
include `description`, `diagnostic_ids`, `failure_reason`, `error`, and
`source_span`.
`event_id` links the report row to the durable `assertion.passed`,
`assertion.failed`, or `assertion.errored` event. Failed and errored
assertions also include `diagnostic_ids` for the diagnostic records produced
from that assertion result.

## `dev --stream ndjson`

`dev --stream ndjson` emits one compact JSON envelope per line while the local
dev loop runs. Each envelope uses:

```json
{
  "schema": "whipplescript.dev_stream.v0",
  "sequence": 0,
  "event": "dev.started",
  "data": {}
}
```

Current event names are `dev.started`, `dev.events`, `dev.step`, `dev.worker`,
`dev.idle`, `dev.assertions`, and `dev.report`. `dev.events` emits batches of
newly persisted raw runtime events using the same event object shape as
`whip log --json`; batches include `after_sequence`, `count`, and `events`.
`dev.assertions` emits the compact executable-spec assertion summary after
assertion events and diagnostics have been persisted. The final `dev.report`
event embeds the same
`whipplescript.dev_report.v0` object emitted by `dev --json` in its `data`
field. Stream mode is descriptive and does not change runtime semantics.

Each assertion read links the assertion to the deterministic projection it
checked:

```json
{
  "kind": "effect",
  "head": "kind agent.tell",
  "guard": "status == completed",
  "source": "effect:kind agent.tell where status == completed",
  "match_count": 6,
  "matches": [
    {
      "id": "eff_...",
      "name": "agent.tell",
      "status": "completed"
    }
  ]
}
```

`kind` is currently `fact` or `effect`. `source` is a stable source-like label
for grouping reports, while `head` and `guard` are structured fields for
machine consumers. `matches` links the read to concrete active facts or effects.
Fact matches include fact id, name, key, provenance class, and source span when
available. Effect matches include effect id, kind, status, and prompt content
type when the effect input declares `prompt_content_type`.

## Acceptance Reports

`whip --json accept <fixture.json>` emits one
`whipplescript.acceptance_report.v0` object:

```json
{
  "schema": "whipplescript.acceptance_report.v0",
  "fixture": "examples/provider-language-e2e.accept.json",
  "workflow": "examples/provider-language-e2e.whip",
  "passed": true,
  "failures": [],
  "observed": {
    "summary": {"facts": 18, "effects": 12},
    "facts": [{"name": "LanguageE2EResult", "count": 6}],
    "effects": [
      {"kind": "agent.tell", "status": "completed", "count": 6}
    ],
    "actions": [
      {"type": "pause", "event_id": "evt_...", "sequence": 2},
      {"type": "resume", "event_id": "evt_...", "sequence": 3}
    ],
    "source_metadata": {
      "summary": {"targets": 3},
      "targets": [
        {
          "key": "workflow:ProviderLanguageE2E",
          "target_kind": "workflow",
          "target": "ProviderLanguageE2E",
          "tags": ["fixture", "acceptance"],
          "description": "Fixture-backed provider x language acceptance workflow"
        }
      ]
    },
    "runs": {
      "summary": {"total": 12, "artifact_count": 12},
      "groups": [
        {"provider": "fixture", "status": "completed", "count": 12, "artifact_count": 12}
      ]
    },
    "artifacts": {
      "summary": {"total": 12},
      "groups": [
        {"kind": "stdout_ref", "mime_type": "text/plain", "count": 6},
        {"kind": "transcript_ref", "mime_type": "text/plain", "count": 6}
      ]
    },
    "evidence": {
      "summary": {"total": 37},
      "groups": [
        {"kind": "agent.turn.provider", "subject_type": "run", "count": 6},
        {"kind": "coerce.provider", "subject_type": "run", "count": 6},
        {"kind": "skills.injected", "subject_type": "run", "count": 6},
        {"kind": "rule.committed", "subject_type": "rule_commit", "count": 19}
      ]
    },
    "inbox": {
      "summary": {"total": 1},
      "groups": [
        {"status": "pending", "severity": "normal", "count": 1}
      ]
    },
    "trace": {
      "summary": {"events": 61, "abstract_events": 24},
      "groups": [
        {"type": "effect_created", "count": 12},
        {"type": "effect_terminal", "count": 12}
      ],
      "items": [
        {
          "sequence": 4,
          "event": {
            "type": "run_started",
            "run_id": "run_...",
            "effect_id": "eff_..."
          }
        }
      ],
      "conformance": {"ok": true}
    },
    "diagnostics_by_code": [],
    "assertion_reads": [
      {
        "kind": "effect",
        "head": "kind agent.tell",
        "source": "effect:kind agent.tell where status == completed",
        "match_count": 6,
        "matches": [
          {
            "name": "agent.tell",
            "status": "completed",
            "prompt_content_type": "markdown",
            "provenance_class": null,
            "trace_items": 24,
            "evidence_items": 12,
            "trace_sequences": [2, 4, 5],
            "evidence_ids": ["ev_..."],
            "count": 6
          }
        ]
      }
    ],
    "executable_spec": {
      "status": "passed",
      "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0},
      "tags": [
        {
          "tag": "acceptance",
          "status": "passed",
          "summary": {"total": 6, "passed": 6, "failed": 0, "error": 0}
        }
      ]
    }
  },
  "dev_report": {"schema": "whipplescript.dev_report.v0"}
}
```

`observed.summary` reports total final active facts and effects. The grouped
`observed.facts` and `observed.effects` arrays summarize final store state by
fact class and effect kind/status. `observed.runs` summarizes provider run
counts and artifact counts by provider/status. `observed.artifacts` summarizes
artifact metadata by kind and MIME type and includes compact artifact item
links without exposing paths or content.
`observed.evidence` summarizes evidence metadata by kind and subject type and
includes the same compact evidence item links as `provider_evidence`, without
exposing evidence metadata payloads. `observed.inbox` summarizes human inbox
items by status and severity. `observed.actions` records fixture control-plane
setup actions that were applied to the started instance. `observed.source_metadata`
summarizes source metadata targets from the embedded dev report.
`observed.assertion_reads` summarizes deterministic assertion reads and concrete
match metadata such as prompt content type. Effect match groups also include
compact `trace_items` and `evidence_items` counts plus `trace_sequences` and
`evidence_ids` links for matched effects. These identifiers link to
`observed.trace.items` and `observed.evidence.items` without embedding raw
payloads in the assertion read summary.
`observed.trace` summarizes raw event count, reconstructed abstract trace event
groups, compact abstract trace items, and trace conformance without embedding
the full raw store event log. Its conformance check includes raw per-instance
event sequence gaps before abstract lifecycle records are checked.
`observed.diagnostics_by_code` and `observed.executable_spec` give compact
summaries of the embedded dev report for fixture failure messages and CI
dashboards. This is acceptance-report metadata only; the embedded `dev_report`
remains the ordinary dev-loop report contract.

Acceptance fixtures validate selected parts of that final report with
`expect.dev_status`, `expect.status`, `expect.source_metadata`,
`expect.diagnostics`, `expect.diagnostics_by_code`, `expect.actions`,
`expect.assertions`, `expect.assertion_tags`, `expect.assertion_untagged`,
`expect.assertion_reads`, `expect.summary`, `expect.facts`, `expect.effects`,
`expect.runs`, `expect.artifacts`, `expect.evidence`, `expect.inbox`, and
`expect.trace`. Fixture `input` is ordinary workflow start input and uses the
same validation path as `run` and `dev`. Fixture `actions` are existing
control-plane transitions applied before the dev loop. Fixture and expectation
shapes are validated before starting a workflow; wrong-typed expectation fields
are rejected instead of being treated as absent. `expect.assertion_reads`
entries must include at least one selector: `source`, `kind`, `head`, or
`guard`.

`accept` is deliberately single-fixture in v0. Test suites should invoke the
command once per fixture with isolated stores until suite-level runtime id
namespacing is specified.

## Table Fact Provenance

Facts seeded from `table` declarations use:

```json
{
  "provenance_class": "table",
  "source_span": {
    "path": "examples/provider-language-e2e.whip",
    "start": 2132,
    "end": 2363,
    "construct": "table_row"
  }
}
```

The span points to the source row. Table provenance is report metadata; the
runtime still commits ordinary durable facts.

## Stability And Open Work

Stable enough for current v0 tests:

- source metadata shape for `check`, `compile`, and `dev`
- report schema/version identifiers for `check`, `compile`, `dev`, `trace`, and
  `accept`
- draft JSON Schema files for `check`, `compile`, `dev`, `trace`,
  `dev --stream`, and acceptance reports/fixtures
- generated provider-language reports validate against those schema files
- NDJSON stream envelopes for `dev --stream ndjson`, including raw `dev.events`
  runtime event batches
- acceptance report pass/fail, mismatch failures, embedded dev report, observed
  fact/effect count summaries, observed provider run/artifact counts, observed
  evidence counts, observed diagnostic counts, and observed executable-spec
  summaries
- acceptance suite isolation policy: v0 accepts one fixture path, with suite
  runners responsible for isolated stores
- dev report diagnostics for provider, policy, assertion, guard, and terminal
  failures
- assertion filter report counts
- `dev` assertion and `executable_spec` summaries
- assertion deterministic read links with concrete fact/effect matches and
  grouped trace/evidence link counts plus compact link arrays
- assertion links to durable assertion events and failure diagnostics
- table fact provenance and row source spans

Still open:

- broader live event streaming beyond `dev` is deferred to observability work
  outside this Gherkin lessons tracker
- richer evidence and artifact links for provider and policy failures can
  continue under provider observability work
