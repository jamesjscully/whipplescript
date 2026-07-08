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
  that step 4 must classify for confinement/redaction. A draft design exists to
  replace this restricted surface with an in-process virtual interpreter tier
  (un-crippling pipes/substitution, DO-compatible):
  [`in-isolate-bash-design-note.md`](in-isolate-bash-design-note.md).
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
bash                      `command.run` capability + `with access to command
                          { run }` turn grant + operator allow-list
                          (`WHIPPLESCRIPT_HARNESS_BASH_ALLOW`; empty =
                          refuse all); single simple command only (control
                          operators, pipes, command substitution, backticks,
                          variable/glob/brace/tilde expansion refused);
                          literal file redirections checked against
                          file-store read/write globs (dynamic targets
                          refused); path-shaped arguments confined to the
                          workspace (absolute/`~`/`..` rejected); + counter.
                          Command-specific argv classification is NOT part
                          of this surface (see the command side-effect
                          boundary open item below).
list_todos                read; ungated
add_todo                  tracker capability; `with access to tracker { file }`;
                          profile `tracker.file` / `tracker.write`; + counter
update_todo               tracker capability; status-specific
                          `claim`/`finish`/`release` grants, or `update`/`write`;
                          profile `tracker.claim`/`tracker.finish`/
                          `tracker.release`/`tracker.update`/`tracker.write`;
                          + counter
@tool workflow            curated workflow.invoke facade; profile/required
                          capability `workflow.invoke`; cross-package
                          `invoke:<pkg>/<tool>` envelope door
```

## Open items (handed to step 3/4)

- **edit format**: Pi-style string-replace for v0 (decided). `apply_patch` as a
  possible per-model-family variant later.
- **file store construct vs. turn-scoped workspace grant**: how the coding tools'
  file access is expressed — an engineering call on technical merits in step 4.
- **command side-effect boundary**: owned-harness bash requires explicit
  `command.run` authority, the applicable profile/capability policy, and the
  operator bash allow-list before execution. It rejects shell
  control/substitution/expansion syntax, checks literal shell redirection
  targets against file-store read/write globs, and refuses obvious
  out-of-workspace path arguments. Command-specific argv classifiers are not part
  of the active v1 owned-harness surface after the classifier rollback;
  command-specific side-effect policy must be model-first and explicitly scoped
  before it is reintroduced.

- **tracker capability projection**: the general mechanism that marks a whip
  capability agent-callable and derives its tool schema from the capability's
  declared I/O types (the same projection serves `file store` and tracker
  facades). Specified in step 4 alongside the governance map.
- **web search tool** (from the 2026-07-01 v1 surface-hardening pass; canonical
  home for this item): a new `ToolSpec` alongside the profile-filtered file
  tool specs, with a `ToolExecutor`, kernel-brokered. **Owned-harness only**
  (command-backed Claude/Codex use their native web search). IFC: the query is
  an **egress** (flow-checked like `send`), the result is a **low-integrity
  ingress** (taint source, like an inbound message). A capability grantable via
  `with access to` — the first real customer of the workflow authority model (a
  subagent gets web search only if delegated it). Settled in shape; the
  network-tool policy discussion's *search* half is drafted with core
  decisions settled (Jack 2026-07-07):
  [`web-search-tool-design-note.md`](web-search-tool-design-note.md)
  (SearchProvider trait; Brave first, Exa/Tavily deferred; zero-config
  floor = model-provider-native search; scraping rejected). The web
  *fetch* tool is the remaining open gap (a fetchkit-based draft was
  withdrawn 2026-07-07 — fetch-only, no search, Jack dropped it).
- **remaining runtime-governance policy extensions** (from encapsulation
  Phase 4b; canonical home for this item): the live owned harness binds tool
  exposure and execution to turn grants, profile/registry capabilities, known
  `tell ... requires [...]` capabilities, and governance-envelope resource
  coverage. Open: governance-envelope label/argument policy beyond resource
  coverage, and future provider/tool capability mappings whose policy is not
  yet specified. Author surface stays `with access to` plus the agent
  declaration's `capabilities`/`profile`; concrete policy lives in the
  governance layer.
