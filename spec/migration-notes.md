# Migration Notes

Status: draft

WhippleScript previously had two legacy tracks:

- `legacy/statechart-workflows-runtime/`
- `legacy/v0.3-runtime/`

They remain in the repository for reference, but active implementation now lives
in the root Rust workspace.

## Why Legacy Systems Were Moved Aside

The legacy systems explored useful orchestration ideas, but they made it too
easy to hide control flow in host-language code or provider scripts. The v0
direction requires:

- event-sourced runtime state
- durable effects instead of inline provider calls
- explicit dependency edges through `after`
- capability/profile enforcement before provider starts
- evidence and trace export for every important transition
- formal model checks for kernel behavior

Keeping legacy folders separate avoids accidental coupling while preserving
design history.

## Migration Guidance

For a legacy workflow:

1. Identify durable facts and external effects.
2. Move provider calls into WhippleScript effects.
3. Replace source-order sequencing with explicit `after` dependencies.
4. Choose least-privilege profiles for each agent/effect.
5. Use Loft for work tracking instead of local ad hoc queues.
6. Add human review where a provider action is destructive or ambiguous.
7. Run `whip check`, model search where dependencies exist, and e2e tests.

Do not port hidden loops, shell-script schedulers, or implicit memory injection
into v0 workflows.
