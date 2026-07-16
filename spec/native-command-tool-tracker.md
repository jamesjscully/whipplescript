# Native command (real-OS `bash`) tool — tracker

Status: **active (open intent, not started)** — a preserved *option*, not current
work. Owner decision (2026-07-16): keep the door open for a native OS-command
tool, but build it **as a whipplescript tool first**; do not wire it into
GaugeDesk (or any embedder) until it exists in whipplescript.

## What this is (and is not)

- **Is:** a future tool that runs a *real OS command* (fork/exec `sh -c` or an
  argv exec) against the workspace, for tasks the in-isolate shell cannot do
  (real `git`, compilers, arbitrary binaries).
- **Is not:** the current `bash` tool. Today `bash` is served entirely by the
  in-isolate **Bashkit** virtual shell (`whip_shell::WhipShell`) over the
  governed workspace VFS — no fork/exec, no ambient filesystem/network. That is
  the default and is **not** changing (DR-0039, `spec/in-isolate-bash-design-note.md`).

## Why there's nothing to wire today (context, so it isn't re-litigated)

An earlier host-level OS-executor seam existed and was **removed 2026-07-16**
(superseded by DR-0039 Bashkit; investigation + git preserve the reference):
- `host_runtime.rs`: `NativeCommandPolicy`/`admits` (prefix allowlist),
  `CommandExecutor`/`AdmittedCommand`/`CommandExecutionOutput`, the
  `command`/`command_execution` builder, and the `validate_simple_command` /
  `simple_command_words` / `looks_path_shaped` string validators.
- `harness_tools.rs`: the `FileToolExecutor` `command_*`/`enforce_command_*`
  methods and the bashkit command-policy free functions (`command_words`,
  `*_redirection_targets`, `run_bounded_command`, …).
Those were the string-parse-in-front-of-`sh -c` approach that C9
(`spec/script-capabilities.md`) and DR-0039 rejected as insecure/decorative.
Reference impl lives in git (introduced `229945a`/`665a8b6`/`5d8ec85`, removed
in the 2026-07-16 cleanup); do not resurrect that specific design blindly.

## Open intent

- [ ] Decide the surface: a distinct whip capability/effect (sibling of the
      capability-gated `exec` effect and C9 content-pinned `exec`), or an
      extension of `exec`. The removed host seam is **not** presumed correct.
- [ ] Governance model: content-pinning (C9) and/or a real sandbox — NOT a bare
      prefix allowlist (the rejected approach). Capability-gated like `exec`.
- [ ] **DR-0036 obligation (hard requirement):** a real OS command mutates
      outside the mediated tool surface, so any host running it MUST either
      witness every effect or call `NativeWorkspaceResolver::witness_taint(...)`
      so `take_turn_witness` reports `Unwitnessed` and the receipt declines the
      workspace-cut claim honestly instead of fabricating one. `witness_taint`
      is retained (unwired) in `host_runtime.rs` precisely as that hook.
- [ ] Only after it exists + is governed in whipplescript: expose it to
      embedders (GaugeDesk). Not before.

Related: [[project-do-safe-bash-tool]] (bashkit), DR-0039, DR-0036,
`spec/script-capabilities.md` (C9), the `exec` effect (A5,
`decision-records/language-ergonomics-tracker.md`).
