# Harness Language Topology Tracker

Status: superseded target vocabulary.

This tracker records an implementation-era harness topology experiment. The
package-system target uses ordinary `provider` clauses on agent declarations and
treats harnesses as implementation/provider-binding details. New package specs
should not use header-level agent route syntax.

## Source Shape

```whip
agent implementer {
  provider coder
  profile "repo-writer"
  capacity 1
}

agent critic {
  provider reviewer
  profile "repo-reader"
  capacity 1
}
```

`coder` and `reviewer` are logical provider bindings supplied by package/operator
configuration. Runtime/provider config owns credentials, adapter surfaces, local
paths, workspace policy, timeouts, and other environment-specific details.

## Implementation Tracker

- [x] Parser accepted top-level and workflow-body harness declarations in the
      implementation-era experiment.
- [x] Parser accepted optional agent header route bindings in the
      implementation-era experiment.
- [x] Formatter preserves harness declarations and agent bindings.
- [x] IR carries `IrHarness` declarations and `IrAgent.harness`.
- [x] Compiler validates duplicate harness names.
- [x] Compiler validates harness kind values.
- [x] Compiler validated agent header route references in the
      implementation-era experiment.
- [x] Program metadata records harness topology in declared profiles/analysis.
- [x] Worker/dev dispatch derive provider selection from target agent harness.
- [x] `--provider` remains a fallback for agents without explicit source
      provider bindings.
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
