# Install WhippleScript

The WhippleScript command-line binary is `whip`.

## Source Install

Until the first binary release is published, install from source with Cargo.
The beginner path is to clone the repository and install from the checkout:

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

Prebuilt releases are planned for GitHub Releases. The release workflow is
configured to publish archives and checksums for:

- Apple Silicon macOS: `aarch64-apple-darwin`
- Intel macOS: `x86_64-apple-darwin`
- x64 Windows: `x86_64-pc-windows-msvc`
- x64 Linux: `x86_64-unknown-linux-gnu`
- ARM64 Linux: `aarch64-unknown-linux-gnu`

After the first release is cut, the expected installer commands are:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.sh | sh
```

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/jamesjscully/whipplescript/releases/latest/download/whipplescript-installer.ps1 | iex"
```

Manual archive downloads will also be available from the matching GitHub Release.
Each archive has a `.sha256` file and the release includes `sha256.sum`.

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
and notarization are enabled. Prefer the source install path until signed
release artifacts are published.

On Linux, the default binary artifacts target GNU libc. If a binary reports a
glibc compatibility error, use source install or wait for the optional musl
artifact tracked in [`../spec/distribution-tracker.md`](../spec/distribution-tracker.md).
