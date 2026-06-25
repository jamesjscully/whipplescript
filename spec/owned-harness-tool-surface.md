# Owned harness — v0 agent tool surface

Status: design, 2026-06-24. Defines the model-facing tool set for the owned
brokered harness ([DR-0024](decision-records/0024-owned-brokered-agent-harness.md)).
Companion to [`owned-harness-tool-taxonomy.md`](owned-harness-tool-taxonomy.md)
(the research that grounds the coding tools).

## Principles (recap)

- **Brokered (I1):** whip executes every tool; the model requests. Each tool is a
  thin model-facing **facade over an existing governed whip effect**, not a raw OS
  call — so familiar verbs land inside the enforced envelope.
- **Familiar shapes:** tool names and field shapes match what models are trained
  on (Pi's coding tools; `TodoWrite`'s fields). We deviate only where durability
  forces it, and say so.
- **Control flow stays out (I3, refined below):** the model may *participate in
  durable shared state* (the tracker); it may not *direct the orchestration*.

The v0 surface is **10 tools**: 7 coding tools + 3 tracker tools.

## A. Coding tools (7, Pi-style)

Reproduced from the convergent Pi/Codex set; shapes follow Pi
(`@mariozechner/pi-coding-agent`). `edit` is Pi-style exact string-replace
(decided: match Pi).

```text
tool    inputs (key)                              facade / governed by
read    path, offset?, limit?                     file store (read sandbox)
write   path, content                             file store (write sandbox)
edit    path, edits:[{oldText,newText}]           file store (write sandbox)
grep    pattern, path?, glob?, literal?, limit?   file store (read sandbox)
find    pattern, path?, limit?                    file store (read sandbox)
ls      path?, limit?                             file store (read sandbox)
bash    command, timeout?                         exec capability (confinement)
```

- All seven are additionally bounded by the turn's `lease` (which workspace) and
  `counter` (budget).
- `bash` is fresh-spawn, **no persistent session** in v0 (sessions deferred,
  DR-0024). Giving `grep`/`find`/`ls`/`read` as first-class read tools diverts the
  bulk of read-style work *off* `bash`, shrinking the polymorphic-`bash` surface
  that step 4 must classify for confinement/redaction.
- The exec sandbox writable-roots and the file-store write-globs **must describe
  the same writable region** (else `bash` writes what `edit` forbids). Unifying
  that boundary is a step-4 item.

## B. Tracker tools (3) — the only durable-state surface in v0

The model participates in the durable **work tracker** (DR-0002). Use case:
*emergent discovery* — mid-task the model records a follow-up the workflow could
not have known to file, and the workflow's rules react to it independently.

**Familiar-shape decision:** fields match Claude Code's `TodoWrite` (`content`,
`status ∈ pending|in_progress|completed`). The one deviation: discrete, id'd
operations instead of TodoWrite's replace-the-whole-list — forced because the list
is *shared* with rules and other agents, so a whole-list clobber would erase their
items. We keep the familiar fields and change only what durability requires.
Tool names use `todo` for familiarity (alternative: `*_issue`); the backing store
is the issue tracker.

```jsonc
// list_todos — read current tracker items. Read-only: ungated, cheap.
input:  { "status"?: "pending" | "in_progress" | "completed" }   // optional filter
output: [ { "id": string,
            "content": string,
            "status": "pending" | "in_progress" | "completed",
            "source": "agent" | "rule" } ]            // who filed it (audit + model)

// add_todo — file a new item. Write: capability-gated + counter-budgeted.
input:  { "content": string, "status"?: "pending" }   // status defaults pending
output: { "id": string }

// update_todo — change one item's status/content. Write: gated.
input:  { "id": string,
          "status"?: "pending" | "in_progress" | "completed",
          "content"?: string }
output: { "id": string, "status": string }
```

- `activeForm` (TodoWrite's live-spinner field) is dropped — this is durable
  state, not a presentation spinner.
- `id` is the only real addition, unavoidable for discrete ops on a shared list;
  read-then-update is the flow models already use with issue trackers.
- **Facades over existing tracker effects:** `add_todo`→`file`,
  `update_todo` status transitions→`claim`/`finish`, `list_todos`→tracker query.
  No new durable mechanism — a model-facing projection of DR-0002 capabilities.

## What is *not* an agent tool, and why

The refined I3 line: the model may write **data to a durable store the
orchestration independently consults**; it may not write to the **control-flow
substrate** (the fact-base rules match on) or otherwise pick the next step.

| Durable state | v0? | Rationale |
| --- | --- | --- |
| Work tracker / to-dos | **yes** | Shared-state participation; emergent discovery; familiar shape. |
| Memory (std.memory) | defer v1 | Useful but overlaps the file tools and needs its own shape study. |
| Ledger (append-only) | defer v1 | Niche audit append; low value-to-surface. |
| Raw queue ops | folded | The tracker *is* the work surface; don't expose two. |
| `counter` / `lease` | **never** | They are the *envelope*; a model managing its own budget/locks is a governance hole. |
| Directed `signal` to instance | defer + gate | Gray-zone: data the target reacts to, but pointed enough to gate carefully. |
| `record <fact>` | **never** | Injects rule-matchable facts directly into the control-flow substrate — an I3 leak. The tracker is the sanctioned shared-state path instead. |

The mechanical distinction for the two "never"s: **facts are the substrate rules
match on**, so `record` is direct control-flow injection; the **tracker is an
external durable store** that rules choose to observe via explicit `when <tracker>
has …` readiness — participation, not direction.

## Governance gating summary

Per-tool, enforced by the envelope (the point of brokering):

```text
read / grep / find / ls   read sandbox (file store read globs); no extra gate
write / edit              write sandbox (file store write globs); + counter
bash                      exec capability + confinement; command classification
                          (step 4); + counter
list_todos                read; ungated
add_todo / update_todo    tracker capability; per-capability gate (e.g. may file
                          but not close); + counter
```

## Open items (handed to step 3/4)

- **edit format**: Pi-style string-replace for v0 (decided). `apply_patch` as a
  possible per-model-family variant later.
- **file store construct vs. turn-scoped workspace grant**: how the coding tools'
  file access is expressed — an engineering call on technical merits in step 4.
- **unified writable boundary**: exec writable-roots == file-store write-globs.
- **tracker capability projection**: the general mechanism that marks a whip
  capability agent-callable and derives its tool schema from the capability's
  declared I/O types (the same projection serves `file store` and tracker
  facades). Specified in step 4 alongside the governance map.
