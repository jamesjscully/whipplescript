# Script capabilities: content-pinned `exec` for hosted authors

Status: spec drafted 2026-06-11 from decided design
([`language-ergonomics-tracker.md`](language-ergonomics-tracker.md) C9).
Stage: spec -> modeling -> implementation + testing -> review.

## Framing

**In hosted mode, the author can never cause execution of bytes the operator
didn't pin.**

The language's pitch is "safe by default, gated escape hatches." Raw
`exec "string"` behind `WHIPPLESCRIPT_EXEC_ALLOW` is a *dev-time convenience
gate*, not a security boundary, and must be documented as such. It has two
structural holes:

1. **The allowlist matches strings, but the command runs under `sh -c`.**
   `WHIPPLESCRIPT_EXEC_ALLOW='echo *'` admits `echo hi; rm -rf ~` — the
   prefix matches, the shell does the rest. Any allowlist in front of a
   shell string is decorative.
2. **Path-based whitelisting doesn't survive agents that write files.** A
   worker agent writes `scripts/backup.sh`; the author runs
   `exec "scripts/backup.sh"`; the path is whitelisted, the content is
   attacker-chosen. Filesystem permissions don't close this: agents commonly
   share a user, and there is a TOCTOU window even when they don't.

The target deployment is an LLM authoring WhippleScript for a non-technical
user, in an environment where the authoring agent has no bash tool and
cannot write or run scripts. Hosted mode therefore replaces the command
*string* with a named, **content-pinned capability** drawn from an
operator-curated manifest. Part C shrank the residue this must cover: events,
`notify`, coordination resources, and JSON ingestion absorbed most of what
used to need glue scripts, so a small vetted standard library covers most
users.

## The three planes (threat model)

- **Operator/harness plane — trusted, user authority.** The user-run harness
  writes and edits scripts, recomputes hashes, updates the manifest, and
  drives the runtime: worker passes and heartbeats are harness cadence (the
  daemon-free design means the trusted tier owns time; a heartbeat is
  `whip notify heartbeat.tick` if the beat should be a visible typed event).
- **Authoring plane — the orchestrator `.whip`, LLM-authored.** Reacts to
  `when <agent> completed turn` and declared events; launches workers with
  `tell`/`invoke`; reaches the outside world only through *names* (manifest
  capabilities, declared agents). It cannot edit scripts because no verb in
  the language writes files — an absence, not a permission.
- **Labor plane — worker agents.** May write any bytes anywhere in their
  workspaces, including over whitelisted script files — and it doesn't
  matter: they can edit the *file* but not the *capability*. Edited bytes
  stop matching the pin and the capability fails closed, loudly (a failed
  effect with the hash mismatch in evidence — itself a routable signal).
  A compromised worker's best move is self-defeating.

The escalation chains this kills: author writes a command string (no such
surface exists in hosted mode); author directs a worker to rewrite a script
(the edit unpins it); worker poisons a script ahead of an authorized call
(same).

## Surface

```whip
exec backup_repo with r -> Report as x

after x succeeds as report { ... }
after x fails { ... }
```

- `backup_repo` is a **capability name** resolved against the operator
  manifest. In hosted mode there is no command string anywhere in source.
- `with <binding>` serializes the typed record to the script's **stdin**
  (the mirror form pinned in [`json-ingestion.md`](json-ingestion.md));
  argv comes from the manifest. No author-controlled text is ever
  interpolated into a shell — the injection surface does not exist, by
  construction rather than by filtering.
- `-> Report` / `-> each Item` type the output exactly as C3 does. Typed
  bytes in, typed facts out; no shell in between.

## The manifest

Operator config, living where provider config lives — outside every
workspace, unreachable from any agent sandbox:

```json
{
  "backup_repo": {
    "argv": ["bash", "scripts/backup.sh"],
    "sha256": "9f2c...e1",
    "env": { "BACKUP_TOKEN": "env:BACKUP_TOKEN" }
  }
}
```

- **Identity is content.** The `sha256` pins the script bytes. An update is
  an explicit operator act: edit script, re-pin, with the old and new hashes
  in the audit trail. Names stay stable while content evolves; author-pinned
  semantics, when wanted, are expressed by versioning the *name*
  (`backup_repo_v2`), never by weakening the mechanism.
- **Secrets are references** (`env:`, keychain handles), per the provider
  config model. Never values.
- **Resolution is at execution time.** The worker resolves the manifest when
  the effect runs; queued effects use the current pin; the run evidence
  records the hash that actually executed. Replay re-reads the recorded
  outcome, so replay determinism is untouched by manifest evolution.

## Enforcement

- **Check (hosted profile):** a raw `exec "string"` is a check error; a
  capability name that does not resolve in the supplied manifest is a check
  error. Dev mode keeps raw `exec` behind the env allowlist.
- **Runtime (defense in depth, since source can be compiled elsewhere):**
  the worker reads the script bytes once, verifies the hash, and executes
  the verified copy — closing the TOCTOU window between check and exec. The
  process is spawned argv-direct; there is no `sh -c` in hosted mode.
- **Policy gate (existing machinery, reused whole):** a manifest entry
  registers as capability `script.<name>`, bound per program at operator
  install. An effect naming an unregistered or unbound script blocks as
  `blocked_by_capability` — the same gate that governs every other
  capability, and the same profile machinery expresses tiering ("this
  orchestrator may invoke `script.backup_repo` but not
  `script.deploy_prod`").

## Hard exclusions and harness obligations

- **The `whip` binary is never whitelistable.** A script that shells out to
  `whip notify` or `whip revise` would let the authoring plane mint
  control-plane actions and the tier separation collapses. The native verbs
  (queue ops, `notify`, coordination) cover the legitimate cases.
- **Worker sandboxes must exclude the manifest path** (and provider config
  generally). Hash pinning makes script-file writes harmless; a manifest
  write re-pins. This is a harness contract, recorded here as one.
- **Manifest changes require explicit human confirmation, diff shown.** The
  curator in the target deployment is itself an LLM (the user's harness),
  which routinely reads worker output — so the whitelist's integrity rests
  on its injection resistance. Content pinning moves the attack from "write
  a file" to "convince the curator to re-pin it"; the human gate is the
  friction that closes the social half. Manifest changes are rare by
  design, so the friction is cheap.
- Mounting the scripts directory read-only into worker sandboxes is cheap
  belt-and-suspenders, not the load-bearing control.

## Tiers

- **Dev (laptop loop):** raw `exec "cmd"` behind `WHIPPLESCRIPT_EXEC_ALLOW`,
  documented honestly as a convenience gate. Iteration stays light; hosted
  rigor must not leak into the laptop loop.
- **Hosted:** manifest-only, enforced at check and at the worker. The
  platform ships a vetted standard library of script capabilities;
  the user's harness adds custom ones through the confirmed-update path.

## Static checks

- Hosted profile: `exec` with a quoted command string is a check error.
- An `exec` capability name absent from the manifest (when supplied at
  check time) is a check error naming the declared capabilities.
- `with <binding>` requires a typed record binding; there is no positional
  or string-interpolated argument surface.

## Out of scope (v1)

- Per-script argument schemas beyond the stdin record (mapping record fields
  to argv positions) — fast-follow; stdin covers the general case.
- Signed manifests / key distribution — hash pinning suffices single-host;
  signing arrives with multi-host distribution.
- Fetching scripts from the network.

## Dependencies

Reuses the C3 `->` output contract and the pinned `with` stdin form
([`json-ingestion.md`](json-ingestion.md)), the capability
registration/binding policy gate, the provider credential-reference model,
and the profile tiering machinery. Adds one config schema, one check-mode
flag, and the hash-verify-then-exec spawn path.

## Modeling notes

- **Fail closed:** bytes differing from the pin never execute under the
  capability name (property: mismatch → failed effect, no spawn, evidence
  records both hashes).
- **No injection:** no author-controlled string reaches a shell
  (construction: argv from manifest + typed stdin only; property test that
  no source text appears in the spawned argv).
- **Provenance:** every run records the executing hash; a trace answers
  "which version ran" for every script invocation.
- **Plane separation (the composite):** the authoring plane cannot cause
  execution of bytes the operator didn't pin — composition of fail-closed,
  no-injection, manifest unreachability, and the `whip` exclusion.
