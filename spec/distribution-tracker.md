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
  `cargo install --git https://github.com/jamesjscully/whipplescript.git --package whipplescript --locked`.
- [x] Add package metadata needed for source installs and future registry
  publishing.
- [x] Document source install, local checkout install, and the installed binary
  smoke check.
- [x] Verify `cargo install --path crates/whipplescript-cli --locked` in a temp
  root.
- [x] Decide package identity: the publishable package is `whipplescript`; the
  installed binary remains `whip`; the crate source path remains
  `crates/whipplescript-cli`.

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
- [x] Generate and review the release workflow.
- [x] Configure release artifact targets for:
  - [x] `aarch64-apple-darwin`
  - [x] `x86_64-apple-darwin`
  - [x] `x86_64-pc-windows-msvc`
  - [x] `x86_64-unknown-linux-gnu`
  - [x] `aarch64-unknown-linux-gnu`
  - [ ] optional `x86_64-unknown-linux-musl`
- [x] Build and smoke-test a local `x86_64-unknown-linux-gnu` release archive.
- [x] Generate local shell and PowerShell installer artifacts.
- [x] Add a `dist` custom CI job that smoke-tests the packaged Linux x64 archive
  before publish.
- [ ] Build all configured platform artifacts in CI.
- [ ] Publish archive assets with checksums.
- [ ] Publish shell and PowerShell installers.
- [x] Add packaged-binary smoke checks:
  - [x] `whip --version`
  - [x] `whip doctor --json`
  - [x] `whip check examples/minimal-noop.whip`

Latest local `dist` verification:

```text
dist plan --output-format=json --no-local-paths
dist build --artifacts=local --target=x86_64-unknown-linux-gnu
dist build --artifacts=global
scripts/check-dist-archive.sh target/distrib/whipplescript-x86_64-unknown-linux-gnu.tar.xz
```

Result: passed for the local Linux x64 archive, global source archive, shell
installer artifact, PowerShell installer artifact, checksum files, and extracted
`whip` smoke checks.

Correction from setup: `dist generate --mode ci --allow-dirty` intentionally
skips CI regeneration because the generated CI is allowed to be out of date.
Running `dist generate --mode ci` produced `.github/workflows/release.yml`.

## Phase 2: Friendly Package Managers

- [ ] Create `jamesjscully/homebrew-tap`.
- [ ] Publish a Homebrew formula from release artifacts.
- [ ] Add Homebrew install docs.
- [ ] Prepare crates for crates.io publishing by replacing internal path-only
  dependencies with versioned path dependencies.
- [ ] Publish `whipplescript` to crates.io.
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

- [x] Add `docs/install.md` as the canonical install guide.
- [x] Update README to prefer installed `whip` commands.
- [x] Update `spec/quickstart.md` so the default path is installed `whip`, with
  `cargo run -p whipplescript --` kept as the checkout fallback.
- [x] Update `docs/manual.md`, `docs/language-reference.md`, and
  `docs/runtime-operations.md` examples to avoid requiring `cargo run`.
- [x] Add a release checklist section for distribution artifacts.
- [x] Add troubleshooting notes for PATH issues, Gatekeeper quarantine,
  PowerShell execution policy, and Linux libc compatibility.

## Open Decisions

- Should Linux default to GNU libc artifacts only, or should musl be promoted to
  a first-class target?
- Should the Nix flake eventually expose a `whip` package, or remain a dev-shell
  dependency provider for formal tools?
- Which release channels are needed after v0.1: stable only, or stable plus
  nightly/pre-release?
