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
  while Codex and Claude live in separate provider packages with native
  semantics and truthful capability maps.
- [0016-codex-agent-provider-package.md](0016-codex-agent-provider-package.md):
  `std.agent.codex` maps the shared agent-provider contract onto Codex App
  Server/SDK surfaces, Codex slash-command features, plugins, hooks, skills,
  subagents, and redacted Codex evidence.
- [0017-claude-agent-provider-package.md](0017-claude-agent-provider-package.md):
  `std.agent.claude` maps the shared agent-provider contract onto the Claude
  Agent SDK sidecar, Claude tool/permission policy, skills, plugins, hooks,
  subagents, sessions, and redacted Claude evidence.
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
- [0024-owned-brokered-agent-harness.md](0024-owned-brokered-agent-harness.md):
  introduce an **owned (brokered)** harness mode alongside the existing
  *delegating* one — whip executes every tool the model requests (I1), a turn
  stays one leaf node (I2), and the loop never leaks control flow upward (I3) —
  so that `lease`/`counter`/`file store`/`capability` become an *enforced*
  envelope on agent execution instead of advisory metadata. *Accepted (founding
  premise + invariants); durability granularity, governance map, sandbox, and
  lifecycle left open as steps 2–5.*
- [0025-workflows-as-agent-tools.md](0025-workflows-as-agent-tools.md):
  expose curated workflows as typed agent tools (the `workflow.invoke` capability
  facade, synchronous), held to a **convergence invariant** — acyclic invoke-tool
  graph + self-terminating, signal-free sub-workflows — so the whole agent tree
  provably converges with non-termination confined to the root `@service` loop.
  Amends DR-0024 slice 2: the workspace lease becomes re-entrant within an invoke
  subtree. *Accepted (design); formal convergence model gates implementation.*
- [0026-session-root-agent.md](0026-session-root-agent.md):
  run the harness as the top-level interface — a **session root** agent that is
  itself a root whip (one harness, DR-0024 reused) and the only loop allowed to
  author and spawn arbitrary (individually-bounded) whips. Bounded by a signed
  **policy envelope** it cannot self-widen; escalation is reactive over
  `human.ask` with kernel enforcement; governance authoring runs on a **separate
  sudo-gated agent** (DR-0028 D5). Observed via a versioned, cursor-tailed
  **session-event stream** over
  the existing durable log (protocol, not TUI). Relocates DR-0024 I3's
  no-self-escalation to the envelope; keeps spawn-and-observe distinct from
  `workflow.invoke`. *Accepted (design); the information-flow lattice is a
  separate research thread it does not depend on.*
- [0027-information-flow-control.md](0027-information-flow-control.md): make
  prompt injection and data exfiltration **provable** at compile time via a
  party-relative information-flow label system — confidentiality "who may read"
  and integrity "who vouches", principals ordered by acts-for, the agent turn an
  opaque **join box** enforced at boundaries, and downgrading
  (declassify/endorse) explicit, authority-scoped, axis-locked, and **NMIF**-safe
  with the audit set as the trusted surface. Labels are total and fail-closed,
  survive every durable and cross-instance boundary, and cover the whole brokered
  tool surface; the guarantee is policy-relative non-interference over
  explicit/implicit flows, side channels out of scope. Verification splits across
  Maude (compiler surface), TLA+ (durable/temporal invariants), and a proof
  assistant scoped to deltas over the published algebra. *Accepted (8 invariants);
  syntax, label algebra, NMIF checking, construct grounding, and slices left open;
  exploratory Maude models gate it.*
- [0028-information-flow-authority.md](0028-information-flow-authority.md): policy
  is two tiers — a **locked governance envelope** holding authority (role
  hierarchy, delegation context, ownership, protected-from, downgrade rights) plus
  **inline usage** proven to refine it (`inline ⊑ envelope`). The agent **acts for
  its user** so **trust required equals authority delegated** — an IT-owner
  envelope is guaranteed-safe with no trust over protected data, an
  agent-drafted-then-ratified envelope is trust-but-verify over the user's own
  data. Governance and whip authoring split across **two root agents separated by
  OS privilege** (D5): a sudo-gated governance agent that alone signs policy, and
  an unprivileged whip agent. Envelope changes are versioned and non-retroactive —
  they never authorize past flows, and in-flight
  work is bound to its version. *Accepted (authority model); the governance half
  of DR-0027; extends DR-0026's envelope.*
- [0029-cross-package-information-flow.md](0029-cross-package-information-flow.md):
  a package that exports a `@tool` carries an attested **information-flow surface**
  (X1–X8) in its contract; the check is two-sided — the producer proves its
  internals stay within the declared surface, the consumer proves
  `surface ⊑ envelope`. *Accepted; IFC lifted to the package boundary.*
- [0030-refining-the-join-box.md](0030-refining-the-join-box.md): how to refine the
  opaque join box **on the label axis, never the quantity axis** (entropy/QIF
  rejected — bits are the wrong unit). Three directions: **(A)** per-tool flow
  signatures as a producer-attested structural dependency matrix (decided +
  modeled, `infoflow-signature.maude`), **(B)** phase/provenance-typed turn outputs
  ("model fills, whip assembles" — zero new syntax), **(C)** semantic checked
  declassifiers (NMIF-on-selector soundness, query-budget backstop). *Accepted
  direction; A modeled, B/C staged; refines DR-0027/0029.*
- [0034-managed-vs-delegated-harnesses.md](0034-managed-vs-delegated-harnesses.md):
  the flat harness/provider enum conflates two categorically different runtimes;
  split it into a **`HarnessClass`** — **Managed** (owned; WhippleScript is the
  runtime, hermetic context + full provenance) vs **Delegated** (codex/claude; a
  foreign runtime that assembles its own context). Context-assembly knobs are
  Managed-only, delegation knobs (un-crippled `setting_sources`) Delegated-only; the
  evidence model forks (full provenance vs a `context: provider-assembled`
  attestation); authority stays WhippleScript's for both. *Accepted; reframes/supersedes
  context-assembly Phase 6; absorbs the candidate sidecar-protocol DR; retrofits DR-0024.*
- [0035-delegated-harness-wire-protocol.md](0035-delegated-harness-wire-protocol.md):
  the D8-6 follow-on under DR-0034 — the delegated turn contract is formalized as
  **obligations over the existing dialects** (Claude whip-sidecar JSONL, Codex
  app-server JSON-RPC), not one unified wire format: canonical turn
  envelope with narrowing-only policy projections, exactly-one-terminal +
  tolerant run-id routing, two-clock liveness (inactivity wall clock + delivered-
  frame budget), ack-then-terminal cancellation wired into the driver, declared
  re-query before `uncertain` resolution, version exchange, kernel-owned
  shape-only redaction. *Accepted (obligations-over-dialects ratified); build
  items B1–B6 un-gated.*
- [0036-per-turn-workspace-cut-and-dynamic-guarantees.md](0036-per-turn-workspace-cut-and-dynamic-guarantees.md):
  the receipt's `workspace_cut_ref` gets populated (the runtime's labeled,
  reference-only claim of the turn's workspace delta; honest decline for
  unmediated delegated harnesses) and the guarantee report gains a **dynamic
  section** — named per-turn guarantees (`writes_within:<scope>`,
  `no_reads_beyond_grant`, `no_tainted_reads:<class>`) evaluated under the
  cited policy epoch, so hosts match guarantee names instead of re-deriving
  semantics. Drafted as the cross-repo dependency of GaugeWright ADR 0082
  (advancement policy). *Proposed.*
- [0037-gauge-campaign-improve-surface.md](0037-gauge-campaign-improve-surface.md):
  the experimentation surface's v1 build — `gauge` + `campaign` as hand-parsed
  core declarations (`judge via coerce|prompt|exec|labels`, chance/stat bars in
  `at least`/`at most` word form, derived gauges via `inputs`), the sibling
  improve store (append-only evidence rows, pinned scenarios, cumulative
  scenario wear, event-sourced campaign records), and the `whip improve` loop:
  naming-is-the-partition, dominance-invariant acceptance
  (`improve-acceptance.maude`), holdout sealing with wear-out and honest
  `unheld-out` degradation (`improve-holdout.maude`), holdout-blind reflective
  proposer, propose-don't-apply with baseline-hash-guarded `whip adopt`.
  *Accepted + built.*
- [0038-mark-pins-and-prefix-replay.md](0038-mark-pins-and-prefix-replay.md):
  the checkpoint-substrate integration — `mark "<name>" after <site>` cut
  points stamped at rule commit, `whip pin ... at <mark>` prefix-pinned
  scenarios, the clone-and-truncate replay driver (replayed prefixes fire
  nothing, quiescence-at-cut refusal, revision activation for candidates,
  epoch-bump refires tagged), `whip suppose` as the what-if verb, and
  campaign evaluation paired at the cut. Witness model
  `prefix-replay.maude`. *Accepted + built.*
- [0039-bashkit-default-bash.md](0039-bashkit-default-bash.md): Bashkit becomes
  WhippleScript's default governed `bash` implementation on native and Durable
  Object placements; non-bash external capabilities remain typed brokered
  effects. *Accepted; implementation in progress.*
- [0042-secret-free-model-egress-broker.md](0042-secret-free-model-egress-broker.md):
  hosted provider bindings explicitly select a transitional Worker secret or a
  secret-free authenticated model broker; the broker receives WhippleScript's
  admitted request with auth headers stripped and may only inject credentials +
  perform transport. *Accepted; authenticated HTTP realization implemented.*

## Historical Decision Trackers

- [language-ergonomics-tracker.md](language-ergonomics-tracker.md): v2 language
  surface decisions and implementation status.
