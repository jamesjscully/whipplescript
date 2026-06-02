# Harness Language Topology Tracker

Goal: make provider harness topology a first-class WhippleScript source concept
so workflows can declare multiple harnesses and bind logical agents to them.

## Source Shape

```whip
harness coder: codex
harness reviewer: claude

agent implementer using coder {
  profile "repo-writer"
  capacity 1
}

agent critic using reviewer {
  profile "repo-reader"
  capacity 1
}
```

Source owns named harness topology and agent bindings. Runtime/provider config
owns credentials, adapter surfaces, local paths, workspace policy, timeouts, and
other environment-specific details.

## Implementation Tracker

- [x] Parser accepts top-level and workflow-body `harness name: kind`.
- [x] Parser accepts optional `agent name using harnessName { ... }`.
- [x] Formatter preserves harness declarations and agent bindings.
- [x] IR carries `IrHarness` declarations and `IrAgent.harness`.
- [x] Compiler validates duplicate harness names.
- [x] Compiler validates harness kind values.
- [x] Compiler validates `agent ... using harnessName` references.
- [x] Program metadata records harness topology in declared profiles/analysis.
- [x] Worker/dev dispatch derive provider selection from target agent harness.
- [x] `--provider` remains a fallback for agents without `using`.
- [x] Provider config can bind source harness ids to concrete native surfaces.
- [x] CLI status/effects/runs JSON surfaces expose harness/provider selection.
- [x] Docs explain source harness topology vs runtime provider configuration.
- [x] Example workflow demonstrates Codex implementer plus Claude reviewer.
- [x] Parser, CLI, store, and kernel tests cover compatibility and routing.

## Audit Follow-Ups

- [x] Provider config values are applied to Codex, Claude, and Pi native request
      fields after harness selection.
- [x] Worker and dev commands accept `--provider-config` and still support
      environment-provided config paths.
- [x] `status --json` includes effect and run provider-selection details.
- [x] Doctor/provider-config validation rejects command harness configs without
      launchable command settings such as `executable`.

## Compatibility Policy

Existing `agent name { ... }` declarations remain valid while this feature
lands. They continue to use the current runtime provider fallback. A later
language revision may make harness binding required for real-provider workflows
after migration docs and diagnostics are in place.
