# std/grammars — grammar-only declaration_block manifests

These are `whipplescript.package_manifest.v0` files that carry **only** the
`declaration_block` (Shape 1, DR-0011) construct grammar for the
declaration-family std packages — `tracker`, `channel`, `counter`, `lease`,
`ledger`, `file store`, and `memory pool`. They have no `effect_contracts`,
`capabilities`, `providers`, `profiles`, or `bindings`; they exist purely as the
single source of the top-level declaration parse grammar.

## What reads them

Only `crates/whipplescript-parser/build.rs`. It transcribes each
`declaration_block` construct's `grammar` object into one `DeclarationBlockSpec`
row of the compiled-in `DECLARATION_BLOCK_GRAMMAR` table (mirroring how
`std/manifests/*.json` feeds `EFFECT_OPERATION_GRAMMAR`). A malformed manifest
here fails the build, naming the manifest and the problem.

## Why they are NOT under std/manifests/ (blocker B1)

`scripts/artifact_admission.py` globs `std/manifests/*.json` indiscriminately to
compute the embedded-std construct identities the authorability door treats as
privileged. The Rust authority
(`registry_construct_is_embedded_std_copy` / `EMBEDDED_STD_MANIFESTS`) instead
reads a hardcoded list. A grammar-only manifest carrying `declaration_block`
constructs, if placed under `std/manifests/`, would be treated as embedded-std
by Python (fail-open) but not by Rust — defeating the door for the migrated
families. Keeping them in this SEPARATE directory means:

- the door glob never sees them (they are **not** door-privileged), and
- they are **not** embedded in the CLI (`EMBEDDED_STD_MANIFESTS`), so no digest
  churn and no `embedded_std_manifests()` panic on an unsupported construct.

A conformance test (`std_manifests_all_embedded`, in the CLI crate) asserts that
every construct-bearing manifest under `std/manifests/` is embedded, so a
non-embedded construct manifest cannot lurk there and re-open the fail-open gap.
