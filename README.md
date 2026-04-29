# armature

Armature is a lightweight local daemon and CLI for running ordinary user-authored
programs in response to schedules, file changes, emitted events, and long-running
process sources.

## Workspace layout

```text
crates/
  armature-cli/
  armature-core/
  armature-daemon/
packages/
  sdk/
```

The repository starts with a narrow foundation slice:

- `armature-core` holds shared domain types, ID helpers, and error conventions.
- `armature-daemon` holds the daemon boundary and runtime shell.
- `armature-cli` holds the command tree and executable entry point.
- `packages/sdk` holds the TypeScript SDK surface for user-space helpers.
