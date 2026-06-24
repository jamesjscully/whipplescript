# Decision Records

Status: draft workshop index

This directory holds product and language decision records before they become
implementation work. Some records are already completed historical decisions;
others are active workshop material.

## Active Workshop Records

- [standard-package-design-tracker.md](standard-package-design-tracker.md):
  active todo tracker for the package-by-package design review before concrete
  package design or implementation work.
- [0001-whipplescript-standard-packages.md](0001-whipplescript-standard-packages.md):
  WhippleScript is the orchestration product boundary; durable work tracking
  becomes an optional standard package.
- [0002-work-tracker-package.md](0002-work-tracker-package.md): the durable
  work record is the issue tracker, with ready-work views as resource-qualified
  construct-graph projections.
- [0004-provider-compatibility.md](0004-provider-compatibility.md): GitHub,
  Linear, Jira, and local providers share a portable semantic subset plus
  capability discovery, not a forced identical model.
- [0006-libraries-packages-providers-and-exec.md](0006-libraries-packages-providers-and-exec.md):
  libraries are source-level reuse, packages install libraries/providers,
  hosted `exec` is the default custom provider path, and "plugin" becomes an
  implementation term.
- [0007-core-standard-libraries-and-providers.md](0007-core-standard-libraries-and-providers.md):
  current language functionality classified into kernel, standard libraries,
  provider implementations, and construct/lowering-class roles.
- [0008-memory-package.md](0008-memory-package.md): `std.memory` owns named
  memory pools, explicit `recall from` and `learn from ... into` operations,
  turn-scoped `with access to` grants, and policy-driven curation.
- [0009-agent-package.md](0009-agent-package.md): agent declarations stay core,
  while provider bindings, profile presets, skill resolution, and provider
  capability discovery belong in `std.agent` metadata/provider catalogs.
- [0010-package-library-provider-boundary.md](0010-package-library-provider-boundary.md):
  formalizes the kernel/library/provider/package boundary, separating source
  imports from runtime authority and provider execution.
- [0011-controlled-library-grammar-extensions.md](0011-controlled-library-grammar-extensions.md):
  defines the constrained construct-graph/lowering-class extension system for
  libraries, with static acceptance contracts and deterministic lowering into
  core IR.
- [0012-plugin-system-retirement-cleanup.md](0012-plugin-system-retirement-cleanup.md):
  tracks the cleanup from a separate public plugin system toward package,
  library, provider, construct-graph, and runtime-provider-registry terminology.
- [0013-coordination-package.md](0013-coordination-package.md): `std.coord`
  owns generic leases, ledgers, and counters as a closed, privileged standard
  package with append-only events and projection-derived state.
- [0014-schema-coercion-package.md](0014-schema-coercion-package.md):
  core owns `coerce`/`decide` typed schema-coercion semantics, while
  `std.coercion` owns backend/toolchain integration and coerce is one concrete
  backend.
- [0015-agent-harness-feature-semantics.md](0015-agent-harness-feature-semantics.md):
  `std.agent` standardizes the provider boundary and feature-report taxonomy,
  while Codex, Claude, and Pi live in separate provider packages with native
  semantics and truthful capability maps.
- [0016-codex-agent-provider-package.md](0016-codex-agent-provider-package.md):
  `std.agent.codex` maps the shared agent-provider contract onto Codex App
  Server/SDK surfaces, Codex slash-command features, plugins, hooks, skills,
  subagents, and redacted Codex evidence.
- [0017-claude-agent-provider-package.md](0017-claude-agent-provider-package.md):
  `std.agent.claude` maps the shared agent-provider contract onto the Claude
  Agent SDK sidecar, Claude tool/permission policy, skills, plugins, hooks,
  subagents, sessions, and redacted Claude evidence.
- [0018-pi-agent-provider-package.md](0018-pi-agent-provider-package.md):
  `std.agent.pi` maps the shared agent-provider contract onto Pi RPC and
  `pi_variant` package/extension sets with command/tool discovery and redacted
  Pi evidence.
- [0019-files-package.md](0019-files-package.md): `std.files` owns
  capability-scoped file stores, deterministic file/document codecs,
  read/write/import/export effects, and turn-scoped agent file grants.
- [0020-blocked-effect-binding-taxonomy.md](0020-blocked-effect-binding-taxonomy.md):
  provider-binding failures block (recoverable) instead of fail, and every blocked
  effect carries one categorized `policy_block_reason` spanning scheduling- and
  binding-time origins. *Implemented (v0).*
- [0021-package-projection-noun-vocabulary.md](0021-package-projection-noun-vocabulary.md):
  multi-word `expect` nouns (`message sent to ops`, `file R at P`) need a
  package-declared projection-noun vocabulary + slot-aware parsing; most target nouns
  are blocked on unimplemented package projections. *Proposed design — recommends
  deferral; dotted-name projections already cover what exists.*
- [0022-collection-valued-projections.md](0022-collection-valued-projections.md):
  introduce a collection-valued projection (`<Schema> [where <pred>]` →
  `Array<Ref<Schema>>`) as the `std.files` `export` row source — a real, general
  collection value built "the right way" but exposed only in the `export { rows … }`
  clause in v0. *Accepted; foundation for export (#6).*
- [0023-action-block-rule-templates.md](0023-action-block-rule-templates.md):
  introduce a top-level `action <name>(<params>) { … }` declaration — a static,
  hygienic, inline-expanded template over rule-body effect chains (`tell → coerce →
  record`), distinct from `pattern`/`apply` (which generate top-level declarations).
  *Accepted (design); fills the last copy-paste ergonomic gap (final-audit G-010).*

## Historical Decision Trackers

- [language-ergonomics-tracker.md](language-ergonomics-tracker.md): v2 language
  surface decisions and implementation status.
