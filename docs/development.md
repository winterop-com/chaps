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

## The build script

`build.rs` produces three things the compiler cannot work out on its own.

- `$OUT_DIR/embedded_files.rs`, the `(relative path, contents)` table that
  `src/registry/embedded.rs` includes. It is generated from
  `vendor/marketplace/registry.yaml` and the model files that index lists, in
  index order, so there is no hand-written list to keep in step. A file the
  index names but the directory does not hold fails the build with a message
  naming it; a file on disk the index does not list is a `cargo:warning` and
  is left out.
- `TARGET`, the triple this binary is built for, which `chaps self update`
  reads to pick its release asset and `chaps self version` prints.
- `GIT_REVISION`, the short commit of the checkout, empty when there is no git
  or no repository, which `chaps self version` prints when it is there.

The script names its own `rerun-if-changed` files, so it re-runs when the
vendor directory or `.git/HEAD` moves and not otherwise.

## Vendoring the marketplace snapshot

The catalogue compiled into the binary is the last fallback when the registry
can be neither fetched nor read from cache, so `chaps` works on a machine that
has never had network access.

```sh
make vendor
```

runs `scripts/vendor-marketplace.sh`, which refreshes the files under
`vendor/marketplace/`. A model added or dropped upstream needs nothing else:
`build.rs` reads the index and embeds what it lists, and `cargo test registry`
is what checks the result.

The vendored layout is the same as the on-disk cache layout, so
`vendor/marketplace/` and a cache entry are interchangeable.

`.github/workflows/vendor-marketplace.yml` does the same thing every Monday at
06:00 UTC, and on demand through `workflow_dispatch`. It runs the script, runs
`cargo test registry`, and opens or updates a pull request titled `chore:
refresh embedded marketplace snapshot` on the `chore/vendor-marketplace` branch
when anything under `vendor/marketplace/` changed. Nothing is pushed to `main`:
these files are compiled in, so a snapshot change is a code change and goes
through review like one.

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
request, and a shellcheck job over `install.sh` (as POSIX `sh`) and
`scripts/*.sh` (as bash).

`.github/workflows/release.yml` builds release binaries for six targets
(Linux, macOS and Windows, on x86_64 and aarch64) when a `vX.Y.Z` tag is
pushed, fuses the two macOS builds into a seventh universal archive, and
attaches them all to a GitHub release with their SHA-256 sums. Every target is
built natively, so there is no cross-compilation toolchain to maintain, the
freshly built binary generates its own completion scripts into the archive,
and the Linux builds are static musl so one binary runs on any distribution. A
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

### Preparing the Apple certificate

`APPLE_CERTIFICATE` is the base64 of a `.p12` archive holding the Developer ID
private key, the certificate downloaded from the developer portal and Apple's
intermediate, so the chain in the archive is complete:

```sh
# the Developer ID certificate from the portal, DER, next to its private key
openssl x509 -inform DER -in developer_id.cer -out developer_id.pem

# Apple's Developer ID G2 intermediate, which the portal does not include
curl -fsSL -o DeveloperIDG2CA.cer https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer
openssl x509 -inform DER -in DeveloperIDG2CA.cer -out DeveloperIDG2CA.pem

openssl pkcs12 -export -legacy -inkey developer_id.key -in developer_id.pem -certfile DeveloperIDG2CA.pem -name "Developer ID Application: <name> (<TEAMID>)" -out developer_id.p12
```

`-legacy` is not optional. OpenSSL 3 exports with AES-256-CBC, PBKDF2 and an
SHA-256 MAC by default, and the `security` tool on the GitHub macOS runner
cannot read that: the import fails with

```text
security: SecKeychainItemImport: MAC verification failed during PKCS12 import (wrong password?)
```

with a password that is perfectly correct. `-legacy` writes the older encoding
`security` accepts. Exporting from Keychain Access instead of OpenSSL is also
fine; it already writes the compatible encoding.

Verify the archive locally before uploading it, in a throwaway keychain rather
than the login one:

```sh
security create-keychain -p "" /tmp/verify.keychain
security import developer_id.p12 -k /tmp/verify.keychain -P '<password>' -T /usr/bin/codesign
security find-identity -v -p codesigning /tmp/verify.keychain
security delete-keychain /tmp/verify.keychain
```

`find-identity` has to print exactly one valid identity; that line is what
`APPLE_SIGNING_IDENTITY` has to match. If it prints none, the runner will fail
in the same way, whatever the password is.

The base64 of that file is `APPLE_CERTIFICATE` and the password chosen on
export is `APPLE_CERTIFICATE_PASSWORD`:

```sh
base64 -i developer_id.p12 | pbcopy
```

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
- a version-less copy of each archive, e.g.
  `chaps-universal-apple-darwin.tar.gz`, byte for byte the same file;
- a `.sha256` next to each versioned archive, and one `SHA256SUMS` covering
  every archive, version-less copies included;
- release notes listing the archives, followed by GitHub's generated notes.

Each archive holds one directory, `chaps-<version>-<target>/`, with the binary,
`README.md`, `LICENSE` and `completions/` holding `chaps.bash`, `_chaps`,
`chaps.fish` and `_chaps.ps1`, generated by running `chaps completions` on the
binary that was just built.

The version-less copies exist because `releases/latest/download/<name>` needs a
file name that is the same in every release, and every archive this project
publishes carries the tag in its name. Copying each archive under a second name
costs one `cp` in the workflow and makes the download table in the README and
in [Install](./install.md) a set of links that never go stale.

### The install script

`install.sh` at the repository root is what
`curl -fsSL .../install.sh | sh` runs. POSIX `sh` rather than bash, so it works
on a minimal container image, and shellcheck-clean under `-s sh` in CI.

It resolves the platform from `uname -s` and `uname -m`, resolves the version
from `CHAPS_VERSION`, then the GitHub API, then the redirect of the
`releases/latest` page, downloads the archive and `SHA256SUMS` into a `mktemp`
directory, verifies with whichever of `sha256sum` and `shasum` exists, and
installs with `install -m 0755`. Completion scripts are copied into the
standard user directories that already exist, and a failure there never fails
the install.

`CHAPS_DOWNLOAD_BASE` overrides where the archive and `SHA256SUMS` come from.
It is there to test the script against a locally built release and an install
never needs it. `curl` reads `file://`, so the whole path can be exercised
without a release:

```sh
name="chaps-v0.0.0-test-$(uname -m | sed 's/arm64/universal/')-apple-darwin"
mkdir -p /tmp/rel/"$name"
cp target/debug/chaps README.md LICENSE /tmp/rel/"$name"/
tar -czf /tmp/rel/"$name".tar.gz -C /tmp/rel "$name"
(cd /tmp/rel && shasum -a 256 "$name".tar.gz > SHA256SUMS)

CHAPS_DOWNLOAD_BASE=file:///tmp/rel \
  sh install.sh --version v0.0.0-test --dir /tmp/bin
```

`--dry-run` prints the platform, the archive URL and the target path and stops
there, which is the quickest check that the detection is right on a machine.

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
