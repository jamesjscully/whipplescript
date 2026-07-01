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

| Tracker | Status | Scope (one line, non-overlapping) | Last triaged |
|---------|--------|-----------------------------------|--------------|
| `spec/distribution-tracker.md` | active | Packaging, versioning, and distribution of whipplescript artifacts | 2026-06-30 |
| `spec/expression-kernel-tracker.md` | active | The expression/evaluation kernel implementation | 2026-06-30 |
| `spec/native-provider-implementation-tracker.md` | active | Building usable native Codex/Claude/Pi providers (prose tracker) | 2026-06-30 |
| `spec/workflow-composition-transition-tracker.md` | active | Migrating to the workflow-composition model (invoke/subworkflows) | 2026-06-30 |
| `spec/decision-records/language-ergonomics-tracker.md` | active | v2 language-surface ergonomics decisions and their build-out | 2026-06-30 |
| `spec/decision-records/standard-package-design-tracker.md` | active | Open design todos for the standard packages | 2026-06-30 |
| `spec/decision-records/discriminated-families-design.md` | active | Discriminated-families design — whole tracker shipped; open questions remain | 2026-06-30 |
| `spec/review-change-plan.md` | active | Follow-ups from the 2026-06-09 review pass | 2026-06-30 |
| `spec/final-audit.md` | active | Running v0 audit log for release readiness | 2026-06-30 |
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

### Not trackers (design/spec docs that merely match the name pattern)

- `spec/control-plane.md` — control-plane specification (draft spec, not a work tracker).
- `spec/decision-records/0002-work-tracker-package.md` — ADR for the *work-tracker
  standard package*; a design record, not one of our work trackers.
