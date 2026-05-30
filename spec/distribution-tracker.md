# Distribution Tracker

Status: draft, active

This tracker turns the cross-platform distribution plan into executable work.
The release goal is that a user on macOS, Windows, or Linux can install the
`whip` CLI without cloning the repository or understanding the Rust workspace.

## Goals

- Provide a verified source-install path for early adopters.
- Publish signed or checksummed release binaries for macOS, Windows, and Linux.
- Keep GitHub Releases as the canonical artifact source.
- Add package-manager entry points only after release artifacts are stable.
- Keep release verification tied to the existing readiness checks.

## Non-Goals For The First Pass

- GUI installers.
- Long-running system services or daemon packaging.
- Linux distro-native packages such as `.deb`, `.rpm`, Arch, or Nixpkgs.
- macOS notarization and Windows Authenticode signing before binary release
  plumbing is stable.

## Phase 0: Source Install And Package Identity

- [x] Choose the first public install command:
  `cargo install --git https://github.com/jamesjscully/whipplescript.git --package whipplescript-cli --locked`.
- [x] Add package metadata needed for source installs and future registry
  publishing.
- [x] Document source install, local checkout install, and the installed binary
  smoke check.
- [x] Verify `cargo install --path crates/whipplescript-cli --locked` in a temp
  root.
- [ ] Decide whether to keep the package name `whipplescript-cli` or rename the
  publishable CLI crate to `whipplescript` while preserving the binary name
  `whip`.

Latest verification:

```text
cargo install --path crates/whipplescript-cli --locked --root /tmp/whipplescript-install-smoke --force
/tmp/whipplescript-install-smoke/bin/whip --version
/tmp/whipplescript-install-smoke/bin/whip doctor --json
/tmp/whipplescript-install-smoke/bin/whip check examples/minimal-noop.whip
```

Result: passed. The installed binary reported `whipplescript 0.1.0`, opened the
default SQLite store, and checked `examples/minimal-noop.whip`.

## Phase 1: GitHub Release Backbone

- [x] Add `cargo-dist` / `dist` release metadata.
- [ ] Generate and review the release workflow.
- [x] Configure release artifact targets for:
  - [x] `aarch64-apple-darwin`
  - [x] `x86_64-apple-darwin`
  - [x] `x86_64-pc-windows-msvc`
  - [x] `x86_64-unknown-linux-gnu`
  - [x] `aarch64-unknown-linux-gnu`
  - [ ] optional `x86_64-unknown-linux-musl`
- [x] Build and smoke-test a local `x86_64-unknown-linux-gnu` release archive.
- [x] Generate local shell and PowerShell installer artifacts.
- [ ] Build all configured platform artifacts in CI.
- [ ] Publish archive assets with checksums.
- [ ] Publish shell and PowerShell installers.
- [ ] Add packaged-binary smoke checks:
  - [ ] `whip --version`
  - [ ] `whip doctor --json`
  - [ ] `whip check examples/minimal-noop.whip`

Latest local `dist` verification:

```text
dist plan --allow-dirty --output-format=json --no-local-paths
dist build --artifacts=local --target=x86_64-unknown-linux-gnu --allow-dirty
dist build --artifacts=global --allow-dirty
/tmp/whipplescript-dist-smoke/whipplescript-cli-x86_64-unknown-linux-gnu/whip --version
/tmp/whipplescript-dist-smoke/whipplescript-cli-x86_64-unknown-linux-gnu/whip doctor --json
/tmp/whipplescript-dist-smoke/whipplescript-cli-x86_64-unknown-linux-gnu/whip check examples/minimal-noop.whip
```

Result: passed for the local Linux x64 archive, global source archive, shell
installer artifact, PowerShell installer artifact, and checksum files.

Open issue: `dist 0.32.0` planned the GitHub CI release matrix, but
`dist generate --mode ci --check` produced no `.github/workflows/release.yml`.
Keep release workflow generation pending until that behavior is explained or a
reviewed workflow is added manually.

## Phase 2: Friendly Package Managers

- [ ] Create `jamesjscully/homebrew-tap`.
- [ ] Publish a Homebrew formula from release artifacts.
- [ ] Add Homebrew install docs.
- [ ] Prepare crates for crates.io publishing by replacing internal path-only
  dependencies with versioned path dependencies.
- [ ] Publish `whipplescript` or `whipplescript-cli` to crates.io.
- [ ] Revisit Windows package managers after GitHub release assets are stable:
  - [ ] WinGet
  - [ ] Scoop
  - [ ] Chocolatey

## Phase 3: Trust And Provenance

- [ ] Sign macOS binaries with Developer ID.
- [ ] Notarize macOS release artifacts.
- [ ] Sign Windows binaries with Authenticode.
- [ ] Record checksum verification instructions in install docs.
- [ ] Consider release provenance attestations once the release workflow is
  stable.

## Phase 4: Documentation And Operations

- [ ] Add `docs/install.md` as the canonical install guide.
- [ ] Update README to prefer installed `whip` commands.
- [ ] Update `spec/quickstart.md` so the default path is installed `whip`, with
  `cargo run -p whipplescript-cli --` kept as the checkout fallback.
- [ ] Update `docs/manual.md`, `docs/language-reference.md`, and
  `docs/runtime-operations.md` examples to avoid requiring `cargo run`.
- [ ] Add a release checklist section for distribution artifacts.
- [ ] Add troubleshooting notes for PATH issues, Gatekeeper quarantine,
  PowerShell execution policy, and Linux libc compatibility.

## Open Decisions

- Should the crates.io package be named `whipplescript` even though the current
  workspace package is `whipplescript-cli`?
- Should Linux default to GNU libc artifacts only, or should musl be promoted to
  a first-class target?
- Should the Nix flake eventually expose a `whip` package, or remain a dev-shell
  dependency provider for formal tools?
- Which release channels are needed after v0.1: stable only, or stable plus
  nightly/pre-release?
