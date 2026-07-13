# Whipplescript as un-tie's Substrate — Research Note (replacing Pi and git)

> **Update 2026-07-05:** the un-tie half is ratified and moving. The monorepo
> split completed — the gaugewright-side home is now **`~/code/gaugedesk`**
> (with `gaugewright-cloud`/`-directory` consuming it via a pinned `platform/`
> submodule); `~/code/un-tie` is the archived pre-split history. The decision
> below is recorded there as **workbench ADR 0071** (renumbered from un-tie's
> 0070 — workbench's 0070 was taken), with migration rows `SUB-1..5` in its
> tracker. **SUB-0 (seam finishing) is DONE 2026-07-05** in workbench: neutral
> `Workspace`/`ChatWorkspace`/`WorkspaceProvider` traits over git, the
> `gaugewright-harness` seam crate (`Harness`/`HarnessSpec`/`HarnessFactory`,
> sandbox, neutral content types), `AgentDefinition`, interrupt handles — so
> §6 step 2's shell surface and step 3's adapter seam now exist to build
> against; the whip impls are "only a new impl" plus the parked decisions
> named in the workbench tracker.

**Status: RESEARCH NOTE (pre-ADR).** Long-term goal set by Jack 2026-07-04:
whipplescript **completely replaces Pi and git** in the un-tie app
(repo `~/code/un-tie`; product/crate namespace **gaugewright**). Big,
invasive, deliberate. This note records the architecture analysis (three
repo sweeps, 2026-07-04), the settled framing and decisions, and the
dependency-ordered plan. Cross-repo: un-tie file paths below refer to that
repo; whip capabilities refer to this corpus's research notes. **Whip-side
build intent is tracked: `untie-substrate-readiness-tracker.md`
(registered 2026-07-04)** — the plan's steps as they apply to this repo,
and the canonical build home for the versioned workspace.

## 1. Why the ground is prepared

Three deliberate un-tie facts make the replacement tractable — the seams
were cut for it:

1. **Pi is already abstract.** Pi runs as an out-of-process subprocess
   behind the `Harness` trait (`crates/pi-bridge/src/lib.rs`:
   `run_turn(gate, prompt, images, sink) → TurnOutcome`); ADR 0031 states
   the swap contract ("a new runtime is *only* a new `impl Harness` — no
   admission-shell change") and three non-Pi impls already exist
   (scripted, remote, loopback). The one named residue is the Pi-native
   agent-definition surface (`.pi/**`, `AGENTS.md`) — flagged there as "a
   follow-on, not a rearchitecture."
2. **Git is already demoted.** The event log is authority; git only moves
   bytes. The pure merge reducer (`crates/core/src/merge.rs`) consumes
   only `GitClean`/`GitConflict` and would run unchanged over any
   substrate; every git call funnels through one shell-out crate
   (`crates/workspace`).
3. **Capabilities are harness-mapped by decision** (ADR 0033): neutral
   intent, realized per-harness — a whip realization is sanctioned.

## 2. The authority split (SETTLED, Jack 2026-07-04)

**Gaugewright is the policy authority; whipplescript is the machinery
owner and consumes policy.** Gaugewright's admin surface sets policy for
the entire federated system, orgs, and projects; its log records
*decisions* (who may, what's allowed, consent, entitlements, releases).
Whip owns everything it owns today — runtime, versioning, IFC mechanism,
evidence — and its log records *happenings* (what ran, what changed, what
evidence accrued). Gaugewright also owns all governance outside whip's
scope (federation, handoff, accounts/orgs, review/export lifecycles,
packages-as-product, entitlements, erasure policy).

**Policy consumption = policy epochs.** Gaugewright's ADR 0004 pattern
("the admission shell materializes authority as data before the pure
reducer runs") extends across the seam: gaugewright compiles admin/org/
project policy into **versioned policy snapshots** (capability grants,
provider allowlists, egress policy, label clearances, resource scopes);
whip consumes a snapshot as ambient config, enforces mechanically, and
the guarantee report cites **which policy epoch it enforced**. A policy
change is a visible epoch bump inside whip's identity machinery, exactly
like a provider-profile bump.

**Auth relocates accordingly (settled direction):** credential
acquisition, OAuth flows, and storage live in gaugewright (which already
owns accounts, devices, enrollment); whip receives resolved credentials
inside provider profiles through the policy channel. Whip's own auth
surface *shrinks* to a thin standalone-mode resolver (env/keychain) for
whip-without-gaugewright — a simplification of the current coerce-auth
design, not an addition.

## 3. Replacing git — the versioned workspace, almost verbatim

Un-tie consumes 13 git capabilities, all via `crates/workspace`: repo
init; per-chat worktrees; refs (`engagement/<id>`, `workstream/<id>/main`,
`main`); commit-per-turn-finalize; merge-probe (`merge-tree`) / merge /
abort; diff-for-review; sync-from-main; revert; workstream promotion;
fork-with-shared-ancestry; bundle transport for handoff; status/hash
plumbing. The mapping onto the versioned workspace
(`versioned-workspace-research-note.md`) is nearly one-to-one: worktrees →
branches + virtual working sets (materialize-on-exec provides the agent's
cwd); refs → branches + the workstream tier (imported *from* un-tie —
convergent by construction); turn-finalize commits → cuts at quiescence
points; probe/merge/abort → merge-engine verdicts (their
`PARTIAL_MERGE_NOT_STANDING` holds by construction — proposed cuts, never
mutate-then-abort); revert → restore; fork → branching with lineage. The
pure merge reducer is untouched: the new shell feeds it the same
outcome enums.

**Where whip is stronger than what it replaces:**
- **Erasure.** Un-tie's documented tension (ADR 0008/0018: tombstoned
  handles over immutable git objects whose bytes still travel in handoff
  bundles) is *solved*, not worked around: whip owns its store, so
  per-blob crypto-erasure/GC with retained hashes — the scope-semantics
  fork-4 honesty-downgrade pattern — removes payload bytes while
  preserving replay of handles+metadata. Their `content-erasure.qnt`
  invariants become dischargeable over the substrate itself.
- **Workstreams and instance forking are native**, not layered.
- **No large-file cliff** (added 2026-07-05): git's large-file story is
  LFS — a bolt-on second system with its own server and failure modes.
  Whip's tiering + content-defined chunking are native
  (versioned-workspace note, "All files, including large ones"), so big
  context resources (datasets, PDFs, video) version like everything else,
  on both desktop and cloud.
- **Certified merge** — with an honest caveat: un-tie content is mostly
  *not* whip source (working files, documents, definitions), so day one
  the merge engine delivers git-parity-plus-provenance on blobs; the
  slice-certificate power arrives as content becomes whip-legible, which
  the Pi replacement drives (§4, archetype-as-package).
- **The interactive surface is designed by census, not vibe** (added
  2026-07-05, answering the "git has a bazillion escape hatches" concern):
  git's operation surface was enumerated and classified
  (versioned-workspace note, "Selective operations — the interactive VCS
  surface"). Compensations dissolve structurally (stash / index / reflog /
  rebase-squash-amend — the record–narrative separation); the surviving
  payload is a **provenance-native selection algebra** with three verbs
  (`undo <selection>` with dependency-closure stranding checks,
  identity-preserving `transport`, `adopt --only`), structured conflict
  objects with content-addressed resolution memory, provenance
  archaeology (blame-superseding; mostly pre-answered bisect), and a
  **no-destructive-verbs** surface whose complete hatch list is: manual
  state authoring, export bundles, the git bridge.

**New requirement:** a **bundle-equivalent workspace export** for handoff
(`STATE_BEFORE_HOME` ships full content) — manifest + reachable blobs,
cleaner than `git bundle`, and it must respect erasure (bundles are where
tombstoned bytes currently escape).

**Prerequisite reality:** all of this rides the versioned-workspace
*build* — branches, virtual working set, materialization, merge engine
v1, reconciliation daemon. Designed; zero built.

## 4. Replacing Pi — whip becomes the runtime; protection moves into the language

**Shape:** a `WhipHarness` impl of the existing `Harness` trait; a **chat
becomes a long-lived whip instance** (the persistent thread = instance
event log + restorable context; chat-fork = instance fork, native and
content-addressed); a turn = an instance turn; abort = cancel; Pi's
`extension_ui_request`/pending-approvals map onto human.ask/pending-asks
(exists); provider/model selection → provider profiles.

**The enforcement headline.** Un-tie's boundary today is three mechanisms
caging a third-party process from outside: an OS sandbox (bubblewrap/
Seatbelt) wrapping Pi, a TS membrane plugin on Pi's tool-call hook, and a
Rust egress gate. With the owned harness, **every tool call is a whip
effect through the membrane by construction** — the encapsulation
non-interference theorem, the egress doors, and the static checker
deliver as language semantics what the triple approximates as perimeter.
`INV-24` (method-write-requires-edit) becomes branch/package permissions
instead of a kernel `read_only_root`. Their **conservative
engagement-scoped taint** (forced by the `LAUNDER` probe because the
boundary cannot see inside Pi) upgrades to whip's certified per-field
flow signatures: precise taint, fewer forced reviews, same soundness —
gaugewright's review lifecycle keeps its role and consumes better labels.

**The definition surface is the deep move: an archetype becomes a whip
package.** Method bundle (prompt/skills/policy, versioned, published,
fork-protected IP) → whip package (pinned source, `ifc_surface` + state
surface, capability registry, versions); their package invariants
(`INSTALL_DOES_NOT_GRANT_PAYLOAD/RUN`) rhyme with whip's package
contract. This is also what unlocks certified merge in un-tie's
edit-chats.

**Gap resolutions (SETTLED, Jack 2026-07-04):**

1. **Tool-loop maturity → port Pi's behavior directly.** Everything
   naturally harness-owned in the whip context is copied from Pi (pinned
   0.73.1, open source — a behavioral-conformance port): tool ergonomics
   (read/edit/bash/fetch, truncation, result digests), turn lifecycle and
   abort semantics, system-prompt assembly, thread-continuation
   semantics, stderr/error surfacing. Explicitly *not* ported (they map
   to whip constructs instead): extensions → capabilities/actions, skills
   and prompt templates → package content, sandboxing → owned-harness
   IFC, session storage → instance event log + branches. Build artifact:
   a Pi-conformance checklist extracted from pi-mono.
2. **No per-token streaming.** The workbench adopts whip's turn/effect
   log shape (`tool_execution_start/end` ≈ effect dispatch/settle);
   token-level streaming is explicitly not a requirement.
3. **Multimodal is a real build item** (image content on agent-turn
   input). **Auth moves to gaugewright** per §2.

## 5. The two event-sourced stores (SETTLED: separate, with three seam disciplines)

Full merge was considered and **rejected**: it inverts authority (the
policy authority's ledger inside the machinery owner's store), couples
compliance to whip's schema cadence, muddies the open-core/BUSL/private
licensing seam, and its payoff shrinks anyway since standalone whip needs
its own store regardless. Pure separation's three costs (double-entry
drift — the failure mode un-tie already measured with git as conformance
debt CONF-1..3; no cross-store atomicity; two cut machineries) are bought
back by three disciplines:

1. **One owner per fact; references, not copies.** Every fact class is
   owned by exactly one log. Gaugewright admits *decisions plus
   pointers* ("runtime session settled per whip log position X"), never
   duplicated runtime events — its own content-behind-handles discipline
   (`INV-10`) applied to whip facts. Drift becomes structurally
   impossible; cross-store joins live only in rebuildable projections
   (`INV-5`). **Adopt from day one — cheap at design time, expensive to
   retrofit.**
2. **The cut is a position pair.** Both logs are append-only, so a
   handoff/backup cut = (gaugewright scope positions, whip workspace cut
   id) under a brief write-fence; `STATE_BEFORE_HOME` discharges over the
   pair. The two-plane cut generalizes to the two-store cut with the same
   cheapness; both sides' idempotency (their `INV-19`, whip's effect
   keys) makes fence-then-replay safe.
3. **The seam is a modeled contract.** Idempotent command/observation
   crossing with explicit jurisdiction (every governance-meaningful whip
   side-effect maps to an admitted gaugewright command; the rest is
   declared whip-internal), modeled in its own small Quint spec shaped
   like `federated-delivery.qnt` — the store seam *is* a
   federation-shaped crossing.

Optional: **physical co-location as a deployment choice, never a
requirement** — one desktop SQLite file, namespaced schemas, makes seam
transactions atomic and the fence free; the contract is written against
fence semantics so co-location stays an optimization.

## 6. Dependency-ordered plan

1. **Build the versioned-workspace floor** (branches, virtual working
   set, materialization, blob-level merge v1, reconciliation, workstream
   tier) — prerequisite for everything; already whip's plan.
2. **Whip workspace API + reimplement `crates/workspace`** over it (13
   operations; merge reducer untouched) + **handoff export**
   (manifest+blobs, erasure-respecting).
3. **`WhipHarness`**: chat-as-instance, turn driving, abort, fork;
   effect-log projection for the workbench; pending-approvals mapping.
4. **Pi-conformance port** of harness-owned behaviors (§4.1) +
   **multimodal input**.
5. **Definition migration**: archetype-as-whip-package;
   `.agent-config.json` policy → capability registry/IFC declarations;
   sandbox retired to defense-in-depth. **Policy epochs + auth
   relocation** (§2).
6. **Seam hardening**: the store-seam contract + Quint model; position-
   pair cuts wired into handoff/backup; taint-precision upgrade into
   review.

## 7. Risks

- Everything gates on whip build-out that is currently all design
  (step 1).
- The erasure story must be *proven* better, not assumed — their models
  name the exact invariants to discharge (`HISTORY_PRESERVED`,
  `EXPORTED_COPY_NOT_RECALLED`).
- Tool-loop quality is where "replace Pi" could quietly degrade the
  product if treated as plumbing — the conformance-port decision (§4.1)
  is the mitigation.
- Cross-repo governance: this note lives in whip's corpus; un-tie's specs
  are authoritative for un-tie. On ADR, each repo records its own half
  (whip: substrate capabilities; un-tie: adapter + migration).

## 8. Non-goals

- Not absorbing gaugewright's core: reducers, Quint models, admission
  shell, federation/handoff, review/export, packages-as-product, and all
  policy authority stay gaugewright's.
- Not merging the stores (§5 — settled).
- Not per-token streaming (§4.2 — settled).
- Not cross-workspace whip federation — gaugewright federation carries
  the seams; whip's multi-device question stays deferred on its own
  track.

## 9. Settled vs. open

**Settled in principle (Jack, 2026-07-04):** the goal itself; the
authority split + policy epochs + auth relocation; gap resolutions
(Pi-conformance port / no token streaming / multimodal required);
separate stores + the three seam disciplines, full merge rejected;
one-owner-per-fact from day one.

**Open:** policy-epoch snapshot format and delivery channel; the
Pi-conformance checklist itself (extraction pass over pi-mono 0.73.1);
handoff-export format details; whether the workbench needs any log-shape
adapter beyond direct projection; the store-seam Quint model's scope;
migration sequencing (the M1/M2 milestone analysis in the 2026-07-04
sweeps is **stale** per Jack — sequencing is decided at build start, not
assumed); the diff/presentation surface for review UX;
where the `whip` primitive in un-tie's specs lands once whip is *below*
rather than above (both postures coexist during migration); **whether the
workspace API ships git-backed first** (jj's backend-split adoption path —
semantics early, storage swap later; the readiness tracker's ⚑ sequencing
fork).

## 10. Relationships

- **Versioned workspace** — the git-replacement substrate; this effort is
  its first external customer and the strongest forcing function for its
  build.
- **Workflow encapsulation / owned harness** — the Pi-replacement
  authority story (shipped theorem).
- **Compute plane** — Class-B containers/exec serve the DO-hosted future;
  desktop un-tie runs whip native.
- **Experimentation subsystem** — gauges over placements and campaign
  records as archetype-version archaeology are the natural post-
  replacement product payoff (gaugewright is named for them).
- **Open-core seam** — the licensing boundary is a load-bearing reason
  for the separate-stores decision.
- **Un-tie specs** — ADRs 0029/0031/0033/0046, `specs/primitives/whip.md`,
  the merge/workstream/handoff lifecycles and their Quint models.
