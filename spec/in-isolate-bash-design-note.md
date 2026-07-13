# The in-isolate bash tool — Design Note (virtual interpreter tier)

**Status: research record; direction accepted by
[DR-0039](decision-records/0039-bashkit-default-bash.md).**
Opened by Jack 2026-07-05: whip needs a bash tool that is safe and
compatible with execution in a Durable Object. This note captures the
design shape, the validation spike run the same day, and the implementation
landed with DR-0039. Cites the
"virtual bash for agents" initiative class (bashkit, just-bash) that
emerged in 2026.

## 1. Two problems, one mechanism

Two independent pressures point at the same design:

- **Before DR-0039 the v0 owned-harness `bash` tool was deliberately crippled** (see
  `owned-harness-tool-surface.md`): single simple command only; control
  operators, pipes, command substitution, and expansion are refused;
  redirection targets are checked literally. All of this because *real*
  bash is opaque — whip cannot see inside a subprocess, so anything it
  cannot parse conservatively it must refuse. The argv-classifier rollback
  (workflow-encapsulation course correction) is the recorded lesson:
  classifying bash *text* from outside is a losing game.
- **Before DR-0039 the DO runtime had no bash at all.** The compute plane
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

Candidate: **bashkit** (everruns, MIT, Rust; validated at v0.12.0 and integrated
at pinned v0.13.0; ~164
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
- **wasm32-unknown-unknown** (our DO target): v0.12.0 exposed a clock/tokio
  incompatibility. Bashkit v0.13.0 added the web-time/gated-runtime support;
  the combined WhippleScript DO host now builds for the target and executes
  the shared conformance scripts without a fork or vendored patch.
- **wasm32-wasip1**: after a 2-line cfg patch (a genuine bug —
  `std::os::unix` under `cfg(target_os = "wasi")`), the full interpreter
  **runs correctly under node:wasi**: pipes, loops, VFS writes, awk, sed,
  date, escalation signal. The interpreter core is fully wasm-portable;
  the unknown-unknown gap is exactly a clock shim plus tokio feature
  gating — mechanical.
- **Size**: composite cdylib of `whipplescript-host-do` + bashkit
  (plus a ~32 KB-gz html-converter probe, since dropped), size-optimized
  (opt-z, LTO, strip, cgu=1):
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
- **Policy at the boundary**: package `command.run` admission determines whether
  `bash` is available. The interpreter receives only the admitted workspace
  snapshot and imports a fully validated delta. There is no second ambient
  command allowlist and no OS-exec fallback hidden inside `bash`.
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
design was settled by DR-0039: the model sees the boundary and a real non-bash
operation must request an explicitly brokered capability. There is no automatic
real-exec escalation. Still open:
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
crippled v0 `bash` surface with the virtual interpreter. That gives native and DO the *same* bash semantics and the
same governance story, and un-cripples pipes/substitution on native
today. The v0 restrictions (single simple command, no pipes) then apply
no longer define a second path. This supersedes the "command side-effect
boundary" open item's premise for the in-isolate tier — classification is
unnecessary where whip executes the structure itself. Real toolchains are
separate named brokered capabilities.

## 8. Sessions and durability (new option the interpreter opens)

v0 decided `bash` is fresh-spawn, no persistent session (DR-0024
deferral) — because OS subprocess sessions are unserializable. The
virtual interpreter changes the calculus: `Bash` state (env, cwd,
functions) plus `vfs_snapshot` are plain data. A **durable bash session
per turn** — snapshot into the store at effect boundaries, replayable
under DR-0033 — becomes cheap. Not v1 scope; recorded as the natural
follow-on that the DR-0024 deferral anticipated.

## 9. Implementation choices under DR-0039

1. **Consumers**: implemented for the native owned harness, native governed
   host, and DO governed host. Other `exec` lowering remains separate.
2. **Unsupported-command signal** (§5): surface the Bashkit boundary honestly;
   DR-0039 rejects automatic real-exec escalation. Audit builtin divergences.
3. **VFS integration**: preload-closure v1 is implemented; demand-fetch remains
   a future large-workspace optimization.
4. **Determinism posture** (§6): v1 uses a fixed Unix epoch and no ambient
   randomness or network.
5. **Caps mapping**: v1 fixes 32 MiB workspace, 8 MiB per file, 5,000 files,
   1 MiB output, and a caller-bounded timeout. Configuration remains open.
6. **Network posture**: v1 = no network in the interpreter (`curl` →
   not-found/escalation); later fork: a custom `curl` builtin over the
   `NeedsHttp` effect (would make in-isolate curl governed, recorded, and
   cache-keyed).
7. **Feature set**: `jq` is on; `python`/`typescript`/`sqlite`/`ssh` are off.
8. **Dependency posture**: pinned upstream v0.13.0 behind WhippleScript's
   `WhipShell` wrapper; no vendored patch.
9. **Naming/tool schema**: stays `bash` (familiar shape) — the tier is an
   implementation property, not a tool the model picks.

## 10. Settled vs. open

**Settled by DR-0039 (Jack, 2026-07-13):** Bashkit is the default governed
`bash` for both native and DO managed harnesses; non-bash capabilities are
brokered explicitly; unsupported Bashkit behavior surfaces honestly rather than
auto-escalating.

**Implemented 2026-07-13:** the shared `WhipShell` wrapper, preloaded VFS,
fixed clock, limits, `jq`, honest unsupported-command failure, and native/DO
adapters. **Still open:** configurable caps, Class-A cache integration, builtin
fidelity auditing, and durable sessions (§8, explicitly later).

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
