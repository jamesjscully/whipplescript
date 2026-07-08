# Package Management

Status: implementation-grade target for local package management v0

This spec defines the next package-management layer above the existing package
manifest and package lock machinery. It intentionally does not define a public
package registry or remote dependency resolver.

## Goal

The current system can validate local package manifests, produce a package lock,
and run workflows against that lock. The missing piece is a project-level intent
file and a reproducible sync workflow that works in CI and across developer
machines.

The v0 package manager should provide:

```text
whip.packages.json  project intent: which package sources this repo uses
whip.lock           exact realized package set: ids, versions, hashes, sources
whip package sync   validate package intent and write/update whip.lock
lock discovery      check/compile/dev/run/worker load whip.lock by default
```

The package manager must not grant runtime authority. It makes package contracts
available. Provider config, credentials, profiles, capability bindings, and
runtime policy remain separate gates.

## Non-Goals

These are deferred, not accidentally omitted:

```text
package registry
package publish/install/update/uninstall
semantic-version dependency resolution
transitive dependency graphs
remote git/http package sources
package cache
provider binary or sidecar installation
native artifact signing and trust policy
workspace environment management
provider credential management
automatic provider binding
package migrations
multi-root workspace package graphs
```

## Terms

package manifest
  A `whipplescript.package_manifest.v0` JSON file describing one package's
  libraries, capabilities, providers, profiles, bindings, and construct
  contracts. A construct declares its parse shape as a DR-0011 `grammar`
  object (spec/construct-grammar.md "Two-Shape Meta-Grammar": ordered slots
  with optional connectives from {`from`, `for`, `into`, `to`, `via`}, an
  optional payload block, a binding mode, and the target capability) —
  `grammar` replaces the older flat `fields[]` array, and the flat view is
  now derived from it (slots, then payload fields, then the binding).
  Declaring both is rejected; `fields[]` alone remains accepted for
  grammar-less constructs. Only `effect_operation` grammars are
  manifest-expressible today, and only the embedded std manifests
  (`std/manifests/`) feed the parser's grammar table.

package set
  A project intent file named `whip.packages.json`. It lists the package sources
  this repository wants to use.

package lock
  A deterministic, checked realization of the package set, named `whip.lock` by
  default. It records exact package identity, version, source, and manifest hash.

package sync
  The operation that reads the package set, validates referenced manifests,
  checks package-contract consistency, and writes the lock.

project root
  The directory containing the package set or lock. For `sync`, this is the
  directory containing the selected package set. For lock discovery, it is the
  nearest ancestor that contains `whip.lock`, unless the user passes
  `--package-lock`.

## Package Set Schema

The v0 project intent file is JSON:

```json
{
  "schema": "whipplescript.package_set.v0",
  "packages": [
    {
      "name": "notes",
      "source": {
        "type": "path",
        "path": "examples/packages/notes.json"
      }
    }
  ]
}
```

Fields:

```text
schema
  Must be exactly whipplescript.package_set.v0.

packages
  Required array. Empty arrays are valid but should produce an empty lock.

packages[].name
  Required import/library name expected from the manifest. The referenced
  manifest must expose a package with this name. This keeps source intent
  readable and catches accidental path swaps.

packages[].package_id
  Optional exact expected package id. If present, the manifest package_id must
  match.

packages[].version
  Optional exact expected version. This is not semver resolution; it is an exact
  assertion against the manifest version.

packages[].source.type
  Required. v0 supports only path.

packages[].source.path
  Required project-relative manifest path. v0 sync rejects absolute paths,
  empty paths, paths containing `..`, and paths that escape the project root
  after symlink/canonical path resolution.
```

Package set entries are not provider authority grants. They only select package
manifests whose contracts may appear in the lock.

## Lock Target Shape

`whip.lock` should remain JSON with schema
`whipplescript.package_lock.v0`, but the target v0 shape should record portable
sources instead of absolute manifest paths:

```json
{
  "schema": "whipplescript.package_lock.v0",
  "packages": [
    {
      "package_id": "package-notes",
      "name": "notes",
      "version": "0.1.0",
      "source": {
        "type": "path",
        "path": "examples/packages/notes.json"
      },
      "manifest_sha256": "..."
    }
  ]
}
```

Lock rules:

- Sort packages by `name`, then `package_id`.
- Store project-relative source paths using `/` separators in `source.path`.
- Do not write absolute paths for normal project-local packages.
- Compute `manifest_sha256` over the exact UTF-8 manifest bytes read from disk.
- On load, resolve relative source paths relative to the lock file directory.
- On load, re-read each manifest and reject identity, version, or hash mismatch.
- On load, apply the same path-escape check as `sync`: reject absolute paths,
  `..` segments, and paths that escape the lock directory after symlink/canonical
  resolution. Load-time enforcement must match sync-time enforcement so a crafted
  lock cannot widen file access.
- Reject duplicate `name`, duplicate `package_id`, duplicate source path, and
  ambiguous library registrations.
- Reject — at load time and at creation time (`package lock`/`package sync`) —
  any entry or input manifest whose `name` claims the reserved `std.*`
  namespace (`std` or `std.` prefix). Std packages ship embedded in the
  platform binary and cannot be provided by a package lock, so a supply-chain
  lock can never shadow them.
- `sync` writes a complete lock derived solely from the current package set.
  Entries are never carried over from a prior lock, so a package removed from
  `whip.packages.json` cannot persist as a stale lock entry. There is no separate
  garbage-collection step.
- Write the lock atomically: serialize to a temporary file in the lock directory
  and rename it over `whip.lock`, so a crash mid-write cannot corrupt an existing
  good lock.
- Preserve the existing invariant that package-owned source forms are accepted
  only when the lock authorizes the owning package contract.

The lock writer emits the portable `source` shape above: `source.path` is the
manifest path made relative to the lock file's own directory, `/`-separated, and
rejected if it is absolute or contains `..`. The lock digest is the lowercase
hex sha256 over the compact `canonical_json` of the lock object. Earlier
pre-release builds wrote an absolute, canonicalized `manifest_path` and digested
a non-canonical compact serialization; both are removed. No backward
compatibility promise is required for pre-release package locks.

## Canonical Serialization And Digests

Reproducibility (`--check-only` byte-identical, stable digests across machines)
requires a pinned serialization. There are two related but distinct forms.

On-disk form (`whip.lock`, and the `whipplescript.package_sync.v0` report):

```text
UTF-8, no BOM
object keys sorted lexicographically
2-space indentation (pretty-printed)
arrays in their declared sort order (lock packages by name, then package_id)
LF line endings
exactly one trailing newline
minimal JSON string escaping; non-ASCII emitted as UTF-8, not \uXXXX
```

`--check-only` compares the regenerated on-disk bytes against the current file
using these rules. Two conforming implementations, and the same implementation
across serializer versions, must produce identical bytes.

Digest form (used for `package_lock_digest` and `package_contract_digest`): the
compact canonical JSON of the artifact content — object keys sorted
lexicographically, no insignificant whitespace, array order preserved. This is
independent of on-disk indentation, so the digest is stable regardless of pretty
formatting. It matches the existing `canonical_json` helper.

```text
package_lock_digest
  lowercase hex sha256 over the compact canonical JSON of the lock object
  (schema + sorted packages, each with package_id, name, version, source,
  manifest_sha256)

package_contract_digest
  lowercase hex sha256 over the compact canonical JSON of the package_contract
  artifact with the package_contract_digest field omitted from the input, then
  inserted into the artifact (the contract digest cannot cover itself)
```

`manifest_sha256` remains the sha256 over the exact manifest bytes on disk and is
not affected by either canonical form.

## Standard Package Provenance

`std.*` packages are platform-provided and bundled with the WhippleScript binary.
In v0 they are intentionally outside the lock's integrity scope: they do not
appear in `whip.lock` (a lock entry claiming a `std.*` name is a load error,
and `package lock`/`package sync` refuse a `std.*`-named manifest), do not
require a project lock to be imported, and their
integrity is the integrity of the platform binary itself, not a per-manifest
hash. Only non-`std.` packages selected by the package set are locked and hashed.
Signing and manifest provenance for first- and third-party packages are deferred
(see Deferred Items).

## CLI

### `whip package sync`

Usage:

```sh
whip package sync [--file <whip.packages.json>] [--output <whip.lock>] [--check-only]
```

Behavior:

1. Discover `whip.packages.json` in the current directory or nearest ancestor,
   unless `--file` is provided.
2. Set the project root to the package-set file's parent directory.
3. Validate the package set against the closed v0 schema.
4. Resolve each `path` source relative to the project root.
5. Reject nonportable or escaping paths.
6. Load each package manifest through the existing manifest validator.
7. Check optional `package_id` and `version` assertions.
8. Merge the package registries and run the existing contract-registry
   validation.
9. Write a deterministic `whip.lock`, or the `--output` path if provided.

`--check-only` performs the same work but does not write. It exits successfully
only when the would-be lock is byte-identical to the current lock.

With global `--json`, `sync` should emit:

```json
{
  "schema": "whipplescript.package_sync.v0",
  "status": "ok",
  "package_set_path": "...",
  "package_lock_path": "...",
  "packages": [
    {
      "package_id": "package-notes",
      "name": "notes",
      "version": "0.1.0",
      "source": {"type": "path", "path": "examples/packages/notes.json"},
      "manifest_sha256": "..."
    }
  ],
  "package_lock_digest": "...",
  "diagnostics": []
}
```

On failure, `status` is `error`, `diagnostics` is non-empty, and no lock is
written.

### Existing Commands

`whip package check <manifest.json>...` remains the low-level manifest
validation command.

`whip package lock [--output <path>] <manifest.json>...` may remain as a
low-level escape hatch during implementation, but the project workflow should be
`whip package sync`.

`whip package catalog` remains the platform construct catalog command.

## Lock Discovery

Commands that analyze or run source should accept an explicit lock and otherwise
discover one:

```text
explicit --package-lock wins
otherwise search from the primary workflow file directory upward for whip.lock
otherwise search from current directory upward for whip.lock
otherwise proceed without a package lock
```

Affected commands:

```text
check
compile
dev
run
worker
```

If source imports a non-`std.` package or uses a package-owned construct and no
lock is available, the command must fail with a diagnostic that names the missing
lock and suggests `whip package sync`.

If multiple source files imply different discovered locks, the command must fail
and require `--package-lock`.

Standard libraries (`std.*`) do not require a project lock merely because they
appear in source. Non-standard package imports do.

## Diagnostics

Implementation should use stable diagnostic codes for package-management
failures:

```text
package_set.missing
package_set.invalid_schema
package_set.duplicate_name
package_set.identity_mismatch
package_source.unsupported_type
package_source.nonportable_path
package_source.escapes_project
package_manifest.invalid
package_contract.invalid
package_lock.missing
package_lock.stale
package_lock.identity_mismatch
package_lock.hash_mismatch
package_lock.ambiguous
package_sync.lock_changed
```

Diagnostics should distinguish:

```text
intent file is missing
intent file is malformed
manifest is malformed
manifest is valid but contradicts package set assertions
lock is stale relative to manifest bytes
lock exists but does not include an imported package
lock exists but package contract validation fails
provider authority is missing at runtime
```

The last case is not a package-management failure. It belongs to provider and
capability diagnostics.

## Security And Portability

The v0 package manager is local and deterministic:

- no network access
- no remote source fetching
- no script execution
- no provider binary installation
- no credential lookup
- no ambient package discovery outside the selected package set and lock
- no authority grant from package import or package sync

Path sources must be project-relative and must not escape the project root.
Symlink resolution must be checked before reading manifests for `sync`.

CI should be able to run:

```sh
whip package sync --check-only
whip check workflow.whip
```

without machine-specific absolute paths in `whip.lock`.

## Acceptance Criteria

Implementation is complete for v0 when:

- `whip package sync` writes a deterministic lock from `whip.packages.json`.
- `sync --check-only` fails when the lock is missing, stale, or differently
  formatted.
- Locks use portable project-relative path sources.
- Existing manifest validation and package contract validation are reused.
- `check`, `compile`, `dev`, `run`, and `worker` discover `whip.lock` by
  default.
- Explicit `--package-lock` still overrides discovery.
- Non-standard package imports fail clearly when no lock is available.
- Runtime commands register locked manifests before provider policy checks.
- JSON reports include enough package-set and package-lock paths to reproduce
  the run.
- Tests cover stale lock, hash mismatch, identity mismatch, path escape,
  duplicate package names, missing lock for `use notes`, explicit lock override,
  and successful default discovery.

## Deferred Items

Registry and dependency resolution:

- registry protocol
- package publishing
- package install/update/uninstall
- semver ranges and resolver behavior
- transitive package dependencies
- lockfile conflict resolution
- offline mirrors and package cache

Trust and distribution:

- package signing
- manifest provenance
- bundle hashing beyond a single manifest file
- native provider binary acquisition
- sidecar lifecycle installation
- platform-specific package artifacts

Workspace and deployment:

- named environments
- deployment profiles
- multi-root workspaces
- team policy files
- package migration hooks
- remote package source policy

Provider integration:

- automatic provider config creation
- provider credential setup
- provider health checks as package install hooks
- provider-specific scaffolding

Language/package evolution:

- package-owned source declarations beyond accepted platform extension classes
- package-authored lowering classes beyond `metadata_only` and
  `capability_call`
- package-defined storage lifecycles
- package-defined scheduler or retry semantics

None of these should block the local package-set and sync workflow.
