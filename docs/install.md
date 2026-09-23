# Install

## The one-liner

On macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
```

Or, to get the binary and nothing else - into this directory as `./chaps`, no
`PATH` advice, no completion scripts, nothing written anywhere outside it:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh -s -- --here
```

Then run `chaps doctor`: it checks in one pass that this machine has everything
a deployment needs - Docker, Compose 2.20 or newer, disk, and a route to the
hosts CHAP pulls from - and says what to do about anything it did not find. See
[Doctor](./doctor.md).

The script works out the platform from `uname`, downloads the release archive
for it, checks the download against the release's `SHA256SUMS` and installs the
binary. It refuses to install anything it could not verify.

It goes into `/usr/local/bin` when that directory is writable and
`~/.local/bin` otherwise, creating the directory and saying so if the result is
not on your `PATH`. Pipe it through `sudo sh` to take the first branch on a
machine where `/usr/local/bin` needs root.

| Option | Environment variable | What it does |
| --- | --- | --- |
| `--version TAG` | `CHAPS_VERSION` | Install that release instead of the newest. `dev` is the rolling build of `main` |
| `--dir DIR` | `CHAPS_INSTALL_DIR` | Install into that directory |
| `--here` | `CHAPS_INSTALL_DIR=.` | Put `./chaps` in the current directory and do nothing else |
| `--dry-run` | | Print what would happen and change nothing |
| `--help` | | Print the options |

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh |
  CHAPS_INSTALL_DIR="$HOME/bin" CHAPS_VERSION=v0.1.0 sh
```

Without `--version` the newest release is looked up through the GitHub API, and
through the redirect of the `releases/latest` page when the API rate limit has
been reached. Unauthenticated API calls are limited to 60 an hour per address,
which a CI runner can exhaust.

There is no PowerShell installer yet. On Windows, take the zip from the table
below, unpack it and put `chaps.exe` on your `PATH`.

## Which version

There are two series, and a binary knows which one it belongs to.

**Stable** is a `vX.Y.Z` tag: a release that was cut on purpose, with notes,
and it never changes once published. It is what the one-liner installs, what
`chaps self update` follows, and what the download links point at.

**dev** is the rolling build of `main`, republished under the single moving
tag `dev` on every push. It is the same seven archives, built and signed the
same way, from a commit that has passed CI and nothing more: no release notes
worth the name, no promise that anything in it stays. It reports the version
of the last tag, so `chaps --version` says `v0.2.1` on a dev build as well;
`chaps self version` is what tells the two apart:

```text
  version       v0.2.1
  revision      bff294c
  channel       dev
```

`channel` is always printed, `stable` or `dev`, and is in `--json` under the
same name. A binary built anywhere else, `cargo install` included, is stable.

Picking one with the installer:

```sh
curl -fsSL .../install.sh | sh                        # the newest stable release
curl -fsSL .../install.sh | sh -s -- --version dev    # the rolling build of main
curl -fsSL .../install.sh | sh -s -- --version v0.2.0 # one named release
```

and with an installed `chaps`:

```sh
chaps self update                      # the newest build of the channel this one is on
chaps self update --version dev        # cross over to the rolling build
chaps self update --version v0.2.1     # go back to a stable release
```

`chaps self update` on a stable build follows `releases/latest`, which
excludes pre-releases, so a stable install is never moved onto a dev build by
itself. On a dev build the same command follows the `dev` release, and
compares the commit it was built from rather than the version number, since
the version does not move between two rolling builds. `--version TAG` always
wins over both.

## Downloading an archive

Each release attaches an archive per platform. These links always point at the
newest release:

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

An archive is named after the platform alone, with no version in it: the
release it belongs to is the one in the URL, so a link written down today
still resolves after the next release. A release attaches these seven archives
and one `SHA256SUMS` covering all of them, and nothing else.

An archive holds one directory, `chaps-<version>-<target>/`, containing the
`chaps` binary, `README.md`, `LICENSE` and a `completions/` directory. In a
dev archive that version is `dev-<short commit>`, which is the one place the
commit shows up without running the binary.

The Linux builds are static musl binaries, so one file runs on any
distribution with no shared library to match. Linux and macOS ship as
`tar.gz`, Windows as `zip`.

On macOS take the universal archive unless you have a reason to pick a single
architecture: it carries the Apple silicon and Intel slices in one binary and
runs on any Mac.

On macOS:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/latest/download/chaps-universal-apple-darwin.tar.gz
tar -xzf chaps-universal-apple-darwin.tar.gz
sudo install -m 0755 chaps-*-universal-apple-darwin/chaps /usr/local/bin/chaps
```

On a Linux server:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/latest/download/chaps-x86_64-unknown-linux-musl.tar.gz
tar -xzf chaps-x86_64-unknown-linux-musl.tar.gz
sudo install -m 0755 chaps-*-x86_64-unknown-linux-musl/chaps /usr/local/bin/chaps
```

## Verifying a download

Every release carries a `SHA256SUMS` file covering every archive:

```sh
curl -fsSLO https://github.com/winterop-com/chaps/releases/latest/download/SHA256SUMS
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
then "Run anyway" gets past it. There `SHA256SUMS` is the check that
matters.

## Keeping it up to date

```sh
chaps self update           # replace this binary with the newest release
chaps self update --check   # say whether there is one, change nothing
chaps self version          # version, revision, channel, target, path, install method
```

`chaps self update` looks up the release, downloads the archive built for this
target (on macOS the universal one, whichever slice you installed), checks it
against `SHA256SUMS` and renames the new binary over the running one. The
installed file is never written through, so a download that fails or does not
verify leaves what you have exactly as it was.

`--version TAG` installs a named release, going backwards included, and `dev`
is a tag like any other. `--yes` skips the confirmation, and `--json` reports
the result as a document, with `channel` alongside `current` and `latest`. If
the binary lives somewhere you cannot write, `chaps` says so and suggests
`sudo` or an install directory of your own rather than half-replacing itself.

Without `--version`, an update follows the channel this build is on: the
newest tag for a stable build, the newest build of `main` for a dev one. See
[Which version](#which-version).

Once a day, after a command that succeeded, `chaps` asks the release feed
whether there is a newer version and prints one dimmed line on stderr if there
is:

```text
chaps v0.2.0 is available (you have v0.1.0): run `chaps self update`
```

On a dev build the same notice compares commits, because the version number
does not move between two rolling builds:

```text
a newer chaps dev build is available (commit 9f3a2c1, you have bff294c): run `chaps self update`
```

It stays quiet unless it can see that the commit moved, so a build with no
recorded revision is never nagged about nothing.

The check has a two-second timeout, its answer is cached in
`<cache dir>/self-update-check.json`, and any failure is ignored: it can never
change what a command did or what it exited with. It is skipped under `--json`,
under `--offline`, when stdout is not a terminal, and for `chaps self` itself.
`CHAPS_NO_UPDATE_CHECK=1` turns it off entirely.

A `chaps` installed with `cargo install` is replaced the same way, but
`cargo install --path .` from an updated checkout is the more honest way to
move that one on. `chaps self version` says which of the two you have.

## Shell completions

Every release archive ships the scripts under `completions/`, and the install
script copies them into place when the directory a shell reads already exists.
`chaps completions <shell>` prints one to stdout for the cases it does not
cover: a checkout, a different directory, or a shell that reads its completions
from somewhere else.

bash:

```sh
chaps completions bash > ~/.local/share/bash-completion/completions/chaps
```

zsh, into a directory that is on your `fpath`:

```sh
chaps completions zsh > ~/.zsh/completions/_chaps
```

fish:

```sh
chaps completions fish > ~/.config/fish/completions/chaps.fish
```

PowerShell, appended to your profile:

```powershell
chaps completions powershell | Out-String | Invoke-Expression
```

elvish:

```sh
chaps completions elvish > ~/.config/elvish/lib/chaps.elv
```

The scripts are generated from the same command tree `--help` is rendered
from, so they are current for the binary that printed them. Regenerate them
after an update.

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
