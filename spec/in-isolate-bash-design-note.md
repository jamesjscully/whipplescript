# The in-isolate bash tool — Design Note (virtual interpreter tier)

**Status: DRAFT DESIGN NOTE (pre-ADR; needs research + refinement passes).**
Opened by Jack 2026-07-05: whip needs a bash tool that is safe and
compatible with execution in a Durable Object. This note captures the
design shape and the validation spike run the same day; the requirements
pass has **not** happened yet — §9 lists what it must settle. Cites the
"virtual bash for agents" initiative class (bashkit, just-bash) that
emerged in 2026.

## 1. Two problems, one mechanism

Two independent pressures point at the same design:

- **The v0 owned-harness `bash` tool is deliberately crippled** (see
  `owned-harness-tool-surface.md`): single simple command only; control
  operators, pipes, command substitution, and expansion are refused;
  redirection targets are checked literally. All of this because *real*
  bash is opaque — whip cannot see inside a subprocess, so anything it
  cannot parse conservatively it must refuse. The argv-classifier rollback
  (workflow-encapsulation course correction) is the recorded lesson:
  classifying bash *text* from outside is a losing game.
- **The DO runtime has no bash at all.** The compute plane
  (`compute-plane-design-note.md`) routes real exec to container sidecars
  (Class A/B). But the majority of agent tool calls are `grep`/`ls`/
  `cat`/`sed`-class file inspection; waking a container (1–3s cold start,
  billed-while-running) per `grep` is the wrong cost structure for the
  hot path.

The mechanism that solves both: a **virtual bash interpreter running
in-process** — commands reimplemented as builtins over a virtual
filesystem, no fork/exec by construction. Opacity disappears because whip
*executes every builtin itself*: a pipe is a data flow between two
functions we run; command substitution is a call we make. The
conservative-refusal posture and its restrictions dissolve — not because
we classify better, but because there is nothing outside our envelope to
classify.

## 2. The tier picture

The in-isolate interpreter is a **fast tier below Class A**, not a
replacement for anything:

| tier | what runs | where | hermetic? |
| --- | --- | --- | --- |
| **in-isolate bash** | file/text ops (grep, sed, awk, find, jq…) | inside the DO isolate / native harness process | by construction |
| **Class A** | real toolchains: validators, builds, tests, judges | container sidecar, warm pool | audited, fail-closed |
| **Class B** | stochastic agent turns in a materialized worktree | container-per-turn | no |

The workflow-instance DO still owns no containers; the in-isolate tier
just means most bash tool calls never emit a container effect at all.

## 3. Spike results (2026-07-05) — bashkit validated, one fixable gap

Candidate: **bashkit** (everruns, MIT, Rust, crates.io v0.12.0; ~164
builtins, pluggable VFS, resource caps, published threat model with
stable threat IDs). Spike evidence:

- **Semantics**: 10/10 on pipes/arithmetic/functions/subshells/command
  substitution; ~38/40 on a 40-command agent-invocation corpus (`jq` is
  an optional feature they ship; other misses were spike-harness bugs).
- **Custom VFS**: their `FileSystem` trait (13 async methods,
  `#[async_trait]`) implemented in ~30 min as a content-addressed
  manifest + blob store with write capture. `sed -i`/`mv`/`rm`/redirects
  produce clean write/delete deltas — exactly the
  diff-back-keyed-by-effect-id shape of the sidecar protocol.
- **Limits fail closed**: typed `LimitExceeded` on loop-iteration caps
  (fuel model: commands, loop iterations, output, fs size).
- **Escalation signal exists**: `cargo build` → exit 127 + "Compilers and
  build tools are not available in the sandbox". They have thought about
  the fidelity trap (§5).
- **Governance seams exist**: `before_exec`/`after_exec` hooks (per-
  command interception with cancel), custom `Builtin` trait (async
  execute + one-line LLM prompt hints), `NetworkAllowlist`.
- **wasm32-unknown-unknown** (our DO target): compiles (1.97 MB raw /
  633 KB gz) but **panics at runtime on `SystemTime::now()`** — 64 time
  call sites, no clock abstraction. Upstream's "WASM" claims are
  emscripten (Python wheels) and a NAPI wasip1 build currently commented
  out in their CI ("needs architectural fix to gate tokio features").
- **wasm32-wasip1**: after a 2-line cfg patch (a genuine bug —
  `std::os::unix` under `cfg(target_os = "wasi")`), the full interpreter
  **runs correctly under node:wasi**: pipes, loops, VFS writes, awk, sed,
  date, escalation signal. The interpreter core is fully wasm-portable;
  the unknown-unknown gap is exactly a clock shim plus tokio feature
  gating — mechanical.
- **Size**: composite cdylib of `whipplescript-host-do` + bashkit +
  fetchkit's converter, size-optimized (opt-z, LTO, strip, cgu=1):
  **3.15 MB raw / 1.03 MB gzipped** — ~10% of the 10 MB compressed
  Workers limit (kernel alone under the same profile: 460 KB gz).
  Size is a non-issue. Note the workspace's default release profile has
  no size flags (786 KB gz kernel); `whip deploy` needs a dedicated
  size profile.

Alternatives set aside: **just-bash** (TypeScript — would put execution
authority in the TS shell, splitting the natively-verified kernel seam),
**brush** (real bash-compat shell, but designed for a real OS; no
built-in coreutils; sandbox is not its design center), **from scratch**
(bash parser + ~150 coreutils = months of low-differentiation surface
with an endless compat tail).

## 4. The seams we own regardless of dependency

- **`WhipShell` wrapper trait**: the interpreter sits behind our own
  trait; bashkit is swappable. Its threat model is an input, not our
  foundation.
- **VFS adapter**: the store-backed `FileSystem` (manifest + blobs +
  write-capture). Two integration models — **preloaded input closure**
  (slicer-computed, same machinery as Class-A materialization; makes the
  sync-VFS-inside-sans-IO question moot) vs **demand-fetch** (VFS raises
  a blob miss → `NeedsIo`; needs suspend/resume through the interpreter).
  Preload is the simpler v1; demand-fetch is the fork to revisit for
  large manifests (ties to slicer input closures,
  versioned-workspace §10.1).
- **Policy at the hooks**: `before_exec` receives each simple command
  *post-parse, pre-execute* — the operator allow-list
  (`WHIPPLESCRIPT_HARNESS_BASH_ALLOW` analog) enforces per-command inside
  compound structures, which the v0 text-level surface could never do.
  Labels/IFC ride the store adapter (every read/write goes through our
  store API), not the interpreter.
- **The escalation contract** (§5) — entirely ours.

## 5. The fidelity trap and the escalation contract

A virtual bash can never run `cargo`, `node`, real `git`, or arbitrary
binaries. The poison failure mode is **silent stubbing** — an agent
believes it ran the tests because something exited 0. Requirement,
stated as invariant:

> Every command the in-isolate tier cannot execute faithfully MUST
> terminate with a signal distinguishable from ordinary command failure,
> and the harness MUST either escalate that invocation to the container
> tier or surface the tier boundary to the model honestly. No emulated
> stub may masquerade as the real tool.

bashkit's exit-127 + explicit sandbox message is the raw material. Open
design: **auto-escalation policy** — does an out-of-tier command
transparently re-run as a Class A/B effect (cost: a container wake the
model didn't ask for), or does the model see the boundary and choose
(cost: prompt-level complexity)? Leaning: surface the boundary in v1
(honest, cheap), revisit auto-escalation with usage data. Also open:
audit bashkit's builtin set for *quiet* divergences from real tools
(their threat-model exclusion list is the starting point) — divergence
that exits 0 is exactly the trap.

## 6. Hermeticity and the Class-A result cache

An interpreter with no syscalls and content-addressed inputs makes every
invocation a **delta kernel without the empirical hermeticity audit**:
identity = script hash + builtin-set/feature hash + input closure hashes,
memoizable in the same workspace-wide cache as Class A. Caveat — bashkit
ships non-deterministic builtins (`date`, `shuf`, `$RANDOM`): the
determinism posture is open. Options: virtual clock/seed folded into the
identity hash; or a per-invocation hermeticity bit (deterministic
builtins only → cacheable; touched `date`/`shuf` → uncached, honestly
tagged). The second is progressive-rigor-shaped and cheap; leaning that
way. (Also the fix for replay: DR-0033 replay of a recorded turn must not
re-observe wall clock.)

## 7. Where it runs — not just the DO

The same tier slots into the **native** owned harness: replace the
crippled v0 `bash` surface with the virtual interpreter + real-exec
escalation. That gives native and DO the *same* bash semantics and the
same governance story, and un-cripples pipes/substitution on native
today. The v0 restrictions (single simple command, no pipes) then apply
only to the *escalation* path (real exec), where they belong. This
supersedes the "command side-effect boundary" open item's premise for the
in-isolate tier — classification is unnecessary where whip executes the
structure itself; the item survives only for escalated real exec.

## 8. Sessions and durability (new option the interpreter opens)

v0 decided `bash` is fresh-spawn, no persistent session (DR-0024
deferral) — because OS subprocess sessions are unserializable. The
virtual interpreter changes the calculus: `Bash` state (env, cwd,
functions) plus `vfs_snapshot` are plain data. A **durable bash session
per turn** — snapshot into the store at effect boundaries, replayable
under DR-0033 — becomes cheap. Not v1 scope; recorded as the natural
follow-on that the DR-0024 deferral anticipated.

## 9. What the requirements pass must settle

1. **Consumers**: agent `bash` tool only, or also `exec` lowering for
   hermetic script sites (an `exec` that never leaves the isolate)?
2. **Escalation contract** (§5): signal shape, tier surfacing vs
   auto-escalate, the builtin divergence audit.
3. **VFS integration**: preload-closure v1 confirmation; demand-fetch
   trigger conditions.
4. **Determinism posture** (§6): hermeticity bit vs virtual clock.
5. **Caps mapping**: bashkit `ExecutionLimits`/`FsLimits` values under DO
   isolate CPU/memory budgets; who sets them (workspace config with
   progressive-rigor defaults).
6. **Network posture**: v1 = no network in the interpreter (`curl` →
   not-found/escalation); later fork: a custom `curl` builtin over the
   `NeedsHttp` effect (would make in-isolate curl governed, recorded, and
   cache-keyed — same machinery as the web tools note).
7. **Feature set**: which bashkit features on (`jq` likely yes; `python`/
   `typescript`/`sqlite`/`ssh` off in v1 — each is its own authority
   discussion).
8. **Dependency posture**: upstream engagement (clock abstraction PR +
   wasi cfg fix + tokio gating — they clearly want wasm to work; active
   project, 1,545 commits) vs temporary vendored patch; `WhipShell` trait
   boundary in either case.
9. **Naming/tool schema**: stays `bash` (familiar shape) — the tier is an
   implementation property, not a tool the model picks.

## 10. Settled vs. open

**Settled in principle (Jack, 2026-07-05):** the need (a DO-safe bash
tool); validation approach ran and passed; bashkit is the lead candidate
(dep-vs-scratch resolved *against* from-scratch by spike evidence, dep
choice pending the requirements pass).

**Open (requirements pass / ADR):** everything in §9; the
wasm32-unknown-unknown clock fix landing (upstream or fork); Class-A
cache integration timing (v1 or follow-on); durable sessions (§8,
explicitly later).

## 11. Relationships

- **`owned-harness-tool-surface.md`** — the `bash` row this replaces
  in-tier; the command side-effect boundary open item narrows to the
  escalation path (§7).
- **`compute-plane-design-note.md`** — the tiers above (§2); escalation
  lands as Class A/B effects; the result cache (§6) is its §2 cache.
- **DR-0033** — sans-IO discipline; preload closure keeps the interpreter
  synchronous between effects; recorded/replayed like any effect.
- **`versioned-workspace-research-note.md`** — the VFS *is* the workspace
  store; slicer input closures (§10.1 partial materialization) serve the
  preload; write-capture deltas import via the same certified path.
- **`context-assembly-tracker.md` / un-tie P3** — the conversational
  runtime whose tool surface needs this on the DO.
- **`information-flow-surface.md`** — labels ride the store adapter;
  interpreter output is derived data of its input closure.
