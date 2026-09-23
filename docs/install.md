# Install

## The one-liner

On macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
```

The script works out the platform from `uname`, downloads the release archive
for it, checks the download against the release's `SHA256SUMS` and installs the
binary. It refuses to install anything it could not verify.

It goes into `/usr/local/bin` when that directory is writable and
`~/.local/bin` otherwise, creating the directory and saying so if the result is
not on your `PATH`. Pipe it through `sudo sh` to take the first branch on a
machine where `/usr/local/bin` needs root.

| Option | Environment variable | What it does |
| --- | --- | --- |
| `--version TAG` | `CHAPS_VERSION` | Install that release instead of the newest |
| `--dir DIR` | `CHAPS_INSTALL_DIR` | Install into that directory |
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
`chaps` binary, `README.md`, `LICENSE` and a `completions/` directory.

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
chaps self version          # version, revision, target, path, install method
```

`chaps self update` looks up the release, downloads the archive built for this
target (on macOS the universal one, whichever slice you installed), checks it
against `SHA256SUMS` and renames the new binary over the running one. The
installed file is never written through, so a download that fails or does not
verify leaves what you have exactly as it was.

`--version TAG` installs a named release, going backwards included.
`--yes` skips the confirmation, and `--json` reports the result as a document.
If the binary lives somewhere you cannot write, `chaps` says so and suggests
`sudo` or an install directory of your own rather than half-replacing itself.

Once a day, after a command that succeeded, `chaps` asks the release feed
whether there is a newer version and prints one dimmed line on stderr if there
is:

```text
chaps v0.2.0 is available (you have v0.1.0): run `chaps self update`
```

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
