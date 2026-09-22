# chaps

`chaps` generates and manages a Docker Compose deployment of
[chap-core](https://github.com/dhis2-chap/chap-core) together with the
forecasting model services published in the
[CHAP model marketplace](https://github.com/dhis2-chap/model-marketplace).

One command writes a self-contained deployment directory: the base stack
(chap-core, its worker, Valkey and PostgreSQL), one Compose overlay per enabled
model, an `.env` file, and a `chaps.json` state file that records exactly what
was generated. After that, `chaps` is a thin wrapper around `docker compose`
that always passes the explicit `-f` list, plus a model manager that can add or
remove models without you hand-editing YAML.

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
chaps up                             # docker compose up -d
chaps status                         # chap-core health and registered models
chaps models tui                     # browse the marketplace, toggle models
chaps up                             # apply what the browser changed
```

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
  models enable ID        write the overlay and record it in chaps.json
      --channel stable|latest | --version X
      --port N  --data-dir PATH  --user USER:GROUP  --allow-template
  models disable ID       remove the overlay and the chaps.json entry
  models tui              the model browser (also `chaps tui`)

  registry update         fetch the catalogue now and refresh the cache
  registry show           where the catalogue came from and what it holds

  up [--attach] [EXTRA..] docker compose up -d (or up, attached)
  down [EXTRA..]          docker compose down
  ps [EXTRA..]            docker compose ps
  logs [-f] [SERVICE..]   docker compose logs
  pull                    docker compose pull
  compose -- ARGS..       any other docker compose command

  status [--url URL] [--timeout SECONDS]
                          GET /health and /v2/services, and diff the registered
                          services against the ones this project enabled
```

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

| File | What it is |
| --- | --- |
| `compose.yml` | The base stack: chap-core, worker, Valkey, PostgreSQL. A copy of chap-core's `compose.ghcr.yml`, so upstream stays the source of truth. |
| `.env` | PostgreSQL credentials (the password is 32 random hex characters generated once), the chap-core image tag, and commented placeholders for `CHAP_API_TOKEN`, `SERVICEKIT_REGISTRATION_KEY`, `CHAP_DATABASE_URL` and per-model image pins. Written by `init` only. |
| `compose.marketplace.yml` | An umbrella file whose `include:` list names one overlay per enabled model. With no models enabled it holds `services: {}` instead of an empty `include`. |
| `compose.<service_id>.yml` | One model service. Regenerated by `models enable` and removed by `models disable`. |
| `chaps.json` | The state file: schema version, the ordered `-f` list, the port range, and every enabled model with its image, pinned version, channel, host port, data directory and user. |

Compose reads `.env` automatically, so `${CHAP_IMAGE_TAG:-latest}` and the
per-model `${<ID>_IMAGE_TAG:-sha-xxxxxxx}` pins can be overridden there without
regenerating anything.

## How model overlays work

Each marketplace model is a [chapkit](https://github.com/dhis2-chap/chapkit)
container listening on port 8000 with `/health` and `/api/v1/info`. The overlay
`chaps` writes does five things:

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
  port 8000. Ports are allocated from `chaps.json` plus a scan of every
  `compose*.yml` in the directory, so two models never collide; `--port` claims
  one explicitly.
- **Hardening**, matching the posture of the base stack: `init: true`,
  `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, an unprivileged
  `user`, a 2 GB tmpfs at `/tmp`, and a named volume for the model's data
  directory (the only writable path besides `/tmp`).
- **Ordering.** `depends_on: chap: condition: service_healthy`, so a model only
  starts once chap-core can accept its registration. The overlay adds no
  `healthcheck` (chapkit images ship their own) and no `networks:` key (the
  default network reaches chap-core but not PostgreSQL or Valkey).

Models built on the R-INLA runtime also get `platform: linux/amd64`; they have
no arm64 build and would otherwise fail to start on Apple silicon.

The data directory and the user differ per image (`/app/data` with
`chapkit:chapkit` for EWARS, `/app/data` with `chap:chap` for the simple
multistep model, `/work/data` with `chapkit:chapkit` elsewhere). `chaps` knows
the published ones; `--data-dir` and `--user` cover anything it does not. A
model that crash-loops right after starting is almost always writing outside
its data directory on the read-only filesystem.

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
forces a fetch and refreshes the cache. `--offline` never touches the network,
so a laptop on a plane still resolves the catalogue from the cache or the
embedded snapshot.

The cache lives in `$CHAPS_CACHE_DIR`, else `$XDG_CACHE_HOME/chaps`, else
`~/.cache/chaps`; `--cache-dir` overrides it for one invocation.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
scripts/vendor-marketplace.sh   # refresh the embedded marketplace snapshot
```

CI runs the same three checks on Linux, macOS and Windows. Tagging `vX.Y.Z`
builds release binaries for six targets (Linux, macOS and Windows, on x86_64
and aarch64) and attaches them to a GitHub release with their SHA-256 sums.
