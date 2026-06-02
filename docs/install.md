# Install WhippleScript

The WhippleScript command-line binary is `whip`.

## Source Install

If you want the current branch rather than a tagged release, install from source
with Cargo. The beginner path is to clone the repository and install from the
checkout:

```sh
git clone https://github.com/jamesjscully/whipplescript.git
cd whipplescript
cargo install --path crates/whipplescript-cli --locked
```

You can also install directly from Git:

```sh
cargo install --git https://github.com/jamesjscully/whipplescript.git --package whipplescript --locked
```

Cargo installs binaries into `~/.cargo/bin` by default. Make sure that directory
is on `PATH`.

Verify the install:

```sh
whip --version
whip doctor
```

From a repository checkout, verify the example compiler path:

```sh
whip check examples/minimal-noop.whip
```

## Prebuilt Releases

Prebuilt release artifacts are published from GitHub Releases. The release
workflow publishes archives, shell installers, PowerShell installers, and
checksums for:

- Apple Silicon macOS: `aarch64-apple-darwin`
- Intel macOS: `x86_64-apple-darwin`
- x64 Windows: `x86_64-pc-windows-msvc`
- x64 Linux: `x86_64-unknown-linux-gnu`
- ARM64 Linux: `aarch64-unknown-linux-gnu`

Install with the shell installer on macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.ps1 | iex"
```

Manual archive downloads will also be available from the matching GitHub Release.
Each archive has a `.sha256` file and the release includes `sha256.sum`.

Verify an archive against its adjacent checksum:

```sh
curl -LO https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-x86_64-unknown-linux-gnu.tar.xz
curl -LO https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-x86_64-unknown-linux-gnu.tar.xz.sha256
sha256sum --check whipplescript-x86_64-unknown-linux-gnu.tar.xz.sha256
```

Or verify every downloaded artifact against `sha256.sum`:

```sh
curl -LO https://github.com/jamesjscully/whipplescript/releases/latest/download/sha256.sum
sha256sum --check sha256.sum
```

On macOS, use `shasum -a 256 -c <file>.sha256` if `sha256sum` is not installed.

## Homebrew

Homebrew installation will be enabled after the first tagged release publishes
stable GitHub Release assets:

```sh
brew tap jamesjscully/tap
brew install whipplescript
```

Until the tap formula is published, use the shell installer, manual archive, or
Cargo source install path.

## Build From Checkout

You can run the CLI directly from the repository without installing it:

```sh
cargo run -p whipplescript -- doctor
cargo run -p whipplescript -- check examples/minimal-noop.whip
```

Use this path for development, not for end-user setup.

## Troubleshooting

If `whip` is not found after source install, add Cargo's bin directory to
`PATH`, then open a new shell:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

On Windows, restart the terminal after the installer updates `PATH`.

On macOS, unsigned prebuilt binaries may be blocked by Gatekeeper until signing
and notarization are enabled. If Gatekeeper blocks a v0.1 archive, use the
source install path or remove quarantine only after verifying the release
checksum.

On Linux, the default binary artifacts target GNU libc. If a binary reports a
glibc compatibility error, use source install or wait for the optional musl
artifact tracked in [`../spec/distribution-tracker.md`](../spec/distribution-tracker.md).
