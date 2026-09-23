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
make release-tag VERSION=x.y.z   # bump, check, commit and tag a release
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
pushed, fuses the two macOS builds into a seventh universal archive, and
attaches them all to a GitHub release with their SHA-256 sums. Every target is
built natively, so there is no cross-compilation toolchain to maintain, and
the Linux builds are static musl so one binary runs on any distribution. A
`workflow_dispatch` run exercises the build matrix without creating a release.

`.github/workflows/docs.yml` publishes the book to GitHub Pages on every push
to `main`. It regenerates `docs/reference.md` and fails if the result differs
from the committed file, which is the same check `cargo test` makes, then runs
`mdbook build` and deploys `site/`.

## Releasing

### Prerequisites

The macOS archives are signed with an Apple Developer ID certificate and
notarized, which needs six repository secrets. `scripts/github-secrets.sh`
sets them, reading each value from 1Password and piping it into `gh secret
set` on a file descriptor, so no secret is written to disk or shown in a
process list:

```sh
OP_VAULT="Private" OP_ITEM="Apple Developer ID" scripts/github-secrets.sh
```

The six are `APPLE_CERTIFICATE` (base64 of the exported `.p12`),
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
`APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. They are the
same six the `winterop-com/maneki` desktop build uses. `--scope org` sets them
on the organisation with `--visibility selected` instead of on this repository
alone; `--dry-run` shows what would be set.

Without the secrets the workflow still succeeds and publishes unsigned macOS
binaries, so a fork or a first tag is never blocked on them.

### Cutting a release

```sh
make release-tag VERSION=0.2.0
```

which runs `scripts/release.sh`. It refuses to do anything unless the tree is
clean and the branch is `main`, bumps `version` in `Cargo.toml` and
`Cargo.lock`, runs `make check test`, commits `chore: release v0.2.0`, creates
an annotated `v0.2.0` tag, and then prints the push command rather than
pushing. Nothing reaches GitHub until you run it:

```sh
git push origin main v0.2.0
```

`make release-tag VERSION=0.2.0 PUSH=1`, or `scripts/release.sh 0.2.0 --push`,
pushes straight away.

### What the workflow produces

The tag starts `release.yml`, which attaches to the release:

- `chaps-v0.2.0-universal-apple-darwin.tar.gz`, both macOS slices in one
  binary, signed and notarized;
- `chaps-v0.2.0-aarch64-apple-darwin.tar.gz` and
  `chaps-v0.2.0-x86_64-apple-darwin.tar.gz`, likewise signed and notarized;
- `chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz` and
  `chaps-v0.2.0-aarch64-unknown-linux-musl.tar.gz`, static;
- `chaps-v0.2.0-x86_64-pc-windows-msvc.zip` and
  `chaps-v0.2.0-aarch64-pc-windows-msvc.zip`, unsigned;
- a `.sha256` next to each archive, and one `SHA256SUMS` covering all seven;
- release notes listing the archives, followed by GitHub's generated notes.

The signing steps live in `.github/actions/sign-macos`, a composite action
used by both the per-architecture build and the universal one, so the two
cannot drift apart. It imports the certificate into a throwaway keychain,
signs with `--options runtime --timestamp`, submits to `notarytool --wait` and
fails the job on any status other than `Accepted`, printing `notarytool log`
first. The universal binary is signed again after `lipo`, because fusing the
two binaries invalidates the per-slice signatures. Nothing is stapled: a ticket
can only be stapled to a bundle, a disk image or an installer package, so
Gatekeeper fetches it from Apple instead. The keychain is deleted in an
`if: always()` step in the calling job.

### Verifying a download

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
codesign -dv --verbose=4 chaps
```

The second prints `Authority=Developer ID Application: ...` and a secure
timestamp for a macOS binary. `spctl --assess --type execute` is not a useful
check here: it assesses app bundles and reports a plain command-line
executable as rejected even when the signature and the notarization are both
good. The same `codesign` output is written to the workflow's job summary, so
a release can be audited without downloading anything.

### The docs deploy

Pushing the release commit to `main` also runs `docs.yml`, which rebuilds the
book and deploys it to <https://winterop-com.github.io/chaps/>. There is
nothing to do by hand; the version in the install commands is written out in
`docs/install.md` and is updated there when it matters.

### Repository setup

`scripts/bootstrap-repo.sh` is the one-time setup for a fresh GitHub
repository. It prints every command and executes nothing until you pass
`--yes`:

```sh
scripts/bootstrap-repo.sh          # print the commands
scripts/bootstrap-repo.sh --yes    # run them
```

It creates `winterop-com/chaps` as a public repository from this checkout,
pushes `main`, switches GitHub Pages to the Actions build source (which has to
happen before `docs.yml` can deploy), sets the About link to the documentation
site, and runs `scripts/github-secrets.sh`.
