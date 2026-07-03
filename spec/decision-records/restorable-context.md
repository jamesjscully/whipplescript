# Restorable Context (checkpointing) — decision record

**Status: PROPOSED (decided in principle 2026-07-01, Jack).** Not built.

## Problem

Provide Claude-Code-style "restore to a prior point" for whip agent work
(checkpointing / restorable context), usable by **non-technical workflow
authors**, without the costs of the obvious options.

## Why the obvious options were rejected

- **Depend on git.** Couples whip to an external, general-purpose VCS and its
  failure modes; and workflows are authored by non-technical users, so a
  git-literacy dependency is unacceptable.
- **Worktree-per-run** (current best practice). Slow and wasteful — full-tree
  copies for what is usually a few touched files.
- **OS copy-on-write** (overlayfs / btrfs / APFS). OS-dependent, non-portable,
  and fragile across the deployment targets whip must run in (incl. the
  Durable-Object / wasm target).

## Decision

"Restore context" decomposes into three things that must rewind; **two are
already solved**, so the design is narrow:

1. **Agent transcript** — already restorable (owned-harness `checkpoint`
   callback + `resume_from` projection + `sanitize_resume_messages`).
2. **Workflow / instance state** — already event-sourced; any prior point is
   reconstructable by replaying to event N. Free.
3. **File state** — the ONLY real gap, and the only reason git/worktrees ever
   entered the conversation.

**Make file state event-sourced in the runtime-owned storage plane** (the plane
the durable-object effort is already building — "one file construct with
runtime-owned tiering, trusted storage plane"). File writes go through the
whip file-store as **content-addressed** events; a **checkpoint** is an event
holding a manifest of content hashes for the (sandbox-mediated, bounded)
file-store; **restore** reverts the storage plane to that manifest.

This is neither git nor worktrees: whip-owned (no external VCS dependency),
invisible to non-technical users (a runtime "undo to before that step", not a
repo/commit/branch), and dedup-efficient (store a touched file once by hash,
reference across checkpoints — not full-tree copies), and uniform with the rest
of whip (files become "just another event-sourced resource").

## Consistency requirement (added 2026-07-01)

A checkpoint is a **consistent cut across all three planes**, not three
independent bookmarks. The checkpoint must bind together, captured at the same
quiescent point (no effect in flight straddling the cut): (1) the agent
transcript position, (2) the instance event-log index, and (3) the file-store
manifest hash. **Restore is atomic across all three.** Reverting files without
rewinding the transcript (or vice versa) yields an incoherent state the agent
would then act on — e.g. a transcript that remembers writing a file the storage
plane no longer contains, or restored files the instance's facts already
supersede. A partial restore is refused, not best-effort. The checkpoint event
in the instance log is the natural carrier of the cut: it inherently has an
event index, and it records the transcript checkpoint id plus the manifest
hash alongside it.

## Cost accepted

File content in the log is a **storage** concern. Accepted with the direction to
"be efficient and clever": content-addressing + blob-level delta/COW + the
**tiering** the DO effort already plans (cold snapshots tier out of hot storage).
Snapshot only the sandbox-mediated surface, never the whole disk. Scope stays
honest — this extends event-sourcing to the file plane whip already mediates; it
is not a re-implementation of a general VCS.

## Sequencing

**This is a slice of the runtime-owned storage plane, not a standalone checkpoint
feature.** Sequence it *with* the durable-object storage work (see that effort's
tracker) since they are the same substrate — the content-addressed, tiered,
event-referenced file store IS the checkpoint mechanism.

## Scope note

Jack's clarified intent is **(c) restorable agent context** (rewind files +
transcript to a prior point), NOT (b) instance branch/fork of alternate
continuations. Branching/forking a workflow instance is a separate, larger
feature and is out of scope here.
