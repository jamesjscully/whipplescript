# 0012: Plugin System Retirement Cleanup

Status: cleanup tracker

## Current Direction

WhippleScript should not have a separate author-facing plugin system. Extension
should be explained through three concepts:

```text
package   locked installable distribution unit
library   compile-time source meaning, construct contracts, and effect contracts
provider  runtime implementation behind durable effects
```

The old plugin system remains only as implementation substrate and compatibility
vocabulary while code is renamed. Runtime provider registration is still needed;
a public plugin extension path is not.

The active design sources are:

```text
0006-libraries-packages-providers-and-exec.md
0010-package-library-provider-boundary.md
0011-controlled-library-grammar-extensions.md
../construct-grammar.md
../construct-graph-calculus.md
../construct-lowering-preservation.md
```

## Spec Cleanup Done In This Pass

```text
0006 now marks plugin as legacy/internal runtime registration vocabulary
0010 now states plugin is not a fourth extension layer
plugin-system.md is demoted to a legacy runtime-provider registry note
plugin-author-guide.md says it is a package guide kept at an old path
spec README no longer describes memory or the runtime registry as the
  active plugin model
effects/control-plane specs now say package capability/provider registration
  instead of plugin capability as the language concept
```

## Stale Implementation Names

These are implementation cleanup targets, not new design decisions.

| Area | Current stale shape | Desired direction | First files to inspect |
| --- | --- | --- | --- |
| Source imports | Done: short `use memory` is represented internally as `IrUseKind::Package`, and snapshots continue to use package/library wording. | Keep `use plugin ...` rejection only as a removed-syntax diagnostic. | `crates/whipplescript-parser/src/lib.rs` |
| Runtime registration store | Done: Rust-facing registration fields and SQLite table/column names now use package terminology. | Keep legacy `plugin_id` only as an input alias while compatibility manifests exist. | `crates/whipplescript-store/migrations/0001_runtime_store.sql`, `crates/whipplescript-store/src/lib.rs` |
| Store API | Done: public Rust API now has `PackageRegistration`, `register_package`, `register_package_manifest`, and `load_package_manifests_from_dir`; plugin-named wrappers were removed. | Keep new callers on package APIs. | `crates/whipplescript-store/src/lib.rs`, `docs/rust-api.md` |
| CLI lock loading | Done: package-lock runtime loading calls `store.register_package_manifest(...)`. | Keep package-lock loading on package/provider APIs. | `crates/whipplescript-cli/src/main.rs` |
| Built-in capability call provider | Done for fresh stores: migration seed uses `builtin-package-call`. | Keep provider seed wording aligned with package capability calls. | `crates/whipplescript-store/migrations/0001_runtime_store.sql` |
| Compatibility manifests | Done: plugin-shaped compatibility manifests live under `examples/legacy-plugin-manifests/` with explicit legacy package/provider names. | Retire after first-class package manifests cover the same runtime cases. | `examples/legacy-plugin-manifests/`, `crates/whipplescript-store/src/lib.rs` tests |
| Example file names | Done: package-backed memory examples are `examples/package-memory.whip` and `examples/package-memory.ir`, and report scripts/docs point at those names. | Keep generated report fixture names aligned with package examples. | `examples/package-memory.whip`, `examples/package-memory.ir`, `scripts/check-report-schemas.sh`, `scripts/check-formal-models.sh` |
| Test names | Mostly done: active parser, store, CLI, and kernel behavior now uses package/provider wording, with remaining plugin terms limited to removed-syntax diagnostics or explicit legacy fixtures. | Continue renaming opportunistically when touching old tests. | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-store/src/lib.rs`, `crates/whipplescript-kernel/tests/e2e.rs`, `crates/whipplescript-cli/tests/control_plane.rs` |
| Core integration extraction | Loft, schema coercion, and memory concepts still have hard-coded parser/kernel/store/CLI paths in places that may become standard packages or providers. | Classify each hard-coded surface against construct families and lowering classes before extraction. Do not move runtime lifecycle semantics into packages. | `crates/whipplescript-parser/src/lib.rs`, `crates/whipplescript-cli/src/main.rs`, `crates/whipplescript-kernel/src/lib.rs`, `crates/whipplescript-store/migrations/0001_runtime_store.sql` |

## Remaining Historical Or Compatibility Names

The following references are intentionally left for a later coordinated rename
because they are tied to fixtures, generated reports, or historical trackers:

```text
models/maude/README.md
examples/legacy-plugin-manifests/
spec/memory-plugin.md
spec/implementation-plan.md
spec/final-audit.md
```

## Cleanup Order

1. Rename public prose from plugin to package/library/provider where the meaning
   is already settled.
2. Done: rename examples and generated report fixtures together so report checks
   stay stable.
3. Done: add package/provider registration API names and remove plugin-named
   store helpers.
4. Done: rename fresh-store schema names to package terminology.
5. Reclassify hard-coded core effects one standard package at a time against the
   construct graph and lowering-class model.
