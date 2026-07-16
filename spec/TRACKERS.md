# Trackers — the registry (single source of truth)

This file is the **one place** that says which trackers exist, what each one
covers, and whether it is live. If a tracker is not listed here, it does not
count — `scripts/check-trackers.sh` fails the gate until it is registered. Read
this before opening a new tracker: if your work fits an existing scope, add it
there instead of starting a duplicate.

## The rule that keeps this honest

**A tracker holds only OPEN INTENT. Reality lives in code + git + gates.**

- The moment an item is *true in the repo*, it stops being a `[ ]` and becomes a
  `[x]` that cites its evidence (commit SHA / test name / PR), then it is eligible
  to leave. Done items are archived, not accumulated — so nothing reads as "todo"
  once it ships.
- Don't track what the repo already records (that a function exists, that a test
  passes, git history). Track the decision or intent that isn't true *yet*.
- One concern → one tracker. Overlap is visible here, so it gets caught here.

## Item vocabulary

| Mark | Meaning | Requirement |
|------|---------|-------------|
| `[ ]` | open | not started; will be triaged into a bucket |
| `[~]` | in progress **or** deferred-with-cause | must carry a *why* and *when* inline |
| `[x]` | done | must cite evidence (commit / test / PR) |

## Tracker lifecycle (status column)

- **active** — being worked or genuinely queued. Has open/`[~]` items (or is prose).
- **closed** — everything shipped; kept as a historical record. No open items.
- **archived** — parked, not being worked; **re-surface on demand** by pulling any
  still-relevant item into an `active` tracker. Physically under `spec/archive/`
  when nothing links to it; kept in place with an ARCHIVED banner when it has
  inbound links (so links don't break).

## Triage cadence

New raw items land under an `## Inbox` section in the most relevant active tracker,
then get moved to `now` / `next` / `later` / `wontfix`. Keep the number of
`active` trackers small; when one goes all-`[x]`, close it. The gate below is the
forcing function — it flags drift so triage is on-touch, not a periodic slog.

## What the gate enforces (`scripts/check-trackers.sh`, in the readiness gate)

**Hard (blocks):** every discovered tracker file is registered here; every row's
file exists; status ∈ {active, closed, archived}; no file listed twice.
**Warn (the triage worklist):** an `active` tracker whose checkboxes are all done
(close it); a `closed` tracker with open `[ ]` items (resolve or reopen); an
`archived` file still outside `spec/archive/` (informational). Legend sections and
radio-style choice lists are excluded from checkbox counting.

---

## Registry

**Release** targets (tagged 2026-07-05; plan = [[project-release-plan]] memory):
**v0.2** language + std packages + native runtime (polished) · **v0.3** cloud
deployment + owned harness · **v0.4** improve/evals + version control. Blank =
closed/archived (shipped or historical).

| Tracker | Status | Scope (one line, non-overlapping) | Last triaged | Release |
|---------|--------|-----------------------------------|--------------|---------|
| `spec/distribution-tracker.md` | active | Packaging, versioning, and distribution of whipplescript artifacts | 2026-06-30 | v0.2 |
| `spec/expression-kernel-tracker.md` | active | The expression/evaluation kernel implementation | 2026-06-30 | v0.2 |
| `spec/native-provider-implementation-tracker.md` | active | Building usable native Codex/Claude providers (prose tracker) | 2026-06-30 | v0.2 |
| `spec/workflow-composition-transition-tracker.md` | active | Migrating to the workflow-composition model (invoke/subworkflows) | 2026-06-30 | v0.2 |
| `spec/workflow-encapsulation-implementation-tracker.md` | closed | v1 workflow-encapsulation + invocation-authorization theorem — all phases shipped 2026-07-02; open remnants re-homed to owned-harness-tool-surface.md | 2026-07-02 |
| `spec/durable-object-runtime-tracker.md` | active | Sans-IO async refactor + Cloudflare Durable Object host binding (run whip in a wasm isolate) | 2026-07-01 | v0.3 |
| `spec/context-assembly-tracker.md` | closed | Owned-harness context assembly (mirror pi): system-prompt seam, skills registry/catalogue, project-instructions, and pluggable cache-aware compaction | 2026-07-04 | v0.3 |
| `spec/experimentation-improve-tracker.md` | active | Experimentation/improve subsystem (the v0.4 banner's second half): release-scope decision + the four open design passes + the gauge/mark/campaign build — registered 2026-07-10, previously pre-ADR notes | 2026-07-10 | v0.4 (lean: re-cut → v0.5) |
| `spec/decision-records/language-ergonomics-tracker.md` | closed | v2 language-surface ergonomics decisions + build-out — **CLOSED 2026-07-16**, all rows shipped: Part B reconciled 2026-07-01; B1g closed 07-02; `consume` removed + B1b–f re-verified + bindingless-guard fix + C9/row 10/18 closed 07-16; B1a lowering-move completed by the DO sans-IO refactor (DR-0033 chunk 1b) | 2026-07-16 | v0.2 |
| `spec/decision-records/standard-package-design-tracker.md` | closed | Standard-package design campaign — all 13 rows v1 BUILT, Current Rule satisfied 2026-07-15 (171a9e0); open tails re-homed to the post-campaign tracker | 2026-07-15 | |
| `spec/std-package-post-campaign-tracker.md` | active | Sequenced post-campaign std-package tails: Waves 0-3 buildable, demand-gated triggers, design-heavy items awaiting rulings | 2026-07-15 | v0.4+ |
| `spec/tracker-phase-b-tracker.md` | active | std.tracker phase B — the DAG/conflict campaign (ADR-0002 phase B, hard-core-first): content-hash event DAG, per-field merge conflicts, resolution, state-tokens | 2026-07-15 | v0.4+ |
| `spec/native-command-tool-tracker.md` | active | Native (real-OS) command `bash` tool — a preserved OPTION (not started): build as a whipplescript tool first, DR-0036-witnessed, capability-gated; the old host OS-executor seam was removed 2026-07-16 (superseded by DR-0039 Bashkit) | 2026-07-16 | later |
| `spec/decision-records/discriminated-families-design.md` | closed | Discriminated-families — all four families (A/B/C) + selector capstone + §5.4 observer-only check SHIPPED & gated; closed 2026-07-03. Sole non-`[x]`: the Stage 1a internal pass-collapse refactor, a deferred-with-cause `[~]` (dropped Rev 2026-06-28e, no capability lost). §9 = demand-gated / v2 design questions, not open build intent. | 2026-07-03 |
| `spec/review-change-plan.md` | closed | 2026-06-09 review pass — shipped; remaining follow-ups folded into language-ergonomics (dedup) | 2026-07-01 |
| `spec/final-audit.md` | active | Running v0 audit log for release readiness | 2026-06-30 | v0.2 |
| `spec/real-provider-validation-tracker.md` | closed | Real (live) provider validation — all v0 items shipped | 2026-06-30 |
| `spec/workflow-revision-transition-tracker.md` | closed | Workflow-revision transition — post-audit hardening complete | 2026-06-30 |
| `spec/decision-records/information-flow-implementation-tracker.md` | closed | IFC build tracker — reconciled + closed (6 deferrals-with-cause) | 2026-06-30 |
| `spec/decision-records/information-flow-audit-findings.md` | closed | IFC audit findings + 6-wave fix plan — Waves 0–6 done | 2026-06-30 |
| `spec/implementation-plan.md` | closed | The original phased implementation plan — historical record (477 done) | 2026-06-30 |
| `spec/implementation-plan-phase-review-tracker.md` | closed | Validation that the implementation-plan phases were reviewed | 2026-06-30 |
| `spec/gherkin-lessons-tracker.md` | closed | Lessons/acceptance from the Gherkin pass — v0 complete | 2026-06-30 |
| `spec/one-way-language-cleanup-tracker.md` | closed | One-way (no-back-compat) language cleanups — complete | 2026-06-30 |
| `spec/documentation-improvement-tracker.md` | closed | Documentation improvement pass — completed | 2026-06-30 |
| `spec/workflow-revision-followups-tracker.md` | archived | Non-blocking vNext follow-ups — bankruptcy 2026-06-30, re-surface on demand | 2026-06-30 |
| `spec/language-ergonomics-tracker.md` | archived | Redirect stub → `decision-records/language-ergonomics-tracker.md` | 2026-06-30 |
| `spec/archive/harness-language-topology-tracker.md` | archived | Superseded harness/language topology vocabulary | 2026-06-30 |

## Known overlaps (dedup — audited 2026-07-01)

Same concern tracked in more than one place. Registry rule: work it in the
**canonical home**; the others carry a pointer, not a live checkbox.

| Concern | Canonical home | Also appears in |
|---------|----------------|-----------------|
| Move rule-body lowering out of the CLI crate | language-ergonomics B1a | review-change-plan §4.11 (folded) |
| Remove `consume` after deprecation | language-ergonomics B3 | review-change-plan (folded) |
| Dynamic rule-coverage CI (per-run committed rules) | language-ergonomics B3 | review-change-plan (folded); static lint already shipped |
| Agent-turn enrichment / drop `AgentTurn.issue`+`changedFiles` | language-ergonomics A3e/B2 | review-change-plan (folded) |
| Full expression-kernel guard typing | expression-kernel-tracker | final-audit G-002/G-010 (points here) |
| Native-provider live validation | native-provider-implementation-tracker | final-audit G-006/G-008 (points here) |
| Provider failure ≠ auto-fail workflow | workflow-composition-transition (done) | final-audit distributed-systems / G-009 |
| C6/C7/C8/C9 capabilities (coord/messaging/telemetry/script/time/ingress) | shipped in core (language-ergonomics) | standard-package-design treats as future *packages* — that tracker is now a packaging/namespacing design question, not feature work |
| `case`/sum-types/coerce→enum narrowing | discriminated-families-design (closed, shipped) | language-ergonomics C1; expression-kernel tagged-terminal rows |
| vNext epic (retarget / fact migration / cancellation depth / destructive confirms) | final-audit (deferred log) = archived `workflow-revision-followups` | native-provider NP-060 builds only the cancellation request/ack model, not out-of-band depth |

## Not trackers (design/spec docs that merely match the name pattern)

- `spec/control-plane.md` — control-plane specification (draft spec, not a work tracker).
- `spec/decision-records/0002-work-tracker-package.md` — ADR for the *work-tracker
  standard package*; a design record, not one of our work trackers.
- `spec/std-tracker.md` — concrete package design for the `std.tracker` standard
  package (issues/claims surface); a package-contract spec, not a work tracker.
