# Install

## A prebuilt release

Download an archive for your platform from the
[releases page](https://github.com/winterop-com/chaps/releases) and put the
`chaps` binary on your `PATH`. Seven archives are built, and each one ships
with a `.sha256` file next to it:

| Platform | Archive |
| --- | --- |
| macOS, any Mac | `universal-apple-darwin` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |
| Linux, x86_64 | `x86_64-unknown-linux-musl` |
| Linux, arm64 | `aarch64-unknown-linux-musl` |
| Windows, x86_64 | `x86_64-pc-windows-msvc` |
| Windows, arm64 | `aarch64-pc-windows-msvc` |

The Linux builds are static musl binaries, so one file runs on any
distribution with no shared library to match. Linux and macOS ship as
`tar.gz`, Windows as `zip`.

On macOS take the universal archive unless you have a reason to pick a single
architecture: it carries the Apple silicon and Intel slices in one binary and
runs on any Mac.

The version is part of every archive name, so the commands below name a tag.
Replace `v0.1.0` with the release you want.

On macOS:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/download/v0.1.0/chaps-v0.1.0-universal-apple-darwin.tar.gz
tar -xzf chaps-v0.1.0-universal-apple-darwin.tar.gz
sudo install -m 0755 chaps-v0.1.0-universal-apple-darwin/chaps /usr/local/bin/chaps
```

On a Linux server:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/download/v0.1.0/chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sudo install -m 0755 chaps-v0.1.0-x86_64-unknown-linux-musl/chaps /usr/local/bin/chaps
```

## Verifying a download

Every release carries a `SHA256SUMS` file covering all seven archives, next to
the per-archive `.sha256` files:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/download/v0.1.0/SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing
```

The macOS binaries are signed with an Apple Developer ID certificate and
notarized by Apple, so Gatekeeper lets them run: no right-click to open, no
`xattr -d com.apple.quarantine`. To see who signed one:

```sh
codesign -dv --verbose=4 /usr/local/bin/chaps
```

Apple serves the notarization ticket online rather than it being stapled to
the file, because a bare executable has nowhere to carry a ticket. The first
run on a machine therefore wants to reach Apple once; after that the verdict
is cached.

The Windows binaries are not signed, which would need an Authenticode
certificate. SmartScreen may warn the first time `chaps.exe` runs; "More info"
then "Run anyway" gets past it. There the `.sha256` file is the check that
matters.

## From a checkout

```sh
cargo install --path .
```

Or build and install through the Makefile, which puts the binary in
`$PREFIX/bin` with `PREFIX` defaulting to `~/.local`:

```sh
make install
```

On macOS `make release` builds a universal (arm64 plus x86_64) binary at
`bin/chaps` first, and `make install` copies that; on Linux it builds the host
binary. See [Development](./development.md) for the rest of the targets.

## Requirements

On the machine that runs CHAP: Docker, with Compose v2.20 or newer. That
version is where `include:` arrived, and `chaps` uses `include:` for the
umbrella file that names one overlay per enabled model. A single published
override also uses `!override`, which needs Compose 2.24 or newer; see
[Ports](./ports.md).

Nothing else is required. No Python, no `uv`, no checkout of chap-core.

`chaps` checks the installed Compose version and warns on stderr when it is
older than 2.20 rather than failing, because an older Compose still runs most
of CHAP. If `include:` is not supported, the model overlays are the part
that will not load.

## A note on the name

The binary is `chaps`, not `chap`, because chap-core's own Python package
already installs a console script called `chap`. The two never collide, so a
machine can have both.
