# Install

## A prebuilt release

Download an archive for your platform from the
[releases page](https://github.com/mortenoh/chaps-cli/releases) and put the
`chaps` binary on your `PATH`. Six targets are built, and each archive ships
with a `.sha256` file next to it:

| Platform | Target |
| --- | --- |
| Linux, x86_64 | `x86_64-unknown-linux-musl` |
| Linux, arm64 | `aarch64-unknown-linux-musl` |
| macOS, Intel | `x86_64-apple-darwin` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| Windows, x86_64 | `x86_64-pc-windows-msvc` |
| Windows, arm64 | `aarch64-pc-windows-msvc` |

The Linux builds are static musl binaries, so one file runs on any
distribution with no shared library to match. Linux and macOS ship as
`tar.gz`, Windows as `zip`.

On a Linux server:

```sh
curl -fsSLO https://github.com/mortenoh/chaps-cli/releases/latest/download/chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sudo install -m 0755 chaps-v0.1.0-x86_64-unknown-linux-musl/chaps /usr/local/bin/chaps
```

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

On the machine that runs the stack: Docker, with Compose v2.20 or newer. That
version is where `include:` arrived, and `chaps` uses `include:` for the
umbrella file that names one overlay per enabled model. A single published
override also uses `!override`, which needs Compose 2.24 or newer; see
[Ports](./ports.md).

Nothing else is required. No Python, no `uv`, no checkout of chap-core.

`chaps` checks the installed Compose version and warns on stderr when it is
older than 2.20 rather than failing, because an older Compose still runs most
of the stack. If `include:` is not supported, the model overlays are the part
that will not load.

## A note on the name

The binary is `chaps`, not `chap`, because chap-core's own Python package
already installs a console script called `chap`. The two never collide, so a
machine can have both.
