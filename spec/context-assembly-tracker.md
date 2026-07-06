# Context-assembly & skills tracker — mirror pi, cache-aware compaction

**Purpose (open intent):** give the **owned (brokered) harness** a real
context-assembly layer — today it is a single hardcoded system-prompt constant —
and build the skills control plane and a pluggable, cache-aware compaction
subsystem on top of it. The observable behaviour mirrors **pi**
(`github.com/badlogic/pi-mono`, `packages/coding-agent`); the compaction design
also draws on **Codex** (`github.com/openai/codex`, `codex-rs`). This file holds
only what is *not yet true in the repo*. Reality lives in code + git + gates.

Registered in `spec/TRACKERS.md` (status: active).

Supersedes the skills sections of `spec/skills.md` and the "Skills And Context"
section of `spec/agent-harness.md` as the *build* plan (those remain the prose
spec). Companion authoring skill content stays in `spec/companion-skill.md`.

---

## Why this is one tracker, not two

Skills, project-instructions, and compaction are the same subsystem viewed from
three angles. The skills catalogue is a **context-assembly input**: it is a block
injected into the system prompt. Project-instructions (`AGENTS.md`) are another
such block. Compaction is the discipline that keeps the *assembled* context
within the window without destroying the server-side prompt cache. They all ride
one seam — the assembler — and one plane — content-addressed evidence. Building
skills forces building the assembler; the assembler is where every other bundle
lands.

---

## Current reality (the starting line)

- **Owned path** (`kind == "owned"`) is the only in-repo context assembler. Its
  entire system prompt is one constant, `OWNED_SYSTEM_PROMPT`
  (`crates/whipplescript-cli/src/harness_tools.rs`), placed verbatim; the user
  message is the raw effect `input_json`. No persona, no project files, no skills
  catalogue, no date/cwd, no env.
- **Compaction today** = `compact_context` (`harness_loop.rs`): keeps anchors
  (system + first user + last 12 messages) and **elides old tool-result bodies to
  `[elided …]` every turn**. This rewrites the middle of the prefix on every
  request → **busts the server-side prompt cache every turn.** It is
  cache-hostile and must be **deleted, not evolved** (see Phase 4).
- **Skills store plane is fully built but inert**: `skills` / `skill_attachments`
  tables, `register_skill` / `attach_skill` / `list_skills` /
  `record_skill_evidence`, `program_versions.declared_skills`, and
  `skills.injected` evidence all exist — but **nothing loads a `SKILL.md` from
  disk into the store** (every `register_skill` call is a test), and every
  production `AgentTurnExecution` passes `skill_names: &[]`, so the runtime writes
  a "no skills injected" row on every turn. `agent { skills [...] }` parses into
  IR and dead-ends at `declared_skills`. `tell … with skills [...]` is **rejected
  by the parser** ("not supported yet").
- **Delegated path** (codex/claude/pi) does its own context assembly; WhippleScript
  currently force-disables it (`setting_sources: Vec::new()` → Claude does not
  read its own `CLAUDE.md`).

---

## Decisions (settled — the constraints these phases must respect)

Locked in discussion 2026-07-04.

1. **Mirror pi's owned-harness behaviour.** The system prompt is assembled in
   pi's order and injects *nothing* pi does not: persona · one-line tool
   snippets · guidelines · doc pointers · `<project_context>` (AGENTS/CLAUDE
   files) · `<available_skills>` · `Current date` · `Current working directory`.
   **No** OS/platform, git status/branch, directory listing, model name, or any
   per-turn-volatile datum in the prompt (that alone is a cache technique — a
   volatile prompt root busts the cache at token 0 every turn). Implementation is
   flexible where that preserves WhippleScript's invariants (determinism,
   evidence, DO-portability) *without changing what the model sees*.

2. **Skills are discover-all + model-driven activation (the agentskills.io
   standard, = pi).** Every registered/visible skill's `name`+`description`(+
   location) go into the catalogue; the model **reads** the full `SKILL.md` on
   demand via an ordinary tool call. Activation is "no different to any tool
   call" — in the owned path a tool call is already record-once, evidence-logged,
   capability-gated and replayable, so a skill-body read inherits every
   determinism guarantee for free. `agent`/`turn` attachment is **provenance /
   pinning**, not a catalogue filter (catalogue stays discover-all).

3. **Skill bodies + project-instruction bytes are content-addressed in the
   store.** The `read`/activation returns identical bytes on native and on the
   durable object (no filesystem there) and replay is stable. The model sees a
   location and reads it (pi's behaviour); the bytes resolve through the registry.
   (`skills.content_hash` column already exists; add the body.)

4. **Skills never grant authority.** Frontmatter `allowed-tools` may only
   *intersect* the profile, never expand it (same rule as `with access to`).
   v1 records `allowed-tools` as provenance and does not act on it; any future
   honouring is narrowing-only. Policy decides tool availability, always.

5. **Every injected context bundle records provenance as evidence** before the
   turn (per `spec/agent-harness.md`). The assembler emits one evidence row per
   bundle (source, version, hash).

6. **Compaction is a pluggable strategy, cache-aware, staged.** A `Compactor`
   trait; strategies selected per agent/profile/config. v1 = trait + one
   strategy (turn-summarization, Codex-local shape); tool-call-result compaction
   and no-LLM hard-reset are fast-follows against the same seam. **Brute
   conversation-level truncation is not a strategy.** (Per-tool-*output* caps at
   capture time are a separate, always-on layer — see Phase 4.)

7. **Cache invariants are non-negotiable** (Phase 0 models them):
   - the assembled prefix is **append-only between compactions**;
   - a **stable cache key per turn-thread** (`prompt_cache_key` = run/effect id on
     OpenAI; `cache_control` breakpoints at stable seams on Anthropic);
   - compaction is **rare + decisive** (trigger ≈ real-usage 90% of window,
     **`BodyAfterPrefix`** scope — count only growth after the server-observed
     cached prefix — target well below the line);
   - a compaction summary is **recorded once and reused on replay**, never
     regenerated.

8. **A summarizing compactor is a recorded effect.** The summarization model call
   is a `NeedsHttp` round on the DR-0033 step machine, so it suspends/resumes on
   the durable object and is captured as evidence like any other model call.

---

## Phase 0 — Model-first invariants (coverage AND bite before code)

Per the model-first rule: prove the invariants in the formal models with both
coverage (they can hold) and bite (a negative fixture shows they can fail) before
writing Rust. **DONE 2026-07-04** — five Maude models under `models/maude/`
(+ `tests/`), all green in `scripts/check-formal-models.sh` with registered
coverage/bite counts. The full multi-turn compaction *lifecycle* (eviction/resume
across the DR-0033 step machine) remains a candidate TLA model at build time
(Phase 4); these Phase-0 models lock the safety invariants in Maude.

- [x] **Cache prefix append-only.** `models/maude/cache-prefix-append-only.maude`
  — between compactions a turn only appends (cache hit); the prefix is rewritten
  only by a triggered `[compact]`. Coverage 3 / bite 1: `cacheMiss` (mid-conversation
  invalidation without a compaction) is unreachable. This is exactly the shape the
  current per-turn `compact_context` violates.
- [x] **Compaction record-once.** `models/maude/compaction-record-once.maude` —
  one summarization records durably; resume reuses the record. Coverage 2 / bite 1:
  `regenerated` (a second summarization call on replay) is unreachable.
- [x] **Skills never grant authority.** `models/maude/skills-never-grant.maude` —
  effective authority is seeded only from the profile; a skill's `allowed-tools`
  are consumed as provenance. Coverage 2 / bite 2: an out-of-profile skill tool
  never enters `effective`, and an empty profile grants nothing.
- [x] **Catalogue determinism.** `models/maude/catalogue-determinism.maude` —
  ordered insert makes the catalogue confluent. Coverage 2 / bite 1: any discovery
  order yields the one sorted normal form; no mis-ordered catalogue is a terminal.
- [x] **Provenance completeness.** `models/maude/provenance-completeness.maude` —
  assembly is provenance-gated. Coverage 2 / bite 1: an `untracked` bundle (no
  provenance source) never reaches `assembled`.

---

## Phase 1 — The context-assembly seam (keystone)

Replace `OWNED_SYSTEM_PROMPT` with an assembler that composes the system prompt
from an ordered list of provenance-tagged bundles, each recorded as evidence.
Everything else in this tracker rides this seam.

**DONE 2026-07-06** (v0.3) — native owned harness path.

- [x] `ContextBundle { kind, provenance (source/version/hash), render() }` and an
  assembler that renders them in pi's fixed order into the system prompt.
  (`kernel/context_assembly.rs`: slot order = `BundleKind` decl order; pure,
  deterministic, content-hashed; empty-body bundles drop with no provenance row.)
- [x] Bundles for the always-on pieces: persona, one-line tool snippets (derived
  from the existing `ToolSpec`s), guidelines, `Current date`, `Current working
  directory` (`harness_tools::owned_context_bundles`). The `doc pointers`,
  `<project_context>`, and `<available_skills>` slots are populated in Phases 2–3
  (they need pi's exact text plus the skills / project-instruction stores).
- [x] Assembler emits one `context.bundle` evidence row per bundle (Decision 5).
  `run_brokered_agent_turn` records source/version/content_hash before the turn,
  exactly once (guarded on a fresh start so crash-recovery resume never duplicates).
- [x] The wire request builder (`harness_model.rs`) places the **cache breakpoint**
  at the stable seam (end of system prompt) and sends a **stable cache key** per
  turn-thread (Decision 7): Anthropic `cache_control: ephemeral` on the system
  block; OpenAI `prompt_cache_key` = the effect id. (Summary + rolling-tail
  breakpoints arrive with the Phase 4 compactor.)
- [x] Raw-`input_json`-as-user-message shortcut **left unchanged** — pi-parity does
  not require deleting it; the user message path stays the turn input.

---

## Phase 2 — Skills control plane

- [x] **Frontmatter validation** to the agentskills.io spec (v0.3) — production
  `whipplescript-store::skill_frontmatter` (dependency-free YAML subset) validates
  `name` (≤64, `[a-z0-9-]`, no leading/trailing/consecutive hyphen) + `description`
  (≤1024), parses `license`/`compatibility`/`metadata`/`allowed-tools`
  (allowed-tools as provenance only). Directory-match is the loader's check.
- [x] **Registry loader** (v0.3) — `skills_loader::load_skills_from_dir` ingests a
  skills dir (each `<name>/SKILL.md`) → `skills` rows with **content-addressed
  bodies** (`body` column via migration `0002` + the DO store's mirrored schema;
  `content_hash = H(body)`). Deterministic (sorted) load order; name==dir enforced;
  license/compatibility/allowed-tools recorded as metadata. Wired into `dev` startup
  (`load_workspace_skills`, `<store-dir>/skills/`). **Follow-on:** first-party +
  package-resource sources (only workspace `.whipplescript/skills/` today).
- [x] **Catalogue bundle** (Phase 1 seam) (v0.3) — `owned_context_bundles` renders
  the `<available_skills>` bundle (name/description/location per skill) from
  `list_skills`, only when a read-class tool is present; rides the assembler seam so
  it is a `context.bundle` evidence row. Unit test (read-tool gate) + integration
  test (workspace skill → owned turn → available_skills evidence). Exact pi XML is
  approximated; the Pi-conformance pass (un-tie Phase 3) reconciles the wording.
- [x] **Activation = registry-backed read** (v0.3) — `SqliteStore::skill_body`
  resolves a catalogue location to the registered content-addressed body; the owned
  read tool checks the skill registry first (bypassing the file-glob policy, since
  the catalogue is only offered with a read tool), so the model reads the exact
  registered bytes rather than the filesystem. The read is already recorded as a
  brokered tool-call observation → evidence. Store + executor unit tests. **DO:**
  the store method is native today; the DO read path lands with the DO agent-tool
  executor (the DO agent turn is still a no-tools stub).
- [x] **`whip skill` CLI** (v0.3) — `list` (loads workspace skills + shows the
  registry with content_hash), `validate <SKILL.md|dir>` (frontmatter check, per-
  skill ok/FAIL + exit code), `install <SKILL.md|dir>` (copies into
  `<store-dir>/skills/<name>/`, then registers into the store). Built over items
  1–2. Integration test for install→list. The `~/.codex/skills` install script is
  orthogonal (delegated Codex provider discovery) and kept.
- [x] Resolve `agent { skills [...] }` into per-turn provenance (v0.3) —
  `run_agent_effect` reads the turning agent's declared skills from the IR
  (compile the program, find the agent) and passes them as `skill_names`, so
  `skills.injected` records the pinned skills instead of always-empty. **Pinning,
  not a catalogue filter** (Decision 2) — the `<available_skills>` catalogue stays
  discover-all. Integration test. (Store-backed skill *attachments* remain a
  follow-on; today the source declaration is the pin.)

---

## Phase 3 — Project instructions (AGENTS.md / CLAUDE.md)

Mirror pi's discovery exactly, registry-backed for the DO.

- [ ] Filenames `AGENTS.md / AGENTS.MD / CLAUDE.md / CLAUDE.MD`, first match per
  directory.
- [ ] Search order: global agent dir first, then walk cwd → filesystem root
  (root-most injected first, nearest-cwd last), de-duped.
- [ ] Inject verbatim into the system prompt wrapped as
  `<project_context><project_instructions path="…">…</project_instructions></project_context>`.
  Disable flag (pi's `--no-context-files` equivalent).
- [ ] On the DO (no fs): resolve the same content from the store (content-addressed),
  so behaviour is host-uniform.

---

## Phase 4 — Compaction: the trait + strategy #1 + cache discipline

**Delete `compact_context`** (per-turn mid-prefix elision — cache-hostile). Two
distinct layers replace it:

**Layer A — per-tool-output caps at capture (deterministic, always-on, not a
"compactor").**
- [ ] Middle-truncate individual tool outputs at capture with a head+tail header
  (Codex ≈10k tokens to the model, pi ≈2000 lines / 50 KB) and a hard byte cap so a
  runaway command cannot OOM. Large outputs remain fully addressable as evidence.

**Layer B — the `Compactor` trait (conversation compaction, cache-aware).**
- [ ] `trait Compactor { should_compact(stats, window) -> bool; plan(transcript)
  -> CompactionOutcome }` where `CompactionOutcome = Deterministic(Plan) |
  NeedsModel(Request)`. `NeedsModel` runs as a `NeedsHttp` round on the DR-0033
  step machine (Decision 8).
- [ ] **Trigger** = real-usage (from `ModelReply` usage) ≈ 90% of the model
  context window, **`BodyAfterPrefix`** scope (count only growth after the
  server-observed cached prefix), hysteresis so it fires rarely and drops well
  below the line (Decision 7).
- [ ] **Strategy #1 — turn-summarization** (Codex-local shape): one recorded
  summarization effect produces a structured handoff summary; rewrite history to
  `[recent user messages ≤ ~20k tokens] + [summary]`, re-inject canonical initial
  context at the model-expected boundary. Summary recorded as an evidence
  artifact and **reused on replay** (Decision 7).
- [ ] **Apply-once + hold stable**: install the post-compaction prefix once and
  reuse it as the new stable prefix; never edit the middle on subsequent turns.
- [ ] **Overflow fallback**: on a provider context-window error mid-compaction,
  trim from the front (keep the recent suffix byte-intact) — Codex's
  cache-preserving fallback — rather than editing the middle.

---

## Phase 5 — Fast-follow strategies (same seam)

- [ ] **Tool-call-result compaction**: summarize/evict *old tool outputs* rather
  than whole turns — but **only at a compaction boundary**, never per-turn (per-turn
  = the cache bug we just deleted). WhippleScript edge: elide to a **re-expandable
  content-addressed reference** the model can pull back via a tool call (lossless
  where Codex/pi are lossy).
- [ ] **Hard-reset (no-LLM)**: Codex's token-budget mode — discard history, rebuild
  initial context only. Cheap fallback / cheap-agent strategy.
- [ ] Strategy selection surface (per agent / profile / config) + a manual
  `whip …`/`/compact`-equivalent entry point.

---

## Phase 6 — Delegated-path context (v2-ish; documented, not built first)

The native providers (codex/claude/pi) each speak agentskills.io themselves. The
owned path is the pi-mirror; delegated turns should let the provider's *own*
harness assemble context.

- [ ] Decide + implement: stop force-disabling `setting_sources` so Claude reads
  its own `AGENTS.md`/`CLAUDE.md`, and/or **materialize** attached skills into the
  provider's skill directory so its native discovery finds them.
- [ ] Reconcile provenance/evidence for provider-assembled context (we no longer
  see every bundle; record what we can).

---

## Phase 7 — `tell … with skills [...]` parse

- [ ] Build the turn-scoped `with skills [...]` parse (currently rejected). Rides
  on the `agent.tell` effect as metadata (per `spec/skills.md`), pins skills into
  the turn's provenance; does not filter the discover-all catalogue.

---

## Non-goals / out of scope

- Knowledge-"memory" folding into the turn (the `memory.query` effect is a
  separate construct; not a context-assembly input here).
- Persona-per-agent prose systems beyond pi-parity.
- Inventing a WhippleScript-native project-instructions format — we read the
  ecosystem `AGENTS.md`.
