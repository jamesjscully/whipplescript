# Install

The WhippleScript CLI is a single binary, `whip`.

These instructions install the latest published release. If you are reading
docs from a local checkout of `main`, the checkout may describe flags or JSON
fields newer than that release. For exact release behavior, use the docs from
the matching Git tag.

## Prebuilt binaries

Releases publish archives, installers, and checksums for macOS
(Apple Silicon and Intel), Windows x64, and Linux (x64 and ARM64, GNU libc).

macOS / Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.sh | sh
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.ps1 | iex"
```

To verify a manually downloaded archive, check it against its adjacent
`.sha256` file (or the release-wide `sha256.sum`):

```sh
curl -LO https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-x86_64-unknown-linux-gnu.tar.xz
curl -LO https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-x86_64-unknown-linux-gnu.tar.xz.sha256
sha256sum --check whipplescript-x86_64-unknown-linux-gnu.tar.xz.sha256
```

On macOS use `shasum -a 256 -c` if `sha256sum` is unavailable.

A Homebrew tap (`brew tap jamesjscully/tap && brew install whipplescript`)
will be enabled once tagged releases stabilize.

## From source

Requires a Rust toolchain (<https://rustup.rs/>).

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
```

Or directly from Git:

```sh
cargo install --git https://github.com/jamesjscully/whipplescript.git --package whipplescript --locked
```

Cargo installs to `~/.cargo/bin`; make sure it is on `PATH`.

## Verify

```sh
whip --version
whip doctor
whip check examples/minimal-noop.whip   # from a checkout
```

`whip --version` prints the package version, for example
`whipplescript 0.3.0`. `whip --help` also prints the implementation-stage
label in parentheses; that label is for project tracking and does not replace
the package version.

`doctor` reports optional tooling (Maude, Apalache, Python `jsonschema`, and
provider CLIs). None of it is needed for fixture-backed development. Generated
formal checks and report-schema validation can use the Nix dev shell
(`nix develop`) or `python3 -m pip install -r requirements-dev.txt` from a
checkout.

## Running without installing

From a checkout, substitute `cargo run -p whipplescript --` for `whip` in any
command. Use this for development on WhippleScript itself.

## Platform notes

- **macOS Gatekeeper:** prebuilt binaries are not yet signed. If a download
  is blocked, install from source, or remove quarantine only after verifying
  the checksum.
- **Linux libc:** binaries target GNU libc. On musl-based systems, install
  from source (a musl artifact is tracked in
  [`spec/distribution-tracker.md`](https://github.com/jamesjscully/whipplescript/blob/main/spec/distribution-tracker.md)).
- **Windows:** restart the terminal after the installer updates `PATH`.
