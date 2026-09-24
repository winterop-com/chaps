# chaps

[![ci](https://github.com/winterop-com/chaps/actions/workflows/ci.yml/badge.svg)](https://github.com/winterop-com/chaps/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/winterop-com/chaps)](https://github.com/winterop-com/chaps/releases/latest)
[![license](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
[![docs](https://img.shields.io/badge/docs-winterop--com.github.io%2Fchaps-informational)](https://winterop-com.github.io/chaps/)

`chaps` is the CHAP stack manager. It deploys and manages
[CHAP](https://chap.dhis2.org), the Climate Health Analytics Platform, as
a Docker Compose deployment of
[chap-core](https://github.com/dhis2-chap/chap-core) together with the
forecasting model services published in the
[CHAP model marketplace](https://github.com/dhis2-chap/model-marketplace). One
command writes a self-contained deployment directory, and after that `chaps` is
a thin wrapper around `docker compose` that always passes the explicit `-f`
list, plus a model manager that can add or remove models without you
hand-editing YAML. A machine that runs it needs Docker and one binary: no
Python, no uv, no checkout of chap-core.

Licensed under the AGPL-3.0, like chap-core.

## Install

On macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
```

Or `... | sh -s -- --here` for the binary alone, as `./chaps` in the current
directory and nothing else.

That works out the platform, downloads the release archive for it, checks it
against the release's `SHA256SUMS` and installs the binary into
`/usr/local/bin` when that is writable and `~/.local/bin` otherwise.
`CHAPS_INSTALL_DIR=/somewhere sh` or `--dir` picks the directory,
`CHAPS_VERSION=v0.2.0` or `--version` picks the release, and `--dry-run` says
what it would do. Shell completions are installed alongside when the
directories a shell reads already exist.

Or download an archive and put `chaps` on your `PATH` yourself. These links
always point at the newest release:

| Platform | Download | Notes |
| --- | --- | --- |
| Linux x86_64 | [`chaps-x86_64-unknown-linux-musl.tar.gz`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-x86_64-unknown-linux-musl.tar.gz) | static, any distro |
| macOS | [`chaps-universal-apple-darwin.tar.gz`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-universal-apple-darwin.tar.gz) | universal, signed and notarized |
| Windows x86_64 | [`chaps-x86_64-pc-windows-msvc.zip`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-x86_64-pc-windows-msvc.zip) | unsigned |

<details>
<summary>Other platforms</summary>

| Platform | Download | Notes |
| --- | --- | --- |
| Linux arm64 | [`chaps-aarch64-unknown-linux-musl.tar.gz`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-aarch64-unknown-linux-musl.tar.gz) | static, any distro |
| macOS, Apple silicon | [`chaps-aarch64-apple-darwin.tar.gz`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-aarch64-apple-darwin.tar.gz) | signed and notarized |
| macOS, Intel | [`chaps-x86_64-apple-darwin.tar.gz`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-x86_64-apple-darwin.tar.gz) | signed and notarized |
| Windows arm64 | [`chaps-aarch64-pc-windows-msvc.zip`](https://github.com/winterop-com/chaps/releases/latest/download/chaps-aarch64-pc-windows-msvc.zip) | unsigned |

</details>

An archive holds one directory, `chaps-<version>-<target>/`, with the binary,
`README.md`, `LICENSE` and shell completions. The name of the archive carries
no version, so the links above keep working after the next release, and
`SHA256SUMS` in the release covers every one of them.

The macOS binaries are signed with an Apple Developer ID certificate and
notarized, so Gatekeeper does not block them. The Windows binaries are
unsigned and SmartScreen may warn on first run.

Once installed, `chaps self update` replaces the binary with the newest
release, `chaps self version` says which build this is, and `chaps completions
<bash|zsh|fish|powershell|elvish>` prints a completion script.

Or, from a checkout, `cargo install --path .` or `make install`. Requires
Docker with Compose v2.24.4 or newer. See
[Install](https://winterop-com.github.io/chaps/install.html).

## Quickstart

```sh
chaps doctor                         # is this machine ready: docker, compose, disk, network
chaps init mychap --models default   # writes the deployment directory
cd mychap
chaps up                             # sync the compose files, docker compose up -d
chaps status                         # chap-core health and registered models
chaps ui                             # browse the marketplace, toggle models
chaps up                             # apply what the browser changed
```

chap-core's API is on <http://localhost:8000>; model services are reached
through it, or given a port of their own with `chaps models expose ID`. Every
command takes `--json`, and every command ends with a line saying what it did.

## The three words to remember

- **`chaps up`** starts what is on disk. It never changes which version of
  anything you run.
- **`chaps update`** fetches newer versions: it asks the marketplace and the
  chap-core release feed what they publish today, moves the pins and pulls the
  images. It never touches a container; it says which ones are now out of date.
- **`chaps restart`** applies them, recreating the running services whose image
  or configuration changed and leaving the rest alone.

## Development

```sh
make test      # run the test suite
make check     # formatting and clippy, fixing nothing
make release   # universal (arm64+x86_64) macOS binary at bin/chaps
make install   # copy bin/chaps to $PREFIX/bin (PREFIX defaults to ~/.local)
make vendor    # refresh the embedded marketplace snapshot
make docs      # build the documentation into site/
make docs-serve  # serve it locally at http://localhost:3000
```

`make help` lists every target. The underlying commands are `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check` and
`scripts/vendor-marketplace.sh`.

CI runs the same three checks on Linux, macOS and Windows, and shellcheck over
`install.sh` and `scripts/`. Tagging `vX.Y.Z` builds release binaries for six
targets (Linux, macOS and Windows, on x86_64 and aarch64), fuses the two macOS
builds into a seventh universal archive, and attaches all seven to a GitHub
release together with one `SHA256SUMS`, each archive carrying the shell
completion scripts. A weekly workflow refreshes the embedded marketplace
snapshot and opens a pull request when upstream moved.

The documentation lives in `docs/` and is built with
[mdbook](https://rust-lang.github.io/mdBook/); `docs/reference.md` is generated
from the `--help` texts by `make docs-reference` and checked by `cargo test`.
