# chaps

`chaps` generates and manages a Docker Compose deployment of
[chap-core](https://github.com/dhis2-chap/chap-core) together with the
forecasting model services published in the
[CHAP model marketplace](https://github.com/dhis2-chap/model-marketplace).

One command writes a self-contained deployment directory: the base stack
(chap-core, its worker, Valkey and PostgreSQL), one Compose overlay per enabled
model, an `.env` file, and a `.chaps/` directory that records exactly what the
deployment is meant to be. After that, `chaps` is a thin wrapper around
`docker compose` that always passes the explicit `-f` list, plus a model manager
that can add or remove models without you hand-editing YAML.

Every command takes `--json` for machine-readable output.

## Install

From a checkout:

```sh
cargo install --path .
```

Or download a prebuilt archive for your platform from the
[releases page](https://github.com/mortenoh/chaps-cli/releases) and put the
`chaps` binary on your `PATH`. Archives exist for Linux (static musl builds),
macOS and Windows, each on amd64 and arm64, and each ships with a `.sha256`
file. On a Linux server:

```sh
curl -fsSLO https://github.com/mortenoh/chaps-cli/releases/latest/download/chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf chaps-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sudo install -m 0755 chaps-v0.1.0-x86_64-unknown-linux-musl/chaps /usr/local/bin/chaps
```

Requirements on the machine that runs the stack: Docker with Compose v2.20 or
newer (`include:` support). Nothing else: no Python, no uv, no checkout of
chap-core. `chaps` warns on stderr when the installed Compose is older.

### A note on the name

The binary is `chaps`, not `chap`, because chap-core's own Python package
already installs a console script called `chap`. The two never collide.

## Quickstart

```sh
chaps init mychap --models default   # writes the deployment directory
cd mychap
chaps up                             # sync the compose files, docker compose up -d
chaps status                         # chap-core health and registered models
chaps models tui                     # browse the marketplace, toggle models
chaps up                             # apply what the browser changed
chaps update                         # move channel pins to what the marketplace publishes now
```

Every command that operates on a project finds it the way git finds `.git`:
from the current directory (or `-C DIR`) upwards to the nearest `.chaps/`, so
`chaps up` works from any subdirectory of the deployment.

`--models default` enables `chapkit_ewars_model` on its `stable` channel.
`--models none` writes the base stack only, and `--models a,b` takes an
explicit list of marketplace ids.

chap-core's API is on <http://localhost:8000>; each model service gets its own
host port from 5001 upwards.

## Commands

```
chaps [--json] [-C DIR] [--registry-url URL] [--offline] [--cache-dir DIR] <command>

  init [DIR]              create a deployment directory
      --models none|default|id,id   which models to enable (default: default)
      --chap-tag TAG                image tag for the chap-core services
      --port-base PORT              lowest host port for model overlays
      --force                       overwrite an existing project
      --no-env                      do not write .env

  models list             list marketplace models (--all, --templates, --enabled)
  models search QUERY     search id, name and summary
  models info ID          everything known about one model
  models enable ID        record the model in .chaps/models.yaml and write its overlay
      --channel stable|latest | --version X
      --port N  --data-dir PATH  --user USER:GROUP  --allow-template
  models disable ID       drop the model from .chaps/models.yaml and remove its overlay
  models tui              the model browser (also `chaps tui`)

  registry update         fetch the catalogue now and refresh the cache
  registry show           where the catalogue came from and what it holds

  sync [--check]          render the compose files from .chaps/; --check writes
                          nothing and exits non-zero if anything would change
  update [--dry-run] [--no-restart]
                          fetch the registry, move channel-following models to
                          the version their channel now points at, sync, pull,
                          up -d

  up [--attach] [--pull] [EXTRA..]
                          sync, then docker compose up -d (or up, attached);
                          --pull passes --pull always
  down [EXTRA..]          docker compose down
  logs [-f] [SERVICE..]   docker compose logs

  docker ps [EXTRA..]     list this project's containers (docker compose ps)
  docker pull             download the pinned images into the local Docker
                          daemon; no files change
  docker exec SERVICE [CMD..]
                          run a command in a running container; CMD defaults to
                          a shell, and -T is passed for you when there is no
                          terminal, so it works in scripts
  docker run -- ARGS..    any docker compose command, behind the project's -f list
  docker config [-- EXTRA..]
                          the finished stack: every compose file merged into one
                          document (--json prints it as JSON)

  status [--url URL] [--timeout SECONDS]
                          GET /health and /v2/services, and diff the registered
                          services against the ones this project enabled
```

The everyday verbs are at the top level; the Docker plumbing you only reach for
when you already know what Docker is doing lives under `chaps docker`, so the
top-level list stays readable if you have never used Compose.

`chaps --help` adapts to where you are: outside a deployment directory it lists
only the commands that can work there (`init`, `models`, `registry`) and says
that the rest appear inside one. The hidden commands still run if you type
them; they just tell you there is no project.

The wrappers always run `docker compose -f <dir>/compose.yml -f
<dir>/compose.marketplace.yml ...` from the project directory, so they behave
the same whatever your shell's working directory is, and they exit with
Compose's own exit code.

`chaps status` exits non-zero when the API is down or a model this project
enabled has not registered, which makes it usable as a health gate in a script.

### The model browser

`chaps models tui` opens a two-pane browser: the catalogue on the left, the
details of the selected entry on the right. Keys (none of them need Alt on a
Norwegian keyboard):

```
j / k / arrows     move            space   enable or disable
g / G              first / last    v       switch channel (stable, latest)
PageUp / PageDown  jump a page     t       show or hide templates
ctrl-u / ctrl-d    jump a page     /       filter (Enter keeps, Esc clears)
Enter / s          save            ?       help
q / Esc            quit            y / n   answer the quit confirmation
```

Saving applies the accumulated changes through the same write path as
`chaps models enable`, then prints what changed. Nothing is written until you
save, and quitting with unsaved changes asks first.

## Generated files

`chaps init` writes a directory you own; nothing outside it is touched.

```
mychap/
  .chaps/
    project.yaml               intent: schema version, chap-core tag, registry URL,
                               port range, the -f list, and which root files sync wrote
    models.yaml                intent: the enabled models (image, pinned version,
                               channel, host port, data dir, user, platform, overlay name)
  compose.yml                  base stack, written once by init
  compose.marketplace.yml      artifact: include: list, one line per enabled model
  compose.<service_id>.yml     artifact: one overlay per enabled model
  .env                         yours after init; chaps only appends pin comments
```

`.chaps/` is the intent. It is what `models enable`, `models disable`, the
browser and `update` edit, and it is small enough to read and to diff. The
compose files at the root are artifacts rendered from it by `chaps sync`, but
they are plain Compose files with nothing `chaps`-specific in them:
`docker compose -f compose.yml -f compose.marketplace.yml up -d` works without
`chaps` installed, which is the point of generating them.

`chaps up` runs `sync` before `docker compose up`, so the intent and the
artifacts never drift in normal use. `chaps sync --check` reports drift
without writing (for CI or a pre-commit hook), and a plain `chaps sync`
re-creates a deleted overlay or picks up a hand edit of `models.yaml`. Sync
only ever removes overlays it wrote itself (`project.yaml` keeps the list), so
a hand-written `compose.custom.yml` next to them is left alone; add it to the
umbrella by hand if you want it included.

| File | What it is |
| --- | --- |
| `compose.yml` | The base stack: chap-core, worker, Valkey, PostgreSQL. A copy of chap-core's `compose.ghcr.yml`, so upstream stays the source of truth. |
| `.env` | PostgreSQL credentials (the password is 32 random hex characters generated once), the chap-core image tag, and commented placeholders for `CHAP_API_TOKEN`, `SERVICEKIT_REGISTRATION_KEY`, `CHAP_DATABASE_URL` and per-model image pins. Written by `init`; afterwards `sync` appends missing pin comments and `update` moves the pin comments of models it changed. |
| `compose.marketplace.yml` | An umbrella file whose `include:` list names one overlay per enabled model. With no models enabled it holds `services: {}` instead of an empty `include`. |
| `compose.<service_id>.yml` | One model service, rendered from its `models.yaml` entry. |
| `.chaps/project.yaml`, `.chaps/models.yaml` | The intent, as above. Both open with a comment saying which commands manage them. |

Compose reads `.env` automatically, so `${CHAP_IMAGE_TAG:-latest}` and the
per-model `${<ID>_IMAGE_TAG:-sha-xxxxxxx}` pins can be overridden there without
regenerating anything.

### Updating

Model pins only move in two ways: `chaps models enable ID` (with `--channel`
or `--version`) and `chaps update`. Nothing else, `up` included, changes the
version a model runs.

`chaps update` fetches the registry from the network (no cache, no fallback;
it fails under `--offline`, `--dry-run` included, because a plan made from a
stale catalogue is not a plan). For every model that follows a channel it
re-resolves the channel; a pin that moved is recorded in `models.yaml` and its
`# <ID>_IMAGE_TAG=` comment in `.env` is updated. Models enabled with
`--version` are listed as pinned and skipped. Then it syncs, runs
`docker compose pull` and `docker compose up -d` (`--no-restart` stops after
the pull). `--dry-run` prints the plan and writes nothing.

chap-core itself follows `CHAP_IMAGE_TAG` (`latest` unless `init --chap-tag`
said otherwise), and Compose never re-pulls a tag it already has. The image is
refreshed only by `chaps docker pull`, `chaps update`, or `chaps up --pull`.

## How model overlays work

Each marketplace model is a [chapkit](https://github.com/dhis2-chap/chapkit)
container listening on port 8000 with `/health` and `/api/v1/info`. The overlay
`chaps` writes does six things:

- **Pins the image.** `image: <repo>:${<ID>_IMAGE_TAG:-sha-<commit>}`, where the
  default is the `image_tag` of the version the channel resolves to. The
  deployment stays reproducible, and a different build is one `.env` line away.
- **Self-registration.** The service registers itself with chap-core through
  `SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register` (the
  `$$` is a literal `$` for Compose). The Compose service name, the DNS name
  and the marketplace `service_id` are the same string, which is what makes the
  registration resolvable. Set `SERVICEKIT_REGISTRATION_KEY` in `.env` to
  require a shared secret.
- **Publishes a unique host port** in the range 5001-5999, mapped to container
  port 8000. Ports are allocated from `.chaps/models.yaml` plus a scan of every
  `compose*.yml` in the directory, so two models never collide; `--port` claims
  one explicitly.
- **Hardening**, matching the posture of the base stack: `init: true`,
  `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, an unprivileged
  `user`, a 2 GB tmpfs at `/tmp`, and a named volume for the model's data
  directory (the only writable path besides `/tmp`).
- **Hands the data volume to the model user.** Docker seeds a fresh named
  volume from whatever the image has at the mount point, ownership included, so
  an image that never creates its data directory yields a root-owned volume the
  unprivileged model cannot write to - and chapkit dies on
  `sqlite3.OperationalError: unable to open database file`. Compose cannot
  `chown` a volume, so each overlay ships a one-shot `<service_id>-init`
  container (busybox, as root, `restart: "no"`) that chowns the mount point to
  the model's *numeric* uid:gid (busybox resolves no `chapkit` account) before
  the model starts. Every overlay gets it, not only the images known to need
  it, and it is the one service in a deployment that is deliberately not
  `restart: unless-stopped`.
- **Ordering.** `depends_on`: the init container with
  `condition: service_completed_successfully` and `chap` with
  `condition: service_healthy`, so a model only starts once its volume is
  writable and chap-core can accept its registration. The overlay adds no
  `healthcheck` (chapkit images ship their own) and no `networks:` key (the
  default network reaches chap-core but not PostgreSQL or Valkey).

Every overlay also pins `platform: linux/amd64`. The marketplace images are
published for amd64 (as is chap-core itself), so an arm64 host such as Apple
silicon pulls that variant and runs it under emulation instead of failing with
"no matching manifest".

The data directory and the user differ per image (`/app/data` with
`chapkit:chapkit` for EWARS, `/app/data` with `chap:chap` for the simple
multistep model, `/work/data` with `chapkit:chapkit` elsewhere). `chaps` knows
the published ones; `--data-dir` and `--user` cover anything it does not. A
model that crash-loops right after starting is almost always writing outside
its data directory on the read-only filesystem. The init container needs those
names as numbers (`chapkit` is uid/gid 1000, `chap` is 1001); a `--user` that
is already numeric is passed through, and a name `chaps` does not know falls
back to `1000:1000` with a warning from `chaps sync`.

Templates (`kind: template`) are scaffolding for writing your own model, not
forecasting models. They are hidden from `models list` and the browser by
default, and enabling one needs `--allow-template`.

## The registry

The catalogue is the YAML in the model-marketplace repository, read from

```
https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml
```

(`--registry-url` points somewhere else, for a fork or a mirror.) Resolution
order for every command that needs the catalogue:

1. a cached copy younger than 24 hours,
2. the network,
3. a stale cached copy, when the network fails,
4. the snapshot compiled into the binary.

`chaps registry show` prints which of those was used; `chaps registry update`
forces a fetch and refreshes the cache, as does `chaps update`. `--offline`
never touches the network, so a laptop on a plane still resolves the catalogue
from the cache or the embedded snapshot (`update` is the one command that
refuses to run that way).

The cache lives in `$CHAPS_CACHE_DIR`, else `$XDG_CACHE_HOME/chaps`, else
`~/.cache/chaps`; `--cache-dir` overrides it for one invocation.

## Development

```sh
make test      # run the test suite
make check     # formatting and clippy, fixing nothing
make release   # universal (arm64+x86_64) macOS binary at bin/chaps
make install   # copy bin/chaps to $PREFIX/bin (PREFIX defaults to ~/.local)
make vendor    # refresh the embedded marketplace snapshot
```

`make help` lists every target. The underlying commands are `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check` and
`scripts/vendor-marketplace.sh`.

CI runs the same three checks on Linux, macOS and Windows. Tagging `vX.Y.Z`
builds release binaries for six targets (Linux, macOS and Windows, on x86_64
and aarch64) and attaches them to a GitHub release with their SHA-256 sums.
