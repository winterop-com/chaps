# Development

## Make targets

```sh
make test      # run the test suite
make check     # formatting and clippy, fixing nothing
make lint      # fix what check reports: clippy --fix, then format (writes files)
make build     # fast host-arch compile check
make release   # universal (arm64+x86_64) macOS binary at bin/chaps
make install   # copy bin/chaps to $PREFIX/bin (PREFIX defaults to ~/.local)
make run       # build the release binary, then run it (ARGS="...")
make vendor    # refresh the embedded marketplace snapshot
make docs      # regenerate the reference, then build the book into site/
make docs-serve        # serve the book locally with live reload
make docs-reference    # regenerate docs/reference.md from the CLI help texts
make clean     # remove target/ and bin/
```

`make help` lists every target. The underlying commands are `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check` and
`scripts/vendor-marketplace.sh`.

## Tests

```sh
cargo test
```

Unit tests live next to the code they cover, in a `mod tests` at the bottom of
each module. `tests/cli.rs` is the end-to-end suite: it runs the built binary
against temporary directories, and every run is `--offline` with its own cache
directory, so the embedded marketplace snapshot is what the CLI sees and no
test touches the network or the developer's real cache.

The tests that need Docker check for it first and skip with a note on stderr
when it is not there, so the suite is green on a machine without a daemon.

## Vendoring the marketplace snapshot

The catalogue compiled into the binary is the last fallback when the registry
can be neither fetched nor read from cache, so `chaps` works on a machine that
has never had network access.

```sh
make vendor
```

runs `scripts/vendor-marketplace.sh`, which refreshes the files under
`vendor/marketplace/`. If the *set* of model files changed rather than only
their contents, update the `FILES` list in `src/registry/embedded.rs` to match.

The vendored layout is the same as the on-disk cache layout, so
`vendor/marketplace/` and a cache entry are interchangeable.

## The documentation

The book is mdbook. `book.toml` at the repository root points `src` at `docs/`
and `build-dir` at `site/`, which is ignored by git.

```sh
make docs-serve    # http://localhost:3000, rebuilds as you write
make docs          # regenerate the reference, then build site/
```

`docs/SUMMARY.md` is the table of contents; a chapter that is not listed there
is not built. Every other file in `docs/` is a hand-written chapter, except
`docs/reference.md`.

## How the command reference is regenerated

`docs/reference.md` is generated from the `--help` texts, so a flag cannot
change without the chapter changing with it.

```sh
make docs-reference     # cargo run -q -- docs-markdown > docs/reference.md
```

`chaps docs-markdown` is a hidden subcommand (`hide = true`) implemented in
`src/commands/docs.rs`. It walks the static `Cli::command()` tree through
clap's introspection API and prints one `##` section per command path, with the
about text, a fenced usage line from clap's `render_usage`, and a table of
arguments and options.

Two details matter. It renders from the **static** tree rather than the one
`main.rs` parsed, because `main.rs` hides the project-only commands when there
is no project and the reference has to list all of them. And it skips clap's
own `-h` and `-V` by their *action* rather than their name, because
`chaps models enable --version` is a flag of ours that must stay.

The generated file is committed, and `cargo test` compares
`chaps docs-markdown` to it byte for byte. When that test fails, run
`make docs-reference` and commit the result. `make docs` depends on
`docs-reference`, so a local book build can never be stale.

To document a new flag: add it to `src/cli.rs` with a doc comment, run
`make docs-reference`, and mention it in whichever hand-written chapter it
belongs to.

## Continuous integration

`.github/workflows/ci.yml` runs the same three checks as `make check` plus
`make test` on Linux, macOS and Windows, on every push to `main` and every pull
request.

`.github/workflows/release.yml` builds release binaries for six targets
(Linux, macOS and Windows, on x86_64 and aarch64) when a `vX.Y.Z` tag is
pushed, and attaches them to a GitHub release with their SHA-256 sums. Every
target is built natively, so there is no cross-compilation toolchain to
maintain, and the Linux builds are static musl so one binary runs on any
distribution. A `workflow_dispatch` run exercises the build matrix without
creating a release.

`.github/workflows/docs.yml` publishes the book to GitHub Pages on every push
to `main`. It regenerates `docs/reference.md` and fails if the result differs
from the committed file, which is the same check `cargo test` makes, then runs
`mdbook build` and deploys `site/`.
