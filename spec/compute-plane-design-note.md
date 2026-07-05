# The Compute Plane — Design Note (DO-host sidecar tier + `whip deploy`)

**Status: DESIGN NOTE (pre-ADR).** The design pass behind the DO tracker's
Phase 8 (registered 2026-07-03 as open intent; designed 2026-07-04, four
forks settled by Jack). The pure-DO host solves the orchestrator + storage
plane; this note designs the **compute plane** — where exec/agent compute
that cannot live in the isolate actually runs (DR-0033 Decision 7:
subprocess effects are HTTP to a container sidecar) — plus the `whip
deploy` surface sketch. Shared design with
`versioned-workspace-research-note.md` (materialization + evidence-grade
boundary sections); the tracker's Phase 8 holds the open build work and
defers here for design.

## 1. Platform ground truth (verified 2026-07-04)

Cloudflare Containers: each container instance is paired 1:1 with a
controlling DO (the container class); instance types lite → basic →
standard, with concurrency ceilings raised 15× in Feb 2026 (≈15,000 lite /
6,000 basic / ~1,500 standard-1 per account); cold starts 1–3s,
image-size-dependent (small images sub-second); billed while running;
**built-in autoscaling still unshipped** — the stateless pattern is
`getRandom` over a *fixed-size* pool, location-blind; deploys are rolling.
DO SQLite storage billing live since Jan 2026. (Sources: Cloudflare
Containers docs — pricing, architecture/lifecycle, limits, scaling-and-
routing, stateless examples; changelog 2026-02-25.)

## 2. The load taxonomy — two service classes, from the evidence-grade boundary

The exec taxonomy the versioned workspace established for evidence
purposes turns out to be the *economic* taxonomy too:

- **Class A — deterministic delta kernels**: validators, builds, tests,
  and **derived-gauge judges** (which run at ambient-scoring frequency —
  the hot path, and new load since Phase 8 was registered). Small, short,
  hermetic, frequent.
- **Class B — stochastic sidecar turns**: coding agents and anything
  long-running in a materialized worktree. Big, long, stateful within the
  turn.

**The Class-A result cache is the biggest economic lever in the plane.**
Delta-kernel identity = script hash + environment hash + input hashes, so
results are memoizable in the content-addressed store, **workspace-wide**
(settled 2026-07-04): a derived-gauge judge scoring the same output twice
never runs twice; a validator on untouched inputs is a cache hit; frozen-
prefix replay was already served-from-record. Safe by construction —
hermeticity (empirically audited, fail-closed demotion) plus content
addressing make cross-branch reuse sound; labels ride the cached result
like any derived data. No new machinery: the effect ledger's
idempotency-key discipline extended with content keys. Expectation: the
majority of Class-A invocations resolve without a container waking.

## 3. Topology (settled 2026-07-04): who owns which containers

The originally-registered "container-per-workflow-instance-DO, 1:1" is
**revised** — it is right for Class B and wrong as a blanket:

- **Class A: a workspace-level warm pool**, owned by the **workspace DO**
  — N small (lite/basic) stateless executor instances, `getRandom`-routed,
  shared safely *because* hermeticity is already demanded and audited.
  Pool size is a manual knob until platform autoscaling ships (progressive
  rigor: the default size just works for small workspaces).
- **Class B: container-per-turn (or per-branch, reused across a session's
  turns)**, the 1:1 controller pattern where it belongs — the container
  holds the materialized branch scratch dir as its snapshot-isolated
  working set; lifecycle owned by its controlling DO; cold start amortizes
  over a multi-minute turn; billed-while-running is inherent (the agent is
  genuinely running). Materialization respects the instance type's disk
  bound via **manifest-subset materialization** (slicer-computed input
  closures, fetch-on-demand, clear failure at the bound — versioned-
  workspace note, "All files, including large ones").
- **The workflow-instance DO owns no containers** — it emits effects.

## 4. Protocol

The sans-IO discipline meeting the settled materialization protocol —
nothing architecturally new. The `InstanceStepMachine` raises
`NeedsIo(Http)`; the TS shell fetches into the container. Request carries:
effect id, **branch marker** (posture rides the protocol), manifest ref,
command, environment digest. The container **pulls only missing blobs**
from the object tier, materializes, runs, **pushes the diff back keyed by
effect id** — atomic, recorded, complete; idempotent under at-least-once
retry (DR-0033 Decisions 3/4).

- **Class A**: one request-response; batching allowed — one
  materialize-request may carry several pending small execs against the
  same manifest (natural under the step machine's serial effect dispatch).
- **Class B: hibernatable WebSocket from day one** (settled 2026-07-04,
  Jack — choosing WS over a polling-first sequencing). This *is* the case
  DR-0033 Decision 6 reserved WS for (live progress/backpressure from a
  long external data-plane job to a sleeping DO); its build-when-needed
  gate is now satisfied by decision. Turn progress streams up; the DO
  hibernates between messages; the diff-back on completion remains the
  same idempotent import.

## 5. Identity — image digest = environment hash

v1: **one workspace image**, declared in workspace config; its digest
folds into generator-hash ambient config (experimentation note, "Evidence
identity — the slice hash"), so a toolchain bump is a visible warm-start
epoch and a rolling redeploy is *legible as* an environment change, never
silent drift. Per-site images are a later refinement (conflicting
toolchains), not v1.

## 6. IFC span, postures, and priority

- **Egress default-deny + allowlists** derived from the same declarations
  that feed `WHIPPLESCRIPT_EXEC_ALLOW` — the sidecar is *stronger* than
  native exec (the versioned-workspace door table's recorded asymmetry:
  sidecar network residual contained; native's declared). Enforced where
  whip cannot see syscalls; designed deliberately, not inherited.
- **Postures ride the protocol**: counterfactual execs are
  live-within-materialization with the network already denied by default;
  containment needs nothing added here. Secrets arrive scoped (P6): only
  what the site's declaration grants.
- **Priority classes implement the versioned-workspace scheduling
  residual**: the pool serves production > working > counterfactual, so
  mass regeneration cannot starve live traffic of executors — that
  parked open item lands here as a concrete queue discipline.

## 7. Economics, assembled (descending leverage)

1. The **delta-kernel result cache** (§2) — most Class-A work never
   executes.
2. The **warm pool** — a few lite instances absorb the residual; manual
   sizing until autoscaling ships.
3. **Batching** (§4).
4. **Placement** — `getRandom` is location-blind today; accepted, revisit
   when resource-aware routing ships.

Class B has no trick: agents cost what thinking costs, metered by the
existing spend machinery.

## 8. `whip deploy` — surface sketch only

One command, zero-config correct (progressive rigor): build the wasm
kernel, build/push the workspace image, provision DO namespaces +
object-tier bucket + pool, wire secrets — wrangler underneath, never
surfaced (the git critique's "non-technical users don't know the verbs"
doctrine applies to deployment verbs too). Deploy unit = the workspace.
Environments (staging/prod) and multi-region: later.

## 9. Non-goals

GPU; cross-workspace anything; a long-lived service construct (whip has no
such effect kind — if one ever exists it is a new door discussion, not a
compute-plane footnote).

## 10. Settled vs. open

**Settled in principle (Jack, 2026-07-04):** the two service classes with
the workspace-wide Class-A result cache; pool ownership at the workspace
DO (revising the registered 1:1 blanket); Class-B transport = hibernatable
WebSocket from day one (satisfying DR-0033 Decision 6's reserved case);
one-workspace-image identity; default-deny IFC span; priority classes;
dedicated-note placement (this document).

**Open (build-time / ADR):** pool default size + priority-queue details;
batching heuristics; image build pipeline inside `whip deploy` (base image
contents, layer strategy for declared toolchains); WS protocol framing for
Class-B progress; cache eviction/GC for delta-kernel results (joins the
versioned-workspace retention question); per-site images; environments;
placement revisit when platform autoscaling ships.

## 11. Relationships

- **DO tracker Phase 8** — holds the open build work; this note is its
  design.
- **DR-0033** — Decisions 3/4/6/7 are load-bearing here; a build-time DR
  should formalize the sidecar protocol (candidate DR-0034).
- **Versioned workspace** — materialization + evidence-grade boundaries
  (the protocol's atomicity/hermeticity obligations); scheduling residual
  lands in §6 priority classes; cache GC joins its retention question.
- **Experimentation subsystem** — image digest in generator-hash ambient
  config; derived-gauge judges as Class-A hot load.
- **Open-core seam** — the platform object store / hosted deploy remain
  the enterprise-tier deliverables already noted in Phase 7; the protocol
  and pool logic are core-shaped.
