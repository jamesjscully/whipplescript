# The Versioned Workspace — Research Note (whip-native version control)

**Status: RESEARCH NOTE (pre-ADR).** Opened 2026-07-03 (Jack), out of the
regeneration side-effect containment thread in the experimentation effort
(`experimentation-subsystem-research-note.md`; `improve-design-note.md`,
"Dependencies, sequencing").
Proposes a whip-native version-control substrate — **the versioned
workspace** — replacing git for whip project state and completing the
checkpoint substrate into a branching one. **Nothing scoped or committed**;
this note defines the target and its relationships. Scope semantics —
what branches at all — is settled in principle (§8); the regeneration
containment question lands on top of it (§9) as a derived per-door
posture, not a sandboxing project.

## 1. The problem — two converging pressures

**(a) Durable project state for agent ecosystems.** This system is an
intellectual descendant of coding agents with a full dev environment, where
git is the usual durable store. But whip must run where git does not (the
Durable-Object host), and git is failing coding agents even at home:

1. **Parallel work requires worktrees** — whole-directory copies; slow and
   inefficient.
2. **Non-technical users don't know the verbs.** Commit, push, pull — they
   can't ask for what they want and don't want that level of control.
3. **Feature bloat.** Notes, submodules, reflogs — baggage from decades of
   human-centric workflows.
4. **No auto-reconciliation.** Agents want CRDT-like or at least fast
   automatic merging, reconciled quickly and often; nothing should change
   files under an in-flight agent's nose, and no test suite should break from
   another agent's half-done work. Instead: ad-hoc worktrees, agents
   rewriting over one another, or the user mentally burdened as the
   coordinator — and automation produces merge-conflict hell.

**(b) Regeneration side-effect containment.** A counterfactual
(regenerated) run re-executes effect sites; its writes must not land in real
project state. "A VCS with pointers to commits" is the natural shape of the
answer — but checkpoints as designed (restorable context) are **linear**
restore points, and none of the existing VCS options fit (a).

## 2. Diagnosis — git's model, not git's features

Git is **patch theory over text, applied to a mutable POSIX working tree,
with no mediator**. Every failure above follows from the model:

- Worktrees copy directories because the working tree is mutable OS state
  that cannot be virtualized (1).
- The verbs exist because the *user* is the transaction manager — staging,
  committing, pushing are a human hand-driving what a database does
  automatically (2, 3).
- Merges are line-based because text is all git can see: it silently merges
  semantically-conflicting edits on different lines (the worst failure mode)
  while manufacturing fake conflicts from adjacent-line noise; and with no
  mediator, divergence accumulates until merge hell (4).

What agents actually need has a name in the database world: **snapshot
isolation with MVCC**, plus causal history, plus semantically-licensed
merge, plus continuous mediated reconciliation. Readers see a pinned
snapshot (nothing moves under their nose); writers never block readers;
reconciliation is the *system's* continuous job, not a user-scheduled event.

## 3. The reframe — the workspace is a database, not a directory

Whip has already made the decisive commitment, twice: the restorable-context
DR (file state event-sourced in the runtime-owned storage plane — explicitly
NOT git/worktrees) and DO DR-0033 Decision 4 (files as content-hash handles,
runtime-owned tiering). The versioned workspace completes that commitment.
Checkpoints are the **linear special case**; three things are missing, and
all three land on prepared ground:

1. **Branches** — cuts that can have *divergent children* (§4).
2. **Merge semantics** (§6).
3. **The concurrency model** presented to agents and users (§5, §7).

## 4. Branching — O(1), because the substrate already paid for it

A branch is a new manifest pointer sharing every blob by content hash.
Cheap branching is *free* on a content-addressed store; the worktree-copy
problem exists only because git's working tree is physical. Whip's file
surface is sandbox-mediated, so the runtime presents each agent/instance a
**virtual working set** materialized from its branch's manifest,
copy-on-write. A hundred concurrent agents cost a hundred pointers plus
their actual divergent writes. Failure (1) is not mitigated; it ceases to
exist.

## 5. Verbs — there are none; the real ones already exist

**Continuous versioning** kills the commit ceremony: everything is
event-sourced, so there is no staged/unstaged and no "did you remember to
commit" — cuts happen automatically at marks, terminals, and edits. And the
experimentation surface already discovered the user-facing verb set before
anyone knew it was a VCS surface:

| user intent | verb (already designed) |
|---|---|
| keep this state | `pin` |
| try something without consequences | `suppose` / a campaign candidate |
| take that one | *adopt* |
| go back | *undo* (restore) |

Those are the intents non-technical users actually have. Commit/push/pull
never appear because the user was never the transaction manager. Failures
(2) and (3) resolve by construction: the feature set is derived from the
runtime's own operations, nothing inherited. (Naming for branch-like objects
shown to non-technical users — "drafts"? — is a deferred bikeshed, §15.)
Revision 2026-07-05: beyond the whole-branch verbs, user-directed *surgical*
requests ("undo only this file") get the **selective verbs** — see §7.3.

## 6. Certified merge — the slicer's fourth client

Merging two branches **is composing two edits**, and the improve effort
already proved the composition theorem (disjoint-slice composition =
licensed crossover). The merge rules fall out:

- **Disjoint blast radii → certified auto-merge**, silent and continuous —
  including edits to *the same file* that provably don't interfere, which
  git can never grant. This is the overwhelmingly common case of agents
  working on different parts of a system.
- **Overlapping blast radii → a real conflict**, surfaced honestly —
  including edits to *different files* that do interfere, which git silently
  merges into a semantic bug. Conflict detection stops being "same lines"
  and becomes "intersecting slices" — the dependency analyzer paying a
  fourth time (after IFC, evidence transfer, and improve composition).
- **Merged results pass the gates** — check, lint, IFC, tests — and the
  gauges: post-merge ambient evidence is the semantic regression detector,
  with the standing-contradiction machinery as the long-tail net.

### 6.1 The decomposition, per substance plane (deepened 2026-07-04)

Scope semantics (§8) already shrank the problem — the workspace plane never
forks and rights serialize — so merge faces **substance only**, and it
decomposes:

- **Whip source — the real engine.** Source is a set of declarations, not
  a text file (`fmt` canonicalization makes textual position presentation).
  The merge is three-way at **declaration granularity** over the
  branch-point base: both-modified-same-declaration is trivially a
  conflict; everything else is the composition theorem — disjoint closed
  slices → certified auto-merge (including same-file edits git could never
  grant), intersecting slices → real conflict (including cross-file
  interactions git silently mis-merges). The anti-dependence discipline
  transfers verbatim from evidence transfer: one edit's write-or-consume
  footprint intersecting the other's read footprint ⇒ no certificate, fail
  closed to conflict. Renames/moves ride the **same canonicalization as
  evidence identity** — one alpha-equivalence ADR serves both. v1
  granularity: whole-declaration, fail-closed; statement-level merging is
  a precision upgrade, not architecture.
- **Runtime state — dissolves; the instance is the unit.** Event logs are
  per-*instance*, and an instance is born on a branch, so merging runtime
  state is **set union of instances**, conflict-free. The
  in-flight-at-branch-time case dissolves too: *stepping* the same
  instance on a second branch **is forking it** — literally what
  counterfactual regeneration does, with branch-distinct effect keys
  already giving the fork its identity. Reading a snapshot of an in-flight
  instance is free on any branch; stepping stays pinned to its home branch
  or is an explicit fork. Divergent histories of "the same" run cannot
  exist, so merge never reconciles them.
- **Blob files.** Path-level three-way; both-modified conflicts unless
  byte-identical; content addressing makes rename detection trivial;
  resolution stays escalate-never-fake (precise provenance-carrying
  detection). One addition: **LLM-assisted resolution may exist as a
  *proposal*** — an agent merges the two versions and the result passes
  gates plus the owning agents' review like any candidate — policy-gated,
  never silent.

(Counters remain PN-shaped and leases exclusive-by-design, but scope
semantics explains why that list was always short: the "trivially
mergeable" planes are workspace-plane and *never forked at all*.)

### 6.2 Confluence, and gates as merge trains

The merge certificate is the composition theorem restated over manifests —
and it rides the same 4-edge-kind closure completeness as transfer,
improve crossover, and identification (the central bet's fourth client;
the Tier-A spot-audit tripwire now guards all four). Two operational
consequences: **confluence** — pairwise-disjoint edits commute, so
certified merges are order-independent and the mediator folds N branches
in any order, with only the connected components of the *overlap graph*
escalating jointly; and **gates as the second opinion** — a certified
merge still passes check/lint/IFC/tests before mainline advances, batched
as a **merge train** (bors/merge-queue pattern) where certificates make
most trains trivially green. Gate cost, not merge cost, sets the
reconciliation budget; trains amortize it.

## 7. Mediated MVCC, not CRDT

CRDTs solve the *mediator-less* problem: reconciling writers who could not
coordinate. **Whip has a mediator** — the runtime serializes writes per
workspace (single-writer-per-DO makes this literal on that host). So the
need is mediated MVCC + certified merge + async replication of
content-addressed stores across deployments. That delivers the CRDT
*experience* — agents never see each other's uncommitted chaos;
reconciliation is fast, frequent, and mostly automatic — without CRDT math
and its semantic contortions. True CRDT treatment is reserved for a
genuinely-disconnected multi-device future, if it ever matters. Failure (4)
resolves as: snapshot isolation for stability, certified merge for speed,
the mediator for cadence.

### 7.1 The reconciliation daemon — "nothing changes under your nose," formalized (2026-07-04)

Reconciliation has two directions with different policies:

- **Rebase-down** (mainline moved; refresh a working branch's base). The
  founding promise becomes a precise contract: **your nose is your
  branch's slice.** A mainline delta disjoint from everything the branch
  has read or written rebases in *silently and continuously* — divergence
  never accumulates where it doesn't matter. A delta intersecting the
  slice waits for a **quiescence point** (a terminal, a mark, an agent
  finishing its task — never mid-run; per-run snapshot isolation stays
  absolute) and arrives as a notification-and-ask, workspace-plane speech
  to the branch's agent.
- **Merge-up** (branch → mainline): event-driven at completion/adoption,
  serialized by the adoption lease, batched by the train (§6.2).

Cadence is therefore not a timer — it is event-driven at quiescence
points, with one real knob: a **staleness bound** (a branch whose base is
too old must rebase-down before it may merge up). And detection moves
earlier: since working branches' slices are tracked continuously, the
mediator performs **conflict prediction** — it flags two branches'
in-progress slices *beginning* to overlap the moment contention starts
("draft-7 and draft-9 are both editing the triage slice"), while
coordination is still cheap. Merge hell is mostly the *lateness* of
conflict discovery; the slicer (its sixth client) removes the lateness.
Escalation policy: the mediator never guesses — conflicts surface to the
owning branches with both versions and provenance; mainline holds no
automatic priority; the only automatic actions are the safe directions
(silent disjoint rebase-down, stale-base refusal, a train reverting a
member that broke the gates).

### 7.2 The workstream tier (imported from un-tie, 2026-07-04)

Adopted from the un-tie workstream model (un-tie repo:
`specs/primitives/workstream.md`, `specs/lifecycles/workstream.md`,
ADR 0046; invariants discharged in `workstream.qnt`) — the missing middle
tier between per-agent branches and mainline. A **workstream** is an
explicitly-created, named shared line with a **membership set**: branches
homed to it sync greedily among themselves, and the line promotes to
mainline through one explicit boundary. The workstream owns the name and
the membership; the merge engine owns every advance — same division of
labor as un-tie's, where the stream delegates all ref-advancement to the
merge lifecycle.

- **Two-tier sync policy.** In-stream: the §7.1 daemon in its greediest
  mode — auto-admit, eager rebase-down, conflict prediction across members
  (most valuable exactly here, where slices are likeliest to collide).
  Un-tie's "auto-admit still requires a clean git merge" upgrades to
  "auto-admit still requires a **certificate**" — the in-stream gate is
  semantic, so intra-group conflicts are rarer and more meaningful.
  Promotion to mainline: the boundary-gated hop — full gate battery,
  merge train, adoption lease.
- **Membership is single-valued and fail-closed**: a branch homes to
  exactly one parent line — its stream's main, or mainline with no
  membership ("a workstream of one"). Joining a second stream means
  leaving the first; the sync topology stays a tree, which keeps the
  confluence story simple. A *member* is an agent session or human
  working context homing its branch to the stream's line.
- **Conflicts are per-contribution, never stream-global** — un-tie's
  *tested* lesson (they reversed an earlier stream-global-conflict
  design): a failed greedy push isolates that one contribution into
  repair; the stream stays active. Adopted on day one.
- **Archive re-homes members** to mainline (with a rebase-down pass) —
  no branch is left syncing into a dead line. Their turn-finalize sync
  moment is our quiescence points — independent convergence on
  sync-at-semantic-completion.
- **Scope semantics refines it**: a workstream groups **substance only**.
  The knowledge plane was never per-stream, so members already share
  learning (issues, evidence) with the whole workspace while their
  substance stays stream-local — a split un-tie's git substrate cannot
  express. Postures compose orthogonally: members are working branches;
  counterfactual candidates are not members of anything (though a
  campaign may *target* a stream's line — registered in the improve
  note).
- **Deliberate limits carried over**: one level of nesting (streams
  promote to mainline; no streams-within-streams until someone needs
  them), one workspace scope (cross-workspace collaboration stays with
  the deferred multi-device question). The mediator is the single home
  trivially (single-writer-per-workspace). Un-tie's two discharged
  invariants — *membership-gates-autosync* and *archive-rehomes-members* —
  import directly into this note's formal-model plan as coverage+bite
  candidates.

### 7.3 Selective operations — the interactive VCS surface (settled in principle, 2026-07-05)

Opened by Jack's git-replacement concern: the design above covers the
*automatic* substrate and whole-branch verbs, but users direct agents with
surgical requests — "undo only this file," "take that one change," "help
me resolve this merge" — which is exactly where git's enormous feature
surface lives. The answer rests on a census of git's actual operations,
classified by what each *semantically is*.

**The census: most of git's surface is compensation; the payload is
selection.** Git's features mostly compensate for its own model: **stash**
exists because the working tree is singular (here: a branch); **the index
and staging** because commits are ceremonies over a mutable tree (here:
continuous versioning — though the index's *second* job, the stat cache,
survives as an implementation detail: §10.1); **reflog** because refs are
mutable pointers with
no history (here: the event log *is* the reflog, first-class); **rebase /
squash / amend / ff-vs-no-ff** because git conflates the record with the
narrative (here: **the record and the narrative are separate** — the event
log records everything, adoption takes a *delta*, presentation is a view —
so the entire history-rewriting debate space evaporates); **fsck/plumbing**
because git's verbs are destructive (here: see below). These absences are
features, stated deliberately. What remains after the compensations is a
hard core with one shape: **selective operations** — cherry-pick, revert,
per-file checkout, partial staging are all "the user's intent is a
change-set, approximated by textual patches over snapshots."

**The primitive: a provenance-native change-set selection algebra.** Every
write is already a provenance-carrying event (instance, effect, agent,
branch, time), and the slicer knows dependencies between changes. Select
recorded changes by path, declaration, effect, instance, agent, time
range, branch — or by *semantic impact* ("everything that touched the
extraction slice"), a dimension git cannot offer. Three verbs over
selections:

- **`undo <selection>`** — construct a new cut = current state minus the
  selected writes, as a **proposal on a branch, never a mutation**. The
  slicer checks the exclusion's **dependency closure** ("undoing these
  writes strands two later edits that read them" — its seventh client);
  and the result is honestly a **counterfactual state** — a state that
  never ran — so the chimera-coherence machinery applies verbatim: gates +
  gauges revalidate, honesty tags mark it synthetic until they do. Git's
  file-scoped revert manufactures the same state *silently*.
- **`transport <selection> onto <line>`** — cherry-pick done right.
  Selection slice disjoint from the target's divergence → certified clean
  transport; overlap → honest conflict. And transport **preserves
  identity** (revert-reunification's mechanism pointed at a new verb):
  content addressing means the eventual full merge recognizes the
  transported change as *the same change* — killing cherry-pick's classic
  sin of ancestry-less duplicates that re-conflict later.
- **`adopt --only <selection>`** — partial adoption: `git add -p`'s real
  intent relocated to where it belongs. You don't stage fractions of a
  mutable tree; you adopt fractions of a branch's delta, and the remainder
  stays live on the branch instead of festering as uncommitted state.

Previews are the default interaction (dry-run shows the resulting delta
and what it strands) — agents show before they do.

**The conflict surface for agents.** Escalation delivers conflicts as
**structured objects**: per-declaration (whip source) or per-path (blobs),
each carrying base + both sides + both sides' *provenance* ("yours is
draft-7's retry refactor; theirs is Sarah's timeout fix; they overlap on
this declaration") — not `<<<<<<<` markers. Resolution is per-item
(take-A / take-B / authored merge), is itself a provenance-carrying edit,
and re-runs the gates. **Resolution memory** comes nearly free: conflicts
are content-addressed *pairs*, so a previously-resolved identical pair
offers its prior resolution — rerere without the fragile hidden cache,
because resolutions are ordinary workspace-plane knowledge.

**Archaeology.** `blame` is strictly dominated by provenance queries
(line-attribution is a weak projection of "which effect/agent/instance
wrote this, under what intent"); `log` → event log + lineage views +
`why --history`; `bisect` is mostly *pre-answered* by the evidence ledger
and quasi-experimental attribution ("which edit degraded the gauge"), and
when genuine bisection is needed it runs checkout-free over materialized
cuts. These need surfaces, not machinery.

**No destructive verbs — and the honest hatch list.** Git needs escape
hatches because its verbs destroy mutable state; agents wielding
`reset --hard` / `push -f` are today's leading VCS disaster mode. Here
**the VCS surface contains no destructive verb at all**: every operation
is a proposal over an immutable record; undo-of-undo is trivial;
"recovery" is a query, not a ritual. Most escape hatches become
*unnecessary*, not missing. What legitimately remains, each a designed
feature: (1) **manual override** — author the desired state directly as
an ordinary edit and adopt it with human authority; plain editing is
complete over states, so the model never traps you; (2) **store-level
disaster recovery** — export bundles + cuts; (3) **the git bridge** as
the interop hatch. That is the complete list.

**Imported from jj (folded 2026-07-05).** The closest prior art
(working-copy-as-commit, autosnapshot, no staging, op log, first-class
conflicts) was mined late; the census reproduced most of it by
derivation — validation — and four lessons from jj's years of production
iteration import directly:

- **Dual identity: content + intent (settled).** jj's killer primitive —
  a **change id** stable across rewrites, separate from the commit hash.
  Adopted: change-sets carry a stable edit/intent id alongside their
  content hashes; transport preserves it; merges reunify on *either* —
  content-identical → certificate-grade, intent-identical-but-
  content-divergent → a **detected divergent change** presented with both
  versions, never a mystery conflict. This resolves the
  transported-then-edited residual, and it names the persistent object
  the lineage DAG and edit-type priors already wanted ("the same edit,
  evolved"). New residual inherited with it: the divergence
  *presentation* UX, jj's known-rough spot.
- **Conflicts don't block (settled).** A **conflict-bearing cut is a
  legal, honestly-tagged state**: recordable, explorable, buildable-upon,
  rebasable-across — never adoptable to mainline while conflicted (the
  gates enforce that), and resolution becomes ordinary, assignable,
  durable work while siblings proceed. Resolutions **auto-propagate to
  descendants** through the reconciliation daemon — jj's implicit rerere,
  extending the resolution memory structurally. Composes with (does not
  replace) per-contribution isolation.
- **The selection algebra is a revset-shaped language (settled).** jj's
  revsets/filesets prove ordinary users wield a small functional
  expression language — so selection is composable expressions closed
  under union/intersection/difference, with the structural operators jj
  cannot have: `dependents-of(S)`, `slice-of(gauge)`, `by-effect(kind)`,
  `in-branch(b)`. One grammar feeds the three selective verbs, the
  archaeology queries, and plausibly gauge scopes.
- **Confirmations with teeth**: no-staging is validated *empirically* by
  jj's adoption, upgrading the census claim from argument to fact;
  op-level undo is jj's most-loved feature, so **workspace-operation undo
  is a front-and-center verb here** ("undo the adopt", "undo that
  reconciliation"), not a buried property; anonymous heads + optional
  bookmarks validate the names-are-optional-labels posture for the
  "drafts" bikeshed; autosnapshot's performance scars confirm the §10.1
  stat cache and its fs-watcher upgrade path. The structure jj lacks —
  workstreams — is what keeps anonymous-head freedom from drifting into
  unstructured megamerges; they stay load-bearing.

## 8. Scope semantics — branchability is a property of the referent

**Settled in principle, 2026-07-03.** Opened by the second review pass's
branch-mode finding: the containment
design implicitly made *every* branch counterfactual, but working branches —
parallel agent work, pressure (a), this note's founding use case — need live
doors (an agent on a feature branch that hits `human.ask` should ask, not
terminate as needs-human). Stepping back showed the membrane drew **one line
where there are two**. The doors answer "what leaves the *trust* boundary?"
— but a tracker item never leaves the store and still resists branching.

**Branchability is a property of the referent, not the store.** Every piece
of state represents something. If the referent is the **artifact under
construction**, branching is the point — two versions are two legitimate
hypotheses. If the referent is **shared reality** — another mind, a
real-world resource, the project's accumulated knowledge — forking the
representation does not fork the referent: two branches with different
beliefs about "what we told the user" are not two hypotheses; at least one
is a lie. Git got this right by architectural accident (code branches;
issues and Slack don't; nobody has ever been confused) — whip's uniform
store *creates* the confusion by making everything look equally versionable.
This section recovers the accidental wisdom on principled grounds.

### 8.1 The taxonomy — what the workspace makes, knows, owes, and says

- **What the workspace *makes* — branch-transactional.** Files, whip
  source, program-derived facts, the run event logs of the branch's own
  instances. The artifact; it branches, and it is the **only class the
  certified-merge engine ever faces**. Discarding a branch discards work
  product — that is what discard means.
- **What the workspace *knows* — workspace-monotone.** The evidence ledger,
  tracker issues, campaign records, telemetry. The decisive argument: **work
  is transactional, but learning is monotone.** A branch tries an approach,
  files the bug it found, fails, is discarded — the *work* should vanish;
  the *knowledge* must not ("we tried X and here's why it failed" is often a
  failed branch's most valuable output). Branch-local knowledge stores would
  un-learn every discarded experiment and make concurrent agents re-discover
  each other's bugs. So: workspace-scoped, append-only, written greedily,
  with **branch provenance mandatory on every entry** (the marking condition
  of §9.2 is a working-branch requirement, not just a counterfactual one).
  The evidence ledger conformed before the principle existed — content-hash
  keys plus the `branch-ref` column.
- **What the workspace *owes and holds* — referent-scoped.** A lease has no
  intrinsic scope; it **inherits the scope of the resource it guards**. A
  lease over a branch-local file: contention exists only within the branch —
  the lease key includes the branch id, and two branches holding "the" lease
  lease different files. A lease over a shared referent (an external API's
  rate limit, a device, the human's attention): exclusion is a fact about
  the world — the table must be workspace-global or the exclusion is fake.
  The **resource registry declaration** decides (§8.4, fork 3), defaulting
  workspace: over-serializing a branch-local resource costs concurrency;
  under-serializing a shared one costs correctness. Budgets and spend caps
  are **workspace-forced**: a branch must not escape its campaign's budget
  by being a branch.
- **What the workspace *says* — workspace-irrevocable.** Messages,
  notifications, asks. Speech acts to other minds cannot be rolled back by
  discarding a branch and cannot coherently fork; they carry branch
  provenance ("draft-7 reports: ready for review") so the recipient knows
  what the claim is about. Ingress is the mirror: the world's messages land
  globally; a reply routing to the asking branch's instance is addressing,
  not scope.

Hard cases, honestly: **knowledge about branch-only state** ("foo crashes"
where foo exists only on draft-7) is the price of monotone knowledge;
provenance is the mitigation and it suffices — the claim is true *about
draft-7*, and if draft-7 dies the issue remains the record of a dead
experiment (branch-local trackers, the alternative, recreate knowledge
silos — strictly worse). **Immutability launders scope**: a pin made on a
branch is globally safe immediately because it names a content-addressed
cut; the scope problem exists only for mutable state. **Queue items** carry
branch-addressed scope in the key inside a workspace-plane store. **Counters
split by referent**: items-processed-this-run is substance;
requests-against-the-API is a workspace fact.

Alternatives rejected: **everything branches** (un-learns discarded
experiments, silos concurrent agents, forces merge semantics onto stores
that never needed to fork, makes real-world exclusivity fake); **only files
branch** (forfeits the point — facts and coordination are the artifact too);
**per-store user configuration** (scope is a semantic property of the
referent, not a preference; config invites incoherent settings — a
branch-scoped budget is a money bug. Configuration belongs only where scope
is genuinely referent-dependent: the resource registry, nowhere else).

### 8.2 Scope and isolation are orthogonal axes

The obvious objection — greedy-global knowledge means the tracker changes
under an in-flight agent's nose, and whip rules can *read* tracker state,
making it dataflow — conflates two independent axes:

1. **Scope** — which timeline a *write* lands on: the branch's own
   (substance) or the workspace's (knowledge, rights, speech).
2. **Isolation** — what a *read* sees: always the run's snapshot, with every
   cross-run read a recorded ingress event — **which replayability already
   required unconditionally.**

Snapshot isolation was never a promise about scope. A run on branch B reads
the global tracker at its snapshot and records what it read; branch A's
write lands globally but reaches B's *next* run. "Nothing changes under your
nose" survives intact; scope only decides whose *subsequent* runs see your
writes. And the recorded-read discipline that makes global stores safe for
isolation is the same discipline that makes them replayable.

### 8.3 The workspace plane — "global" does not mean "mainline"

If workspace-scoped stores were "on mainline," greedy writes from branches
would violate mainline's own snapshot semantics. They are not. The workspace
has **two planes**: the **branch DAG of substance** (mainline = a
distinguished branch) and the **workspace plane** of monotone stores —
evidence ledger, tracker, speech logs, resource tables — sitting *beside*
the DAG and referencing branches by id (the evidence ledger already lives
exactly there). Three payoffs: the **merge problem shrinks to substance
only** — a workspace-monotone store never forks and never needs merging
(§6's "trivially mergeable append-only planes" are really *never-forked*
planes); the rights tables **serialize rather than merge** (§6's
observation, now explained); and **adoption itself is a workspace-plane
operation** — the right to merge into mainline is a workspace-scoped leased
resource, held by one branch at a time, mediated like everything else.

### 8.4 Four resolved forks (2026-07-03)

1. **Knowledge writes are instant at the plane; batching is only ever
   presentation.** Monotone stores gain no consistency from batching
   (batching is merge-avoidance; these never merge); dup-prevention needs
   immediacy; the ledger's anytime statistics *require* immediate landing
   ("every observation lands immediately" is the design's center); and crash
   safety enforces work-transactional/learning-monotone mechanically — a
   branch that dies mid-run loses its work, not its learning. The one real
   counter-argument (a flailing branch spamming half-formed issues) is a
   curation problem with a curation-layer answer: surfaces may debounce,
   group, and rank; revision is itself monotone append. Semantics instant;
   presentation free; the two never trade.
2. **Ask dedup falls out of the taxonomy — the *answer* is knowledge,**
   therefore workspace-scoped, therefore shared by construction. The
   question side is subscribe-don't-duplicate on the pending-asks queue,
   keyed by content hash of payload + referents. Immutability launders scope
   again: anchor comparisons on content-addressed outputs — the common
   cross-branch case, improve campaigns racing candidates — dedup
   *perfectly*; questions about branch-mutable state dedup only when
   content-identical, which is rare and correct. EVSI consequence: an
   interruption's value **sums over subscribers** — a question blocking five
   candidates outranks one blocking one; demand aggregation allocates the
   scarcest resource better under branching, not worse. The synthetic
   respondent never enters the anchor pool regardless of subscriber count.
3. **Scope grammar: one field, safe default, and branch scope is a claim
   the slicer can refute.** `scope workspace | branch` on registry
   declarations; omitted = `workspace` (matching today's coordination
   store). No instance scope (holder identity already lives in the lease
   key); no cross-workspace scope (undesigned); queues get item addressing,
   not store scope. **Workspace-forced families** — provider spend, channel
   rate limits, exec quotas, anything guarding an egress door — reject
   `scope branch` at declaration, not warn. For user-declared resources,
   `scope branch` claims "this contention lives entirely inside a branch,"
   and **the dependency analyzer checks it**: guarded sites reaching an
   egress door refute the claim, failing closed to workspace scope — the
   slicer's fifth client (IFC, transfer, merge, improve crossover, scope
   validation), keeping the design's one user-facing knob honest rather
   than trusted.
4. **Raw run logs are substance; extracted observations are knowledge — and
   knowledge holds GC roots into substance.** Ledger rows, scores, and
   identity keys are workspace-plane, tiny, effectively immortal. An
   observation's checkpoint-ref/output-ref, a scenario pin, a campaign
   record pin *exactly the blobs they name* — content addressing makes
   "unrooted except where knowledge points into it" a computation, not a
   heuristic. Branch discard collects the unreferenced remainder (typically
   most of it); tiering spills the cold pinned fraction. Under real
   pressure, referenced substance may be collected **only with an honesty
   downgrade, never a dangling reference**: the observation keeps score +
   hashes (identity and pooling verdicts survive — lazy hash derivation
   needs only the tiny pinned program versions), loses replay and
   `why`-level drill-down, and carries a provenance-pruned tag. This
   *derives* the GC root set §15 previously gestured at.

## 9. The boundary identity — versioning boundary = IFC egress boundary

The payoff that motivated this note. Regeneration containment splits
exactly in two:

- **Storage-plane effects** (file writes, facts, coordination updates,
  queue/ledger operations): a counterfactual run executes **on a branch**.
  Its writes are fully real to the run itself, invisible to the mainline,
  discarded or adopted wholesale. Containment is not a mechanism bolted onto
  regeneration — it is what branch semantics *means*. Nothing stubbed, no
  fidelity loss.
- **Egress effects** (send to a real human, notify an external channel, exec
  against the real host, provider side effects beyond paid inference): the
  world has no branches. These cannot be versioned, only diverted, stubbed,
  or refused — and the set of effects in this class is **exactly the set the
  IFC egress doors already enumerate**. The doors answer "what leaves the
  trust boundary?"; the VCS asks "what can't I branch?" — the same list.

**Inside the membrane, everything is branchable state; at the doors,
branching fails and policy takes over.** Two systems built for different
reasons — information-flow control and the runtime-owned storage plane —
define the same boundary; that convergence is the strongest signal the cut
is right. Containment therefore reduces to a **per-door policy** for
counterfactual runs — settled in principle in §9.1–§9.3 below
(replay/divert/live verdicts) — plus **canary/deliberate reversal as the
consented exception** where a branch is deliberately allowed to touch the
world (one consent surface governs both — `improve-design-note.md`,
"Canary").

### 9.1 Per-door containment policy — derived as posture (settled in principle 2026-07-03; rebased on §8)

**Rebase (2026-07-03): with §8 in place, this policy stops being stipulated
door-by-door and derives from two rules.** First, every branch carries a
**posture** — a grant vector over the workspace-scoped categories
(knowledge / rights / speech / ingress), per **role**. Second, a
counterfactual branch hosts two actors with opposite needs:

- **The subject** — the workflow under test; its behavior is the measurand.
  Its workspace-plane writes must not land: speech obviously, and
  **knowledge too** — a `settle` re-executes the subject N times, and a
  subject whose logic files a tracker item would flood the plane with N
  mode-marked copies of a hypothetical claim. Subject writes are *behavior,
  not learning*: they divert into branch-local would-have-written facts,
  judgeable like any diverted payload.
- **The instrument** — the measurement machinery: gauge-declared judges,
  the ledger writer, the anchor-ask surface, spend metering. Its writes are
  the point: they land globally, instantly, mode-marked. Its speech is real
  (an anchor ask goes through the actual ask surface, EVSI-priced).

The boundary is **structural, not aspirational**: the instrument is the
runtime plus declaration-identified scorers, running in separate effect
contexts from the workflow instance. Four named postures line up:
**working** (subject holds the §8 defaults), **counterfactual** (subject's
workspace-plane grants revoked; instrument keeps its row), **shadow**
(counterfactual grants + a live ingress tee: the subject processes a copy
of real traffic and its outputs divert — consent-free, single-turn by
construction; improve note, "Canary"), **canary** (a
scoped re-grant of subject grants over a traffic slice). **Consent edits
the subject's row only; the instrument's row is never user-editable** — a
soundness property, not a UX choice. The non-egress invariant restates as
a taxonomy theorem: **the subject of a counterfactual branch holds no
authority over irrevocables** (speech + world-referent rights beyond
metered spend). Rights follow the door verdict: where a door is live, the
guarding lease is acquired for real against the workspace table; where it
is diverted, no claim is made — there is no contention for a call never
placed. And **branch-distinct effect keys generalize to all branches**:
two *working* branches running the same workflow on the same input are
distinct executions and must not dedupe against each other either —
simpler than a counterfactual special case, and it closes the
silent-corruption bug in both flavors.

Two dissolutions fall out. **Divert-then-adopt has no residue**:
counterfactual branches are measure-then-discard by construction — you
adopt the *edit* under test, not the branch's run state, and the branch's
purpose completes when its observations land on the workspace plane;
working branches never divert speech in the first place. No owed mail in
any quadrant. And the `human.ask` three tiers below were **the role
distinction trying to be discovered**: tier 2 is the *subject's* ask being
diverted; the real-ask opt-in was never an exception — it is the
*instrument's* ask, which was always allowed; the synthetic respondent is
a fake *counterparty* for the subject's diverted ask, never the
instrument — "never anchors" becomes a category error rather than a rule.
One operational residual: counterfactual work acquiring real leases (where
doors are live) must be **preemptible / lower-priority** on shared rights,
or mass regeneration starves production runs of leases (§15).

The original organizing principle and the door table survive as the
derivation's worked-out consequences:

**Organizing principle: for evaluation, egress *payloads* matter and egress
*delivery* doesn't.** A gauge judges the reply's content; whether the email
was delivered is irrelevant to the measurement. Diverting an outbound effect
into a branch-local fact is therefore *lossless for the experiment's
purpose* — the payload is the measurand. Diversion is not the compromise
position; it is the correct one.

Three verdicts per door, fail-closed toward divert:

- **replay** — the effect sits in the frozen prefix, outside the regenerate
  set's blast radius: serve the recorded outcome (the replay frontier; doors
  only pose questions beyond it).
- **divert** — perform the effect *into the branch*: capture the payload as
  a schema'd branch-local fact ("would-have-sent: channel, payload,
  virtual-timestamp"), touch nothing outside. Default for everything
  outbound; diverted payloads are first-class evidence, judgeable by gauges.
- **live** — touch the world. Reserved for doors whose worldly effect *is
  the sampling being paid for*, plus consented exceptions.

**Inviolable and enforceable: no counterfactual run performs unconsented
outbound egress, ever.** The branch-execution context is a runtime marker
(the encapsulation E2-DYN discipline is the precedent); every egress door
checks it before going live. "Counterfactual non-egress" is a
guarantee-report line, model-first (Maude bite: a counterfactual `send`
that would escape absent the marker check).

Default verdicts by door:

- **Provider inference** (`coerce`/`tell`/`prompt`/agent model turns) —
  **live**, deliberately: the worldly effect is spend + a fresh sample,
  which is what regeneration buys. Governed by spend caps. Agent-turn tool
  calls are separate effects hitting their own doors — the policy composes
  through turns. **Effect keys must be branch-distinct** (branch/cut id
  joins program_version + revision_epoch in the idempotency key), or
  counterfactual effects dedupe against real ones — a silent-corruption
  bug; deserves its own bite fixture. (Generalized by the rebase above:
  branch-distinct keys are a rule for *all* branches, not a counterfactual
  special case.)
- **Outbound messages** (`send via channel`, `notify`) — **divert-to-record**.
- **`human.ask`** — three tiers: (1) replay if the prefix recorded the
  answer; (2) default otherwise: the path **terminates honestly as
  "needs-human"** — evidence is scoped "up to the ask", tagged, never
  fabricated; (3) opt-ins: **real-ask** through the existing ask surface
  tagged counterfactual (this *is* the anchor/preference-elicitation
  machinery, EVSI-priced) and **synthetic respondent** (an agent answering
  as the human) — off by default, a distinct low-integrity source in the
  integrity machinery, and **never anchors a judge or counts as elicited
  preference** (hard constraint: a synthetic human leaking into the anchor
  set corrupts the scale).
- **`exec`** — **live-within-materialization** (file effects contained by
  the branch scratch dir + `WHIPPLESCRIPT_EXEC_ALLOW`). The network
  residual differs by host: the Phase 8 sidecar gets default-deny network
  policy (contained); **native has no backstop** — hermetic-verified execs
  run freely, unverified ones run with an evidence tag, strict workspace
  policy may refuse. Declared, not pretended away.
- **Telemetry export** — **suppress by default** (branch-scoped), opt-in
  export with an explicit counterfactual resource attribute.
- **Workflow `invoke`** — **transitive, mandatorily**: the branch marker
  rides the invocation; children's doors get the same verdicts. A policy
  sheddable by nesting is no policy.
- **Timers** — the regenerated suffix runs on a **virtual clock** (timers
  fire logically, delays compress). Counterfactual runs are the second
  customer for the virtual clock, after the flaky-timer-race fix. The clock
  mode is visible to evidence identity for clock-dependent slices — the
  experimentation note's execution-mode hazard ("Soundness hazards") owns
  that rule.

Cross-cutting: truncated-path honesty tags (needs-human, synthetic
respondent, unverified exec) ride the resulting evidence; and the **consent
surface sits above this policy, not inside it** — doors default-divert;
canary/deliberate-reversal (owned by `improve-design-note.md`, "Canary")
selectively re-opens them for a whole branch.

### 9.2 What is *not* a door — three conditions and the pump audit

"Files/queues/coordination/tracker state are branch-contained, not doors"
is true **only under three conditions** (corrected 2026-07-03 after
challenge):

1. **The store is private** — structural on the DO host (DO SQLite is
   unreachable except through the runtime); **by declared convention on
   native** (anything on the machine can read whip's storage directory; an
   external watcher pointed at it sees branch state with no effect firing).
   Native containment is conventional where DO containment is structural —
   declared, not assumed.
2. **Every escape from the store is itself an effect.** A door is anywhere
   state escapes the mediated store — *including lazy, derived, and
   subscription-shaped escapes*. Whip already contains at least one
   **store→world pump** that is not a per-run effect: the telemetry
   exporter (cursor-walks the store and POSTs outward) — it must be taught
   branches (branch-blind: mainline only; or branch-aware: flags what it
   forwards). The design obligation is an **audit**: enumerate every
   store-reader that forwards outward (exporter, mirrors/bridges, webhook
   subscribers, `whip export`) and classify each. Pumps that predate
   branches are exactly where containment silently leaks.
3. **Inspection surfaces mark branch state.** `whip status`, `whip
   evidence`, pending-asks, the LSP — humans read these and act. An
   unmarked counterfactual tracker item or ask converts a human into an
   unwitting egress channel (they act in the world on counterfactual
   state). Any surface a human treats as operational must distinguish
   branch-scoped content.

The tracker-item example generalizes: a tracker entry is internal *state*
but often a shared *commitment* — the store entry is contained; any
subscriber that mirrors it outward is a door; any human who reads it
unmarked is one too. Residual, budgetable-not-containable: counterfactual
runs consume genuinely shared finite resources (provider rate limits,
quota, disk) observably.

### 9.3 Coherence — the chimera problem, the package state surface, and the two-plane cut

Distinct from containment: **replay of the past is always valid — it is
regeneration *against* that past that can be incoherent.** The frozen
prefix embeds reads of external state as it stood at recording time (S₁);
a regenerated suffix may re-read the same external state now (S₂). If
suffix sites depend on state the prefix also read, the run is a **chimera**
— half a world at S₁ stitched to half at S₂ — and evidence from it is about
neither. The coherence condition is reach-scoped: only external state read
by *both* the prefix and the regenerate set's downstream reach matters (an
external read the prefix never touched poses no mixing problem, and the
slicer knows the difference).

The fix extends the package contract by the DR-0029 pattern (`ifc_surface`:
consumer-checked, derived from pinned source where possible). The same
contract section grows a **state surface**, per effect kind:

- **`self_contained`** — no external state read; output determined by
  recorded inputs. For whip-source packages this is **derived, not
  declared** (the slice analysis answers it consumer-side from pinned
  source). Replay and regeneration unconditionally coherent.
- **`versioned_external(fingerprint-scheme)`** — the effect reads external
  state that carries version identity (snapshot id, etag, index version,
  content hash). The recorded effect carries the fingerprint; regeneration
  is coherent iff the handler **reads at that fingerprint** (a contract
  capability: "supports historical reads") or degrades honestly.
- **`unversioned_external`** — world state with no version identity (live
  web search, unversioned API). Regeneration through such a site cannot be
  made coherent, only tagged: evidence carries **`external-state-drift`**;
  strict mode may refuse the experiment.

This is **Tier-B logic pointed outward**: program state got "version
identity or honest tags" via slice hashes; external state gets the
identical treatment via fingerprints — the fingerprint is the world's
`revision_epoch`, and `unversioned_external` is the world declining to have
one. Opaque/native provider packages *declare* their state surface —
trusted by declaration, like dataset labels, and **empirically audited**:
a package that claimed `self_contained` while secretly reading the world
gets caught by flapping replays (the hermeticity double-run). Cross-compat
lands as one coherent contract block: door class (existing `ifc_surface`) +
state surface + historical-read capability + fingerprint scheme — external
packages plug into containment and coherence through the same pinned
contract they already plug into IFC through.

Open sub-fork: must a `versioned_external` claimant *support* historical
reads to claim the class, or may it claim with fingerprint-recording only
(check-but-can't-serve — still detects incoherence, just can't avoid it)?

**The domestic twin — the two-plane cut (added 2026-07-03).** The chimera
problem has an internal face: the frozen prefix recorded the subject
reading workspace-plane state as it stood (the tracker, resource tables,
prior knowledge); a regenerated suffix re-reading the plane *now* stitches
half a run against T₁'s knowledge to half against T₂'s. So the consistent
cut spans **both planes** — and the workspace plane's half is nearly free:
the plane is monotone by design, and a monotone store's snapshot is a
**high-water mark** — one position per store. Three state classes, one
coherence mechanism: **substance checkpoints by manifest, knowledge by
position, external state by fingerprint** — the fingerprint story above was
the external face of a fully general rule. The restorable-context DR's
"cut must include coordination state" (mirrored in the DO tracker's
downstream-customer note) is one instance, now generalized and made cheap.
The pump audit gains a sibling work item: enumerate the workspace-plane
stores and define their position markers — likely the same enumeration
walked twice (§15).

**Pinned vs. current knowledge is a declared intervention.** Regeneration
reads the plane at the pinned position by default (ceteris paribus — only
the edit varies). "How would the new program behave with what we know
*now*" is sometimes the real question — but under the do-calculus framing
it is a **compound intervention** (the edit *plus* an intervention on the
knowledge inputs): legitimate if declared as an explicit experiment
parameter and surfaced in the honesty tags, incoherent if accidental. The
house template applies: exact, or declared-and-tagged, or refused.

## 10. The materialization boundary — POSIX as projection

The versioned workspace never asks existing tools to understand the
database: **it materializes real directories for them at the boundary, and
imports the diff back.** Everything that exists sorts into three zones:

1. **Whip-native state** (facts, event logs, coordination, whip source, file
   constructs, evidence, campaign records): fully in the database,
   branch-native, no POSIX anywhere. Already decided; already the storage
   plane.
2. **The exec/agent boundary — materialize-on-exec.** When a branch's run
   reaches a POSIX-needing effect (script, validator, build, test suite,
   coding-agent sidecar), the runtime materializes that branch's manifest
   into a **real scratch directory** (genuine dir: inodes, mmap, file
   watchers, subprocesses all work — no FUSE compatibility hell), runs the
   tool, and imports the diff back as content-addressed, provenance-carrying
   writes (which branch, which effect, which agent). The materialized dir
   *is* the agent's snapshot-isolated working set: nothing changes under its
   nose, and its writes touch nothing else until reconciliation.
3. **The genuinely external** (the user's own repo, editor, CI): the git
   bridge (§13). Whip doesn't try to own it.

This is the industrially proven pattern, not a hope: git itself is a
content-addressed database whose working tree is a materialization (its
mistake: making the materialization the permanent, mutable primary);
Bazel/Nix run the world's build tools in ephemeral action directories
projected from content-addressed stores; Docker/overlayfs is
content-addressed manifests + COW mounts; Meta's Sapling/EdenFS materializes
virtual working copies lazily at industrial scale; Jujutsu (jj) is the
closest philosophical relative (working-copy-as-commit, autosnapshot, no
staging, first-class conflicts). What whip adds — provenance on import,
slice-certified merge, branch semantics tied to run semantics — sits *above*
the boundary, not inside it (§11).

Costs and caveats, honestly: materialization is proportional to what the
tool touches, and reflinks/hardlinks (btrfs/XFS/APFS) make even full
materialization nearly free on native hosts (EdenFS-style lazy
materialization is a later optimization, not the foundation); import-back is
a hash-walk diff, O(touched); **what escapes the directory escapes the
model** — network calls, global machine state, `~/.config` reads are egress
and environment, not files: the doors + `WHIPPLESCRIPT_EXEC_ALLOW` remain
the honest boundary; **toolchains/environments** are declared **ambient
config hashed into identity** exactly like provider profiles (the light
road; the content-address-everything Nix road stays available if ever
needed).

**Host symmetry.** On the DO host there is no POSIX foundation at all — the
database model is forced, not chosen (that is why restorable context went
event-sourced). Zone 2 resolves identically on both hosts: native
materializes a local scratch dir (reflink-cheap); the DO host routes the
effect over HTTP to a **container sidecar**, which pulls only the blobs it
is missing from the shared content-addressed store, materializes there, and
ships the diff back — idempotent by construction (DR-0033 Decisions 3/4/7).
Same boundary, same import protocol, different machine. The versioned
workspace is what makes the two hosts *converge*: on native, POSIX is
demoted to a projection; on DO, it never existed. (The sidecar tier itself —
lifecycle, economics, enforcement — is designed:
`compute-plane-design-note.md`, 2026-07-04; the DO tracker's Phase 8 holds
the build work.)

### 10.1 All files, including large ones (settled in principle, 2026-07-05)

DR-0033 Decision 4 did the load-bearing work (content-hash handles;
stream-based size-agnostic operations; runtime-owned tiering — small
inline/transactional, large spilled to the object tier; the isolate never
buffers bytes; in-memory materialization a bounded runtime limit). Note
the comparison: **git's large-file story is that it doesn't have one** —
LFS exists because git's object model hits a cliff, and LFS is a bolt-on
second system. Whip's tiering is native; there is no cliff. Four
additions complete the story:

1. **Content-defined chunking.** Whole-blob granularity means a one-byte
   append to a 5 GB file mints a new 5 GB blob. Large blobs become
   FastCDC-style chunk trees; **the file's identity is the stable Merkle
   root**, so nothing upstream changes (file-handle edges, evidence
   identity, manifests all key on the root exactly as today); storage and
   transfer dedupe at chunk level. Small files stay whole-blob
   (threshold). Erasure composes: erase a file = erase its chunks, the
   retained root hash is the honesty-downgrade handle.
2. **The stat cache — a correction to the §7.3 census.** Git's index had
   *two* jobs: staging (compensation — dissolved) and a **stat cache**
   that avoids re-hashing unchanged trees. The second job survives as an
   implementation detail of the virtual working set: mtime/size/inode
   fingerprints so turn-finalize import-back is genuinely O(touched), not
   O(tree). Soundness hazard to model, not rediscover: a stat cache must
   never miss a same-size-same-mtime content change (git's
   racy-timestamp logic — the invariant goes in the formal plan). Upgrade
   path per jj's production experience: fs-watcher integration on
   desktop.
3. **Partial/lazy materialization — sparse-checkout's intent.** Desktop:
   full materialization stays nearly free via reflinks; laziness remains
   a later optimization there. **DO/cloud: not optional at the same
   tier** — Class-B container disks are bounded, so materialize the
   manifest *subset* the effect touches. Whip's advantage over
   sparse-checkout: for many effects **the slicer already knows the input
   closure**, so the subset is computed, not user-configured;
   fetch-on-demand covers surprises; exceeding the bound fails clearly,
   never mysteriously.
4. **Chunk-granular transfer + packing.** "Pull only missing blobs"
   extends to missing *chunks* — hybrid flows (desktop agent ↔ cloud
   workspace, handoff bundles, sidecar warm-up) become rsync-class
   incremental. Object-tier note: thousands of tiny chunk objects cost
   per-op fees/latency → **chunk packing** (pack objects indexed by the
   manifest) — packfiles reinvented at the right layer: an internal
   storage optimization, never a user-visible object.

Deployment physics, stated: **desktop** reflink matrix — APFS/btrfs/XFS
✓; ext4/NTFS have no reflink, hardlinks are safe *only* for read-only
inputs (a tool writing through one corrupts the store), so mutable
materialization falls back to copy — rare after chunking + stat cache,
but a documented cost. Cold-local-to-cloud tiering is the later
extension (enterprise-seam-shaped). **DO/cloud**: inline threshold sits
well under the platform's per-row limits; R2 takes multipart objects to
terabyte scale with zero egress fees (presigned direct-to-client delivery
of big artifacts is economically sane); platform numbers re-verified at
build like the Phase 8 facts. **Limits that stay, honestly:** in-memory
materialization bounded; binary merge remains detection + escalation;
review-grade diff on large binaries degrades to size/hash/provenance
change, stated as such; many-versioned large files still cost storage
even chunked — the retention/GC policy's job, not a new mechanism.

## 11. The evidence-grade boundary

Provenance-on-import is load-bearing for the experimentation subsystem, not
storage hygiene: it is what makes runs replayable and experiments sound when
workflow behavior flows through scripts and file I/O. Three requirements:

- **(a) Imports as content-addressed events make replay exact.** A frozen
  prefix containing "ran the test suite, imported diff `abc123`" replays by
  *serving the recorded diff* — no re-execution, no toolchain, byte-exact.
  This is the mechanism by which checkpoint-and-regenerate extends over
  POSIX at all. It constrains the import protocol: imports must be
  **atomic, recorded, and complete** (no side files escaping the diff).
- **(b) An effect taxonomy at the boundary, checked, fail-closed.**
  *Deterministic exec* (validators, builds) are **delta kernels** —
  P(output | context) is a point mass; identity = script content hash +
  environment hash + input file hashes; replayable from record, and the
  Goodhart-resistant judges of the improve effort. *Stochastic sidecar
  turns* (coding agents) are ordinary **sub-kernels whose sampled output
  includes their imported file diff**. Nothing guarantees a script that
  claims determinism actually is (clocks, network, `$RANDOM`) — so
  hermeticity is **checked empirically** (Bazel-style sampled double-run,
  compare output hashes), and anything unverified or caught flapping is
  **demoted to stochastic**: it gets kernel identity and its evidence
  partitions accordingly. Fail-closed; nothing silently poisons the ledger.
- **(c) The dependency closure grows file edges.** File-handle
  producer→consumer edges join signal/coordination/consume as the **fourth
  member of the experimentation note's dependency-closure prerequisite**
  ("Soundness hazards", item 1) — and they are
  the most precise of the four, since the content-hash handle literally
  names the artifact flowing between sites. Script content joins the
  generator closure of exec sites, exactly like prompt templates.

The payoff, and the point: **evidence transfer starts working for
scripts.** Edit a prompt and the evidence about a Python validator's
behavior carries with a certificate (its slice untouched); edit the
validator and exactly the gauges downstream of it warm-start. "Program
element" generalizes from *rule* to *rule-or-script*, and every layer above
— transfer, identification, quasi-experiments, improve — inherits the
generalization for free.

## 12. Unifications

- **One DAG.** Program lineage and state branches are the same DAG —
  `revision_epoch` was always its spine. A campaign candidate is a program
  branch plus the state branches of its evaluation runs. `suppose` runs on a
  branch; `adopt` is a merge; the improve loop's population *is* a set of
  branches.
- **The evidence ledger's *identity* needs zero changes** — it was keyed by
  content hashes precisely so evidence follows content; cross-variant
  evidence sharing was this feature ahead of time. (Corrected 2026-07-03:
  the raw ledger *row* does gain two provenance columns — `branch-ref`,
  serving §9.2's marking condition, and `execution-mode`, serving the
  experimentation note's execution-mode hazard. Neither keys pooling;
  identity stays content-derived.)
- **Restorable context is the linear special case.** "Undo" = restore a cut
  with no divergent children. The DR's consistent-cut requirement
  (transcript + event log + file manifest + coordination state, atomic) is
  inherited as the branch-point definition — extended 2026-07-03: the cut
  spans both planes (§9.3), the workspace-plane half by high-water
  positions, which monotonicity makes nearly free.

## 13. Interop — a git bridge at the boundary

Users and coding agents live in git repos today; CI and code review expect
them. The versioned workspace does not fight that: a **bridge** at the
boundary (import a repo state as a cut; export a branch as a git
branch/commit series) keeps the outside world reachable — while git is never
the engine. The bridge is deliberately lossy in the unimportant direction
(git sees snapshots and messages; it does not see cuts, certificates, or
provenance) and lossless in the important one (nothing whip needs ever
depends on git state). Bridge export is also a **source-egress door**:
source leaving the workspace becomes world-readable, so the improve
effort's proposer-leakage check hardens here (clean content-overlap
required; strict mode may refuse — improve note, "The proposer"), exactly
as it does at package export.

## 14. Scope, floor, risks

The floor is smaller than "build a VCS" suggests, because the expensive
parts exist or are in flight: the runtime-owned storage plane (DO Phases
3–5), content addressing, event sourcing, consistent cuts. Genuinely new:

- branch/manifest pointers with children (+ the virtual working set);
- the **materialization/import protocol** (§10–§11: atomic, recorded,
  complete imports; shared with the DO compute plane);
- the **certified-merge engine** — a new client of the slicer, like
  evidence transfer was;
- the **reconciliation daemon** and its cadence/policy;
- the blob-file escalation UX;
- the git bridge.

Risks, honestly: slice-certified merge is only as sound as the dependency
closure — **the same dependency-closure prerequisite as the experimentation
subsystem** ("Soundness hazards"; signal/coordination/consume/file edges),
now load-bearing for a third system;
blob-heavy coding-agent workloads get precise detection but not resolution;
reconciliation cadence is a real tuning problem (too eager = churn under
agents; too lazy = divergence). And this note must not quietly become "a
general VCS product" — the scope is whip project state, with the bridge as
the pressure valve.

## 15. Open questions

- **Merge/reconciliation residuals** (design settled in principle
  2026-07-04 — §6.1–§6.2, §7.1–§7.2): statement-level merge granularity
  (v1 = whole-declaration, fail-closed); the shared alpha-equivalence
  canonicalization ADR (one answer for merge identity and evidence
  identity); the staleness-bound default; train batching policy; whether
  LLM-assisted blob resolution ships in v1; the exact quiescence-point set
  (do long-running agents need an explicit safe-to-rebase signal?);
  workstream surface details (creation/join/leave verbs, and whether the
  user-facing name is "workstream" — interacts with the "drafts"
  bikeshed).
- **Selective-operations residuals** (design settled in principle
  2026-07-05, §7.3): the selection grammar's concrete surface (revset-
  shaped expression language — CLI/agent-tool form); the structured
  conflict-object schema; resolution-memory scope (per-workspace vs
  per-stream) and its suggestion UX; the **divergent-change presentation
  UX** (dual identity makes divergence *detected* — jj import — but
  presenting it well is jj's known-rough spot; transported-then-edited
  is otherwise resolved).
- **Per-door containment: policy settled in principle (§9.1–§9.3), now
  *derived* from §8 scope semantics + the §9.1 roles; five sub-forks
  open:** the native exec network residual (accept-and-tag vs.
  require-hermeticity — lean tag, strict mode available); telemetry
  suppress-vs-flagged-export (lean suppress); whether the synthetic
  respondent exists in v1 (lean yes for improve exploration, with the
  never-anchors constraint hard-coded); virtual-clock semantics for
  regenerated suffixes (uniform compression vs. preserving relative timing
  where races matter — interacts with the slice analysis's timing hazard);
  and whether `versioned_external` requires historical-read support or
  fingerprint-recording suffices (§9.3).
- **The store→world pump audit** (§9.2): enumerate and classify every
  store-reader that forwards outward (telemetry exporter first), before
  branches ship. Walked twice: the same enumeration yields the
  workspace-plane stores and their position markers for the two-plane cut
  (§9.3).
- **The consent surface** — **owned by `improve-design-note.md`
  ("Canary" section)**; **settled in principle 2026-07-03** there (grant
  object semantics; exposure ladder incl. the consent-free shadow
  posture; de-escalation-autonomous asymmetry; grants as workspace-plane
  events). Residual: grant/standing-policy surface syntax + segment
  carve-out vocabulary.
- **The instrument boundary's exact extent** (§9.1): gauge-declared judges
  and runtime machinery are clearly instrument-side; what marks custom
  scorers beyond gauge declarations? (Shared with the experimentation
  note's measurement-feedback question.)
- **Scheduling priority for counterfactual work** on shared rights:
  preemptible / lower-priority lease acquisition, so mass regeneration
  cannot starve production runs (§9.1's operational residual). The
  compute-plane half landed 2026-07-04: the executor pool serves
  production > working > counterfactual (`compute-plane-design-note.md`);
  the general lease-acquisition-priority question for other shared rights
  remains here.
- **Multi-device / offline**: how far mediated replication stretches before
  the reserved CRDT question reopens.
- **Retention/GC across branches — THIS NOTE IS THE OWNER** (assigned
  2026-07-03; the experimentation note's retention question converges
  here). Classification resolved by §8.4 fork 4: knowledge is
  immortal-by-default; substance lifetime = max(branch lifetime, knowledge
  roots into it); pruning referenced substance only with an honesty
  downgrade. Remaining: retention windows, pressure thresholds, and the
  tiering policy that implements the root walk.
- **Bridge scope**: one-way export first? import fidelity; monorepo subpath
  mapping.
- **Hermeticity-check policy** (§11): sampling rate for double-run checks;
  whether verified-deterministic status expires; how demotion-to-stochastic
  is surfaced to the user.
- **User-facing vocabulary** for branches ("drafts"?) — deferred bikeshed.
- **Formal-model plan** when the effort starts: the merge-safety claim
  ("certified auto-merge preserves both edits' effects") is the
  disjoint-slice composition theorem re-stated over manifests — same
  Maude/Lean discipline, likely sharing models with the evidence-transfer
  plan. Added 2026-07-04: the **confluence claim** (pairwise-disjoint ⇒
  order-independent); the essential bite = a **cross-file semantic
  conflict that text merge silently accepts and slice-overlap rejects**
  (the engine's reason to exist), plus an anti-dependence merge bite; and
  un-tie's discharged `workstream.qnt` pair (*membership-gates-autosync*,
  *archive-rehomes-members*) imported as coverage+bite candidates for the
  workstream tier. Added 2026-07-05: the **selective-undo stranding bite**
  — a file-scoped undo that a naive path filter accepts and the
  dependency-closure check rejects (a later edit read the undone writes).

## 16. Relationships

- **Restorable-context DR** — generalized by this note (linear special
  case); its consistency requirement is inherited as the branch-point
  definition.
- **Durable-object storage plane** (DR-0033, tracker) — the same substrate;
  this is its third downstream customer (after undo and the experimentation
  subsystem), and should be designed *with* it.
- **Experimentation subsystem** — regeneration containment (§9), branches as
  the home of `suppose`/candidates, evidence ledger unchanged.
- **`whip improve`** — the population is a set of branches; adopt is a
  merge; the composition theorem is shared.
- **Open-core seam** — the substrate is core-shaped (like IFC); hosted
  replication/collaboration services, if ever, sit behind the standard seam.
- **Un-tie substrate replacement**
  (`untie-substrate-replacement-research-note.md`) — this note's first
  external customer and the strongest forcing function for its build: the
  workspace replaces git in un-tie/gaugewright (13-operation mapping;
  handoff export; per-blob erasure as the honest upgrade over tombstoned
  git objects).
