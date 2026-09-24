# Models and the marketplace

## Where the catalogue comes from

The catalogue is the YAML in the model-marketplace repository, read from

```text
https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml
```

`--registry-url` points somewhere else, for a fork or a mirror. The
marketplace has no JSON API, so the index is one request and each model file it
lists is one more; the requests are sequential, which is fast enough for a
handful of small files and makes a failure easy to attribute.

Resolution order for every command that needs the catalogue:

1. a cached copy younger than 24 hours,
2. the network,
3. a stale cached copy, when the network fails,
4. the snapshot compiled into the binary.

`chaps registry show` prints which of those was used:

```text
  url     https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml
  source  cache (56 minutes old)
  models  6
  cache   ~/.cache/chaps/registries/https_raw_githubusercontent_com_dhis2_chap_model_marketplace_mai-f6e5270ab795f1bd

  chapkit_ewars_model
  chapkit_rwanda_malaria_bym_model
  chapkit_simple_multistep_model
  auto_arima_chapkit
  chapkit_minimalist_example_py
  chapkit_minimalist_example_r

6 models in the catalogue (cache (56 minutes old)); `chaps models info ID` describes one
```

`chaps registry update` forces a fetch and refreshes the cache, as does
`chaps update`. `--offline` never touches the network, so a laptop on a plane
still resolves the catalogue from the cache or the embedded snapshot;
`chaps update` is the one command that refuses to run that way.

The cache lives in `$CHAPS_CACHE_DIR`, else `$XDG_CACHE_HOME/chaps`, else
`~/.cache/chaps`; `--cache-dir` overrides it for one invocation. A private
marketplace and the public one never share a cache entry, because the entry is
keyed by the registry URL. Every cache read failure is a warning, never an
error: a missing, truncated or hand-edited cache just means the fetch or the
embedded snapshot answers instead.

The embedded snapshot is the last resort, so `chaps` works on a machine that
has never had network access. It is refreshed with `make vendor`.

## Channels and versions

Each marketplace entry lists versions, and the channels `stable` and `latest`
each point at one of them.

```sh
chaps models enable chapkit_ewars_model                       # stable, the default
chaps models enable chapkit_ewars_model --channel latest
chaps models enable chapkit_ewars_model --version 1.0.0       # an exact pin
```

A model that follows a channel records both the channel and the version it
resolved to, and `chaps update` re-resolves the channel. A model enabled with
`--version` is listed as pinned by `update` and never moved. `--channel` and
`--version` cannot be combined.

## Templates

Entries with `kind: template` are scaffolding for writing your own model, not
forecasting models. They are hidden from `chaps models list` and from the
browser by default, `--templates` and `--all` show them, and enabling one needs
`--allow-template`.

## Looking at the catalogue

```sh
chaps models list                 # models enabled in this project are marked
chaps models list --enabled       # only this project's
chaps models list --all           # templates too
chaps models search malaria
chaps models info chapkit_ewars_model
```

```text
ID                                SERVICE                           NAME                STATUS  STABLE  LATEST  ENABLED
chapkit_ewars_model               chapkit-ewars-model               CHAP-EWARS          orange  1.0.0   1.0.0   internal
chapkit_rwanda_malaria_bym_model  chapkit-rwanda-malaria-bym-model  Rwanda Malaria BYM  gray    0.1.0   0.1.0   -
chapkit_simple_multistep_model    chapkit-simple-multistep-model    Simple Multistep    orange  0.1.0   0.1.0   -
auto_arima_chapkit                auto-arima-chapkit                Auto-ARIMA          red     1.0.0   1.0.0   -

4 listed, 1 enabled in this project
```

The `ENABLED` column is the host port, `internal` for an enabled model with no
host port of its own, or `-` for one this project does not enable.

`chaps models info ID` prints everything the marketplace says about one entry:
its image and runtime, the period types and forecast horizon it supports, the
covariates it requires and defaults to, the channels and the version table,
the maintainers and the attribution, and any named configurations. For a model
this project has enabled with no host port, it also prints the chap-core proxy
URL that reaches it.

Both `ID` forms work everywhere an id is taken: the marketplace id
(`chapkit_ewars_model`, with underscores) and the service id
(`chapkit-ewars-model`, with hyphens, which is also the Compose service name
and the DNS name).

## Enabling and disabling

```sh
chaps models enable ID [--channel stable|latest | --version X]
                       [--port N|auto] [--data-dir PATH] [--user USER:GROUP]
                       [--allow-template]
chaps models disable ID [--purge]
chaps models expose ID [--port N|auto]
chaps models unexpose ID
chaps up                            # apply any of them
```

`enable` and `disable` edit `.chaps/models.yaml` and then re-render the compose
files; `expose` and `unexpose` only rewrite the overlay's `ports:`, so they
never re-resolve the version and are safe on a deployment running a build you
do not want moved. The browser is the same: moving a port there keeps the
version the model is pinned to, and only a toggle or a new channel resolves the
marketplace again.

`disable` also stops and removes the model's container when one is running,
before the definition goes away, and says so:

```text
disabled chapkit_ewars_model
removed compose.chapkit-ewars-model.yml
note: stopped and removed the chapkit-ewars-model container; the host port it published is free again
note: kept volume mychap-1ab2c3_ck_chapkit_ewars_model_data; remove it with `chaps models disable chapkit_ewars_model --purge` or `docker volume rm mychap-1ab2c3_ck_chapkit_ewars_model_data`
run `chaps up` to apply
```

The port is therefore free straight away, rather than at the next `chaps up`,
which would have removed the container as an orphan but only once it was run.

### The data volume

What `disable` does not take is the model's data: the named volume
`<compose project>_ck_<id>_data` stays exactly as it was, so enabling the model
again finds everything it had. That is why the line above names it. Nothing
else does any more - the overlay that declared the volume has just been
removed, so `chaps docker run -- down -v` no longer knows about it, and a
volume nobody names again is kept for as long as the machine lasts.

`--purge` is how the data goes with the model:

```sh
chaps models disable chapkit_ewars_model --purge
```

```text
disabled chapkit_ewars_model
removed compose.chapkit-ewars-model.yml
note: removed volume mychap-1ab2c3_ck_chapkit_ewars_model_data
run `chaps up` to apply
```

The volume is removed after the container, which is the only order docker
allows: a volume a container still has mounted cannot be removed. Everything
about it is best-effort and reported rather than fatal - `volume <name> not
found` for a deployment that was never started, and `volume <name> could not
be removed: <docker's own words>` for anything else. `--json` carries the same
answer as two lists, `purged` and `kept_volumes`.

`--purge` also works on a model that is not enabled any more, which is the
state the kept-volume line leaves you in:

```text
note: chapkit_ewars_model is not enabled here, so only its data volume was looked for
note: removed volume mychap-1ab2c3_ck_chapkit_ewars_model_data
```

`chaps doctor`'s `volumes` line finds the ones that were forgotten: a volume
under this deployment's name that belongs to no enabled model and no enabled
component is reported as a leftover, with the same two ways to remove it. See
[Doctor](./doctor.md).

```text
exposed chapkit-ewars-model on http://localhost:5001
written  compose.chapkit-ewars-model.yml
run `chaps up` to apply
```

```text
unexposed chapkit-ewars-model; it stays registered with chap-core and reachable at http://localhost:8000/v2/services/chapkit-ewars-model/run/
written  compose.chapkit-ewars-model.yml
run `chaps up` to apply
```

## The model browser

`chaps ui` opens a two-pane browser: the catalogue on the left, the details of
the selected entry on the right. None of the keys need Alt on a Norwegian
keyboard:

```text
j / k / arrows     move            space   enable or disable
g / G              first / last    p       publish a host port, or stop
PageUp / PageDown  jump a page     v       switch channel (stable, latest)
ctrl-u / ctrl-d    jump a page     t       show or hide templates
Enter / s          save            /       filter (Enter keeps, Esc clears)
q / Esc            quit            ?       help
y / n              answer the quit confirmation
```

`ctrl-n` and `ctrl-p` also move down and up, and `ctrl-c` always leaves.
While filtering, the arrow keys still move the cursor and `ctrl-u` empties the
filter without leaving it.

The port column reads `internal` for an enabled model with no host port,
`:5001` for one that has had a port allocated, and `:auto` for a row where `p`
has asked for one that is not picked until you save.

Saving applies the accumulated changes through the same write path as
`chaps models enable`, then prints what changed. Nothing is written until you
save, and quitting with unsaved changes asks first. `chaps ui` owns the
terminal, so it rejects `--json`.

`chaps init --interactive` opens the same browser to pick the initial model
set instead of reading `--models`.

## How a model overlay looks

Each marketplace model is a [chapkit](https://github.com/dhis2-chap/chapkit)
container listening on port 8000 with `/health` and `/api/v1/info`. This is the
overlay `chaps` writes for the default model, annotated by the generator
itself:

```yaml
# compose.chapkit-ewars-model.yml - generated by chaps (0.1.0); rendered from .chaps/models.yaml by `chaps sync`.
# Model: CHAP-EWARS (chapkit_ewars_model) v1.0.0 - https://github.com/chap-models/chapkit_ewars_model
# Pinned to a sha-<short commit> build from the CHAP model marketplace so the deployment stays
# reproducible; set CHAPKIT_EWARS_MODEL_IMAGE_TAG in .env to move the pin.
services:
  chapkit-ewars-model:
    restart: unless-stopped
    image: ghcr.io/chap-models/chapkit_ewars_model:${CHAPKIT_EWARS_MODEL_IMAGE_TAG:-sha-fa880a1}
    # Marketplace images are published for linux/amd64 (like chap-core itself);
    # the pin makes an arm64 host pull that variant instead of failing.
    platform: linux/amd64
    init: true
    # No host port: chap-core reaches this service at http://chapkit-ewars-model:8000 on the
    # compose default network. `chaps models expose chapkit-ewars-model` publishes one for
    # curl and the model's own /docs page.
    expose:
      - "8000"
    environment:
      # $$ is a literal $ for compose; the service self-registers with chap-core on startup.
      SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register
      # Uncomment if chap has SERVICEKIT_REGISTRATION_KEY set:
      # SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}
    # Same posture as chap and worker: unprivileged user, read-only root filesystem,
    # writable only at /app/data (SQLite) and /tmp (model workspaces).
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    user: chapkit:chapkit
    volumes:
      - type: tmpfs
        target: /tmp
        tmpfs:
          size: 2000000000 # 2GB for model workspaces
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    # No `networks:` on purpose: the default network reaches chap but not redis/postgres.
    # No healthcheck: chapkit images ship their own HEALTHCHECK.
    depends_on:
      chapkit-ewars-model-init:
        condition: service_completed_successfully
      chap:
        condition: service_healthy

  chapkit-ewars-model-init:
    # Fresh named volumes are root-owned unless the image pre-creates the data
    # dir; the model runs as an unprivileged user, so hand the volume over first.
    # Without this, chapkit dies on "unable to open database file".
    # This is the one service that is not `restart: unless-stopped`: it must run
    # once per start and exit. (`no` has to be quoted; bare no is YAML false.)
    image: busybox:1.37
    command: ["sh", "-c", "chown 1000:1000 /app/data"]
    user: "0:0"
    volumes:
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    restart: "no"

volumes:
  ck_chapkit_ewars_model_data: {}
```

The overlay does six things.

- **Pins the image.** `image: <repo>:${<ID>_IMAGE_TAG:-sha-<commit>}`, where the
  default is the `image_tag` of the version the channel resolves to. The
  deployment stays reproducible, and a different build is one `.env` line away.
- **Self-registration.** The service registers itself with chap-core through
  `SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register` (the
  `$$` is a literal `$` for Compose). The Compose service name, the DNS name
  and the marketplace `service_id` are the same string, which is what makes the
  registration resolvable. Set `SERVICEKIT_REGISTRATION_KEY` in `.env` to
  require a shared secret.
- **Publishes no host port.** The service gets `expose: ["8000"]` and nothing
  else. See [Ports](./ports.md).
- **Hardening**, matching the posture of the base services: `init: true`,
  `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, an unprivileged
  `user`, a 2 GB tmpfs at `/tmp`, and a named volume for the model's data
  directory (the only writable path besides `/tmp`).
- **Hands the data volume to the model user**, through the init container
  below.
- **Ordering.** `depends_on`: the init container with
  `condition: service_completed_successfully` and `chap` with
  `condition: service_healthy`, so a model only starts once its volume is
  writable and chap-core can accept its registration. The overlay adds no
  `healthcheck` (chapkit images ship their own) and no `networks:` key (the
  default network reaches chap-core but not PostgreSQL or Valkey).

## The init container, and why

Docker seeds a fresh named volume from whatever the image has at the mount
point, ownership included, so an image that never creates its data directory
yields a root-owned volume the unprivileged model cannot write to, and chapkit
dies on `sqlite3.OperationalError: unable to open database file`.

Compose cannot `chown` a volume, so each overlay ships a one-shot
`<service_id>-init` container (busybox, as root, `restart: "no"`) that chowns
the mount point to the model's **numeric** uid:gid before the model starts.
Busybox resolves no `chapkit` account, which is why the numbers matter. Every
overlay gets this container, not only the images known to need it, and it is
the one service in a deployment that is deliberately not
`restart: unless-stopped`.

It has a second job: because it mounts the same named volume at the same path
as the model itself, `chaps backup` reads and writes model data through it,
whether the model is running, stopped, or brought down entirely. See
[Backup and restore](./backup.md).

## Data directories and users

The data directory and the user differ per image:

| Image | Data directory | User |
| --- | --- | --- |
| EWARS | `/app/data` | `chapkit:chapkit` |
| The simple multistep model | `/app/data` | `chap:chap` |
| Everything else | `/work/data` | `chapkit:chapkit` |

`chaps` knows the published ones; `--data-dir` and `--user` cover anything it
does not. A model that crash-loops right after starting is almost always
writing outside its data directory on the read-only filesystem.

The init container needs those names as numbers: `chapkit` is uid/gid 1000 and
`chap` is 1001. A `--user` that is already numeric is passed through, and a
name `chaps` does not know falls back to `1000:1000` with a warning from
`chaps sync`.

## The amd64 pin, and why

Every overlay also pins `platform: linux/amd64`. The marketplace images are
published for amd64, as is chap-core itself, so an arm64 host such as Apple
silicon pulls that variant and runs it under emulation instead of failing with
`no matching manifest for linux/arm64`.
