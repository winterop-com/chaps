# chaps

`chaps` is the CHAP stack manager. It deploys and manages
[CHAP](https://chap.dhis2.org), the DHIS2 Climate Health Analytics Platform, as
a Docker Compose deployment of
[chap-core](https://github.com/dhis2-chap/chap-core) together with the
forecasting model services published in the
[CHAP model marketplace](https://github.com/dhis2-chap/model-marketplace). One
command writes a self-contained deployment directory, and after that `chaps` is
a thin wrapper around `docker compose` that always passes the explicit `-f`
list, plus a model manager that can add or remove models without you
hand-editing YAML. A machine that runs it needs Docker and one binary: no
Python, no uv, no checkout of chap-core.

**Documentation: <https://mortenoh.github.io/chaps-cli/>**

## Install

Download an archive for your platform from the
[releases page](https://github.com/mortenoh/chaps-cli/releases) and put the
`chaps` binary on your `PATH`:

```sh
curl -fsSLO https://github.com/mortenoh/chaps-cli/releases/latest/download/chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sudo install -m 0755 chaps-v0.1.0-x86_64-unknown-linux-musl/chaps /usr/local/bin/chaps
```

Or, from a checkout, `cargo install --path .` or `make install`. Requires
Docker with Compose v2.20 or newer. See
[Install](https://mortenoh.github.io/chaps-cli/install.html).

## Quickstart

```sh
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

## The two words to remember

- **`chaps up`** starts what is on disk. It never changes which version of
  anything you run.
- **`chaps update`** fetches newer versions: it asks the marketplace and the
  chap-core release feed what they publish today, moves the pins, then pulls
  and restarts.

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

CI runs the same three checks on Linux, macOS and Windows. Tagging `vX.Y.Z`
builds release binaries for six targets (Linux, macOS and Windows, on x86_64
and aarch64) and attaches them to a GitHub release with their SHA-256 sums.

The documentation lives in `docs/` and is built with
[mdbook](https://rust-lang.github.io/mdBook/); `docs/reference.md` is generated
from the `--help` texts by `make docs-reference` and checked by `cargo test`.
