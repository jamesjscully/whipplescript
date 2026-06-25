# Owned harness — tool taxonomy (DR-0024 step 2)

Status: research note, 2026-06-24. Scoped taxonomy feeding the step-3 durability
decision in [DR-0024](decision-records/0024-owned-brokered-agent-harness.md).
Sources: OpenAI Codex CLI (`openai/codex`, `codex-rs/`), Pi
(`@mariozechner/pi-coding-agent` = `badlogic/pi-mono`, `packages/coding-agent/`),
and — for compaction reference — Claude Code. This is a classification, not a
reimplementation study. Claims are source-cited; "unverified" marks anything not
confirmed against source.

## Why this note exists

DR-0024 locks brokered execution (whip runs every tool the model requests). This
note originally framed the spine question as "*per tool, does a call become a
durable committed sub-effect or just evidence?*" — but the DR-0024 boundary
corollary answered it: **all tool calls are append-only stream events; none are
committed effects.** What the per-tool properties — **mutating?**,
**idempotent?**, **cadence** — actually drive is therefore *redaction* and the
step-4 *sandbox/governance* split, plus how the harness keeps the model's context
bounded (which separates "what's in the durable log" from "what's replayed to the
model"). This note settles those facts.

## Unified tool taxonomy

Codex and Pi converge on the same shape: a small set of read tools, a small set
of mutating tools, and one polymorphic shell tool. The last column is the
governance/redaction relevance (not durability — every row is a stream event):

```text
                         MUTATES?  IDEMPOTENT?   CADENCE   GOVERNANCE RELEVANCE
read-only
  read / view / cat        no        yes         high      evidence; read sandbox
  grep                     no        yes         high      evidence; read sandbox
  find / glob              no        yes         medium    evidence; read sandbox
  ls                       no        yes         medium    evidence; read sandbox
  view_image (codex)       no        yes         low       evidence
mutating (file)
  write (pi)               yes       yes*        low-med   write sandbox; redact?
  edit (pi)                yes       NO          med-high  write sandbox; redact?
  apply_patch (codex)      yes       NO          medium    write sandbox; redact?
polymorphic
  bash / shell / exec      depends   depends     HIGH      see "the bash problem"
control-plane
  update_plan (codex)      no        yes         low       evidence
  request_permissions      no(self)  depends     low       evidence + policy
session-stateful
  exec_command + write_    yes       NO          medium    deferred to v1
    stdin (codex PTY)                                      (session resource)
```

`*` Pi `write` is a blind full-overwrite: idempotent to the same *end state*, but
it clobbers concurrent changes. From a replay standpoint that is actually good —
one write event fully determines the file's content, unlike `edit`.

Sources: Codex tools in `codex-rs/core/src/tools/handlers/*_spec.rs` (assembly in
`spec_plan.rs`); Pi tools in `packages/coding-agent/src/core/tools/{read,write,
edit,bash,grep,find,ls}.ts`. Codex's full set is feature/model-gated (Code Mode,
multi-agent, MCP, plugins, imagegen); the table is the always-on local-CLI path.
Pi markets "four tools" but ships seven (the four above plus grep/find/ls).

## Four findings that drive step 3

**1. Cadence asymmetry is in our favor.** The high-frequency tools are almost all
read-only (read/grep/ls, and bash *used as* search). The genuinely mutating tools
— `write`, `edit`, `apply_patch` — fire at low-to-medium cadence. Since the
boundary corollary makes *every* tool call a cheap append (no per-tool commit),
event volume is never the constraint; this asymmetry just confirms the expensive
*governance* attention belongs on bash, not on the mutating file tools.

**2. The bash problem is the whole problem — but it's now a sandbox problem, not a
durability one.** `bash`/`shell`/`exec` is the only intrinsically polymorphic
tool: mutation and idempotency are *command-dependent*, unknowable from the tool
name, and it is the highest-cadence tool because it is the catch-all (build, test,
git, and ad-hoc search). The boundary corollary removed durability as a reason to
classify it (every call is just a stream event). What still wants classification
is **confinement and redaction** (step 4): which commands may run, with what
filesystem/network reach, and which command lines persist in cleartext. Precedent
to reuse: Codex already splits read-only vs escalation-needing commands via
`is_safe_command()` for its `UnlessTrusted` approval gate (`codex-rs/.../safety`).
The same classifier feeds the step-4 sandbox gate.

**3. No mutating tool is idempotent — two are anti-idempotent, and that is a
feature.** `apply_patch` (Codex) and `edit` (Pi) consume their anchor/context on
success, so re-applying *fails* ("context won't match" / "oldText not found").
This is *intended*: it is how the model self-corrects a misaligned edit — it
retries and the failure tells it "already applied." **Correction to an earlier
draft of this note:** this does *not* mean we impose commit-before-execute +
dedup per tool. Per the boundary corollary in
[DR-0024](decision-records/0024-owned-brokered-agent-harness.md), the loop
interior is exempt from workflow invariants — tool calls are append-only stream
events, not committed effects, and carry no exactly-once guarantee. The keystone
exactly-once / record-once rules apply only at the layer-3 turn boundary. Crash
recovery is **resume-from-projection** (rebuild context from the stream, continue
against the real filesystem), not per-tool retry — so non-idempotence never needs
a dedup ledger.

**4. Persistent sessions break the stateless-tool model.** Codex `exec_command`
can return a `session_id` that `write_stdin` resumes across calls/turns (a live
PTY for REPLs/long processes). That session is durable mutable state that lives
*outside* any single tool-call event. Step 3/4 needs a **session-resource**
notion for it — structurally the same as how the kernel already treats
instance-held leases/claims (see [`coordination`](decision-records/0013-coordination-package.md)),
including cleanup on every terminal path (the cancel-cleanup lesson applies).
Pi sidesteps this: its `bash` is always a fresh `spawn()`, no carried shell
state — the simpler model, and likely the right v0 default.

## Compaction model — and why it validates the architecture

The single most important cross-harness finding: **all three already separate a
complete, append-only log from a compacted, model-facing context.** That is
event-sourcing. Owning the loop does not impose a foreign structure on these
harnesses — it adopts the structure the mature ones already have.

- **Pi is closest to an event-sourced design**: the session is an append-only,
  *tree-structured* JSONL log (`id`/`parentId`, active leaf = current position).
  Compaction **appends** a `CompactionEntry { summary, firstKeptEntryId }` — it
  never deletes. Model context is *reconstructed by replay*: `buildSessionContext()`
  walks leaf→root; at a `CompactionEntry` it emits the summary, then messages from
  `firstKeptEntryId` forward. "The full message history stays in the JSONL file;
  only the in-memory context gets compacted." (`packages/coding-agent/src/core/
  compaction/compaction.ts`, pi.dev session-format docs.)
- **Codex**: durable `rollout`/`thread-store` log vs a compacted in-context
  `Vec<ResponseItem>`; resume replays the rollout.
- **Claude Code**: full JSONL transcript under `~/.claude/projects/`; compacted
  view to the model; documented two-stage eviction ("clears older tool outputs
  first, then summarizes").

All three truncate tool outputs **at record time** to a byte/line budget and keep
a bounded verbatim slice (Codex middle-truncation ~10 KB; Pi head/tail 2000 lines
/ 50 KB with the full output spilled to a temp file + a rehydration pointer;
Claude Code ~2000 lines / ~25 K tokens for reads, ~30 K chars middle for bash).

### Borrowable designs (for step 3 and the compaction step)

These map cleanly onto whip's event log:

1. **Log = source of truth; context = a derived replay projection.** Never mutate
   the log; append a compaction event and let the projector honor it. Pi's
   `buildSessionContext()` is the reference port.
2. **Compaction as an appended event with a first-kept pointer.** Store the
   model-generated summary + the boundary id of the verbatim recent window.
   Chained compactions fold prior kept spans into the next summary.
3. **Truncate tool output at record time; store the full output by reference —
   and in whip the reference is just the event id.** No temp files: rehydration
   on demand is a log read. Use middle-truncation for command output, head for
   files/search.
4. **Two-tier eviction: clear old tool *results* before summarizing.** A
   projection-time transform: replace old tool-result payloads with
   `[cleared — see event <id>]` while keeping the tool-call record (name + args);
   keep K recent tool pairs verbatim (Claude Code default K=3). Only escalate to
   model-summarization when that is insufficient.
5. **Re-inject regenerable context fresh from the log after compaction** (touched
   files, instructions, memory) rather than baking it into a lossy summary. Pi
   already accumulates touched-file paths across compactions for exactly this.
6. **Anti-thrashing guard:** if one oversized output refills the projected context
   immediately after compaction, stop and signal rather than loop.

## What this hands to step 3

> Reshaped by the [DR-0024](decision-records/0024-owned-brokered-agent-harness.md)
> boundary corollary: the loop interior is a **stream**, not committed
> sub-effects. So step 3 specifies the **event-stream + projection contract**, not
> a durability granularity. Several forks this note originally raised dissolved.

Resolved facts:
- All tool calls are append-only **stream events** (evidence-grade), regardless of
  whether they mutate. No per-tool durable commit; no exactly-once. The guaranteed
  unit is the layer-3 turn terminal.
- Read/mutating classification is clean for 6–7 tools by identity, and still
  matters — but for **redaction and step-4 sandbox/governance**, not durability.
- Mutating-tool cadence is low/medium and bash is high; since nothing is committed
  per tool, event volume is just append cost, and the old "bash durability" worry
  is gone.
- Non-idempotence is a feature (model self-correction); recovery is
  resume-from-projection, so no dedup ledger is needed.
- The log/context split is the proven, event-source-shaped pattern; whip already
  has the log half.

Open forks step 3 must decide:
- **Event types appended inside a turn**: the shape of model-request /
  tool-call / tool-result / compaction events on the stream.
- **The projection function**: how the stream derives the model context, carrying
  the borrowable compaction designs above (truncate-by-reference where the
  reference is the event id, two-tier eviction, first-kept pointer, re-inject
  regenerable context, anti-thrashing). Resume-from-projection is the recovery
  instance of this same function.
- **How the layer-3 terminal re-engages the keystone**: the turn's terminal,
  structured result, and idempotency key/fingerprint at the boundary only.
- **Per-tool redaction policy** now that whip sees tool I/O in cleartext: which
  stream events persist in cleartext vs shape-redacted.

Deferred (recorded in DR-0024, not step 3):
- **Persistent PTY sessions** — forbid in v0 (fresh-spawn shells, Pi-style).
- **Atomic-turn isolation** (worktree/snapshot rollback) — not needed for
  recovery; a later capability that composes with step-4 governance.
