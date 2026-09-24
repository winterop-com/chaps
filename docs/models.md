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
removed, so `chaps down --volumes` no longer knows about it, and a volume
nobody names again is kept for as long as the machine lasts.

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

## Testing a model

`chaps status` and `chaps doctor` can both be entirely green while a model
cannot produce a single prediction. Registration is a heartbeat: it says the
service is alive and talking to chap-core, not that its runtime works, that the
account it runs as can write, or that the covariates it declares are the ones
it reads. `chaps models test` is the check neither of them can make, because it
is the only one that makes the model do the work.

```sh
chaps models test ID..                 # one or more, by marketplace id or service id
chaps models test --all                # every model this deployment enables
chaps models test --all --backtest     # the whole way round, through chap-core
```

Models are tested one after another, and the exit code is non-zero only when
one of them **failed**: a model that could not be tested at all is a skip, and
a skip is counted rather than treated as a bad answer.

### The model level

The default. For each model, `chaps` runs chapkit's own end-to-end test inside
that model's container:

```text
docker compose exec -T <service> chapkit test --url http://127.0.0.1:8000 --timeout 300
```

chapkit reads the service's own configuration schema, creates a config from it,
generates data matching the covariates, period type and geometry the service
declares, and then validates, trains and predicts against it.

```text
testing 5 models (model level; add --backtest to run them through chap-core)
auto-arima-chapkit                  pass   12s   1 training, 1 prediction
chapkit-ewars-model                 pass   18s   1 training, 1 prediction
chapkit-ghr-model                   pass   24s   1 training, 1 prediction
chapkit-rwanda-malaria-bym-model    pass   22s   1 training, 1 prediction
chapkit-simple-multistep-model      pass    5s   1 training, 1 prediction

5 of 5 models pass
```

**What it proves:** the image runs, the service answers, the account the
container runs as can write to its data directory and its workspace, the
runtime has the libraries the model needs, and the model can turn a config and
a frame into a prediction. **What it does not touch:** chap-core. This is
therefore the level to reach for when chap-core is the thing that is broken,
and the level that answers in seconds rather than minutes.

A failure names the phase and the first sentence of the model's own error, out
of the stderr chapkit quotes back:

```text
chapkit-rwanda-malaria-bym-model    FAIL   14s   predict: Error in file(file, "rb") : cannot open file 'model.rds': Permission denied
  run `chaps models test chapkit_rwanda_malaria_bym_model -v` for the full output, and `chaps logs chapkit-rwanda-malaria-bym-model` for the service's own
```

A model that could not be tested is skipped, with the reason and the way out:

| Skip | What to do |
| --- | --- |
| `its container is not running` | `chaps up`. |
| ``the image has no `chapkit test`; the service reports chapkit 1.0.0`` | The image predates the command. `chaps update` moves the pin; `--backtest` tests it through chap-core instead. |
| `no answer in 5m` | The model is wedged or genuinely slow. `--timeout SECONDS` raises the limit; the same number is handed to chapkit as its per-job deadline. |

### The backtest level

`--backtest` goes the whole way round, the path the Modeling App takes. For
each model, `chaps` fetches sample data from the model itself through
chap-core's proxy (`GET .../api/v1/ml/$generate-sample-data?kind=train`,
36 periods over 5 org units, in the period type the service declares),
transposes that frame into chap-core observations, posts it as a dataset,
waits for the dataset job, runs a small rolling backtest over it (3 periods, 2
splits, stride 1) and reads the scores off the result.

Geometry is asked for whatever the service declares (`include_geo=true`).
chap-core's dataset always carries a GeoJSON collection, so a model that says
it needs no geometry would otherwise be handed features with none in them - and
a model that builds a neighbour graph fails on an empty polygon where it would
have been perfectly happy with no geometry at all. Real polygons are never
worse: a model that ignores geometry ignores these too. chapkit puts each
location's id in `properties.id` and nothing at the top level, and chap-core
matches org units on the feature's top-level `id` and silently drops the ones
it cannot match, so `chaps` sets it before posting.

```text
testing 2 models (through chap-core: a dataset, a backtest and its scores)
chapkit-rwanda-malaria-bym-model    pass   38s   crps 15.5  mae 23.8  rmse 26.9
chapkit-ewars-model                 pass   33s   crps 4.6  mae 6.6  rmse 9.7

2 of 2 models pass
```

**What it proves:** everything the model level does, plus that chap-core can
reach the model, that the covariates and period type the model declares are the
ones chap-core builds a dataset from, that the org units survive the round trip,
and that a real forecast comes back with scores on it. The three printed are
chap-core's `crps`, `mae` and `rmse`; `--json` hands back all fourteen.

A job that fails prints chap-core's own reason - the same line
[`chaps jobs logs`](./jobs.md) digs out of the `--- stderr ---` section - and
the command that shows the whole log:

```text
chapkit-ghr-model    FAIL   1m 12s   Error in predict_chap() : the inla program crashed
  run `chaps jobs logs 9a84e02f-6a0e-4a3b-9a63-1f2b5a1f3f21`
```

Scores are not a verdict on the model: a two-split backtest over generated data
says the pipeline works, not that the model is any good.

### What it leaves behind, and what it cleans up

Both levels put something in a database, and both delete it again unless
`--keep` says not to.

| Level | What it creates | What is deleted |
| --- | --- | --- |
| model | one config named `test_config_<ulid>` and a few artifacts, in the **model service's own** database | The service's configs and artifacts are listed before the run and again after it, and exactly what appeared is deleted. Deleting a config cascades to the artifacts linked to it. |
| `--backtest` | one dataset and one backtest row in **chap-core's** database, plus the jobs that made them | The backtest first, then the dataset, and nothing else. The jobs stay, because that is the record of what was run; they show up in [`chaps jobs`](./jobs.md). |

chap-core's model proxy is read-only by design, so a delete cannot go through
it: the model level lists through the proxy and deletes with the `curl` every
chapkit image carries for its healthcheck (`docker compose exec -T <service>
curl -X DELETE ...`). If a delete does not work, the command says so and names
what was left, so it can be removed by hand.

`--keep` keeps it, and says what was kept:

```text
kept backtest 8 and dataset 7; remove them with `chaps api DELETE /v1/crud/backtests/8` then `chaps api DELETE /v1/crud/datasets/7`
```

A backtest chap-core ran also leaves a configuration in the model service's own
database, the way any backtest started from the Modeling App does. That one is
chap-core's, not the test's, so it is left alone.

### What each level needs

| Level | Requirement |
| --- | --- |
| model | The model's container running, and an image with the `chapkit` CLI in it. Every chapkit-built image has one; the marketplace images ship chapkit 2.0 or 2.1. chap-core need not be up at all - it is only asked for the model's declared period type, and for which chapkit the service reports when there is none to run. |
| `--backtest` | chap-core up, the model registered with it, and chapkit **1.1.0 or newer** in the image, which is where `$generate-sample-data` arrived. An older one is skipped with the version it reports. |

### Flags

| Flag | What it does |
| --- | --- |
| `--all` | Test every model in `.chaps/models.yaml`. Cannot be combined with an id. |
| `--backtest` | Run the backtest level instead of the model level. |
| `--seed N` | Seed the generated data, so two runs compare. Without it every run is fresh data. |
| `--timeout SECONDS` | How long one model gets: 300 at the model level, 900 with `--backtest`. At the model level the same number is chapkit's per-job deadline. |
| `--keep` | Do not delete what the run created. |
| `-v` | Stream `chapkit test`'s whole output as it runs, and narrate every request. This is what to add to a failure. |
| `--json` | One object per model: `id`, `service_id`, `level`, `result`, `seconds`, `summary`, `detail`, and - for a backtest - `job_id`, `backtest_id` and the whole `metrics` object. |

## Models outside the marketplace

A model the catalogue does not list - a new one, a private one, a fork of your
own - is added to one deployment with `chaps models add`:

```sh
chaps models add https://github.com/chap-models/chapkit_ghr_model
chaps models add ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1
chaps models add ghcr.io/chap-models/chapkit_ghr_model@sha256:31163f6a...
```

The two forms differ in one thing, and it is the important one:

- A **repository URL** follows the default branch. `chaps models add` asks
  GitHub for that branch and its newest commits, and ghcr for the newest of
  those commits that has a published `sha-<short commit>` build - which is the
  tag the model repositories' publish workflow writes. `chaps update` moves the
  pin to whatever the newest published build is then.
- An **image reference** pins exactly what it names, tag or `@sha256:` digest,
  and `chaps update` reports it as pinned and leaves it alone.

Either way the entry is a full definition, so everything else treats it like a
marketplace model: it is enabled by the same write path, rendered into the same
kind of overlay, listed by `models list`, described by `models info`, moved (or
not) by `chaps update`, seen by `chaps doctor` and shown in the browser.

```text
added chapkit_ghr_model (chapkit-ghr-model)
  source    https://github.com/chap-models/chapkit_ghr_model
  image     ghcr.io/chap-models/chapkit_ghr_model:sha-b1d6c31
  pin       sha-b1d6c31  (commit b1d6c31)
  follows   main  (`chaps update` moves the pin)
  data dir  /work/data  (from the image config)
  user      10001:10001  (from a docker probe)
enabled chapkit_ghr_model sha-b1d6c31 on http://localhost:5001 (compose.chapkit-ghr-model.yml)
note: the service must register with chap-core as `chapkit-ghr-model`; if its own MLServiceInfo.id differs, `chaps status` shows it as unmanaged - re-add it with `--service-id <that id>`
run `chaps up` to apply
```

Every line says where its answer came from, because none of them came from a
reviewed catalogue entry:

| Value | Where it comes from |
| --- | --- |
| id | `--id`, else the repository or image name in snake_case. |
| service | `--service-id`, else the id with hyphens. |
| name | `--name`, else the repository or image name. |
| pin | The newest published `sha-` build of the branch, or the tag or digest that was named. |
| data dir | `--data-dir`, else `<WorkingDir>/data` from the image config, else `/work/data`. |
| user | `--user`, else the image's own `User` when it is numeric or an account `chaps` knows, else the numbers a `docker run ... id -u` probe reads out of the image, else the name with a warning. |
| amd64 | Whether ghcr publishes the image for amd64 alone; `--runtime-amd64` records it either way. |

The data directory and the user are resolved the same way for a marketplace
model, and from the same two fields of the same image config; see
[Data directories and users](#data-directories-and-users). The difference is
only where the answer is recorded: `.chaps/models-manual.yaml` holds a manual
model's definition, `.chaps/models.yaml` holds every enabled model's.

`--port N|auto` publishes a host port, exactly as on `models enable`, and is
the one flag that is about the deployment rather than about the model.

The **service id has to match the id the service registers with chap-core**
(chapkit's `MLServiceInfo.id`). It is the Compose service name, the DNS name
and the name the registration resolves to, all at once. Where a model's own id
differs from its repository name, pass `--service-id`; `chaps status` shows the
mismatch as a registered service nobody manages next to a model that never
arrived.

The uid probe is the one step that needs Docker: an image that runs as an
account name (`USER app`) says nothing about what that name resolves to, and
the overlay's init container chowns the data volume from busybox, which
resolves no names at all. So `chaps models add` pulls the image once and asks
it. Pass `--user <uid>:<gid>` to skip that.

### Where it is recorded

The definition goes in `.chaps/models-manual.yaml`, beside the enabled set:

```yaml
chapkit_ghr_model:
  service_id: chapkit-ghr-model
  display_name: chapkit_ghr_model
  repository: https://github.com/chap-models/chapkit_ghr_model
  image: ghcr.io/chap-models/chapkit_ghr_model
  tag: sha-b1d6c31
  commit: b1d6c312a83f07aa1a4e66fce05ae7f4eccb8188
  follow: main
  data_dir: /work/data
  user: 10001:10001
  runtime_amd64: true
  added: 2026-09-24
```

That file is the definition; `.chaps/models.yaml` still says which models are
on. So `chaps models disable chapkit_ghr_model` keeps the definition and
`chaps models enable chapkit_ghr_model` brings the model back at the recorded
tag, data directory and user, without asking the network anything. It is
carried over by `chaps init --force` and included in `chaps backup create`.
A deployment that has added nothing has no such file.

Both listings mark these entries, because a local definition is not a reviewed
catalogue entry:

```text
ID                   SERVICE            NAME               STATUS  STABLE       LATEST       ENABLED  KIND
chapkit_ewars_model  chapkit-ewars-...  CHAP-EWARS         orange  1.0.0        1.0.0        -        model
chapkit_ghr_model    chapkit-ghr-model  chapkit_ghr_model  gray    sha-b1d6c31  sha-b1d6c31  5001     manual
```

`chaps models info chapkit_ghr_model` says the same in its own words - a
`kind` of `manual`, a `source` line naming `chaps models add` and the day it
was added, and a `follows` line naming the branch or reading
`nothing (pinned)` - and leaves out every field a
marketplace entry would have filled in (the horizon, the covariates, the
period types, the assessment) rather than reporting them as zeroes. The
browser marks the row `[manual]`.

### Removing one

```sh
chaps models remove ID [--purge]
```

`remove` is the way back: it disables the model if it is enabled - stopping its
container and removing its overlay, exactly as `models disable` does - and then
drops the definition. `--purge` takes the data volume as well, and means the
same thing here as it does there. A marketplace model has no local definition
to remove, and says so:

```text
error: chapkit_ewars_model is a marketplace model, so there is no local definition to remove; run `chaps models disable chapkit_ewars_model`
```

### What it needs, and what it refuses

`chaps models add` reads GitHub's REST API and ghcr anonymously; both are
public for the CHAP model repositories, and neither needs a token. The
repository form cannot work under `--offline` and says so, naming the image
reference to pass instead. The image form works offline as long as the image's
`linux/amd64` variant is in the local image store, because then `docker image
inspect` can answer what the registry would have; the error names the `docker
pull --platform linux/amd64` that puts it there.

Refused rather than guessed at: a bare image name (`chapkit_ghr_model`), an
image on a registry other than ghcr, an image reference with no tag or digest,
an id or service name the marketplace already uses, and an id this deployment
has already added. The marketplace always wins a collision, so an id it
publishes later shadows nothing: the local entry is reported and ignored until
it is removed or renamed.

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
overlay `chaps` writes for the default model. The file carries two header lines
and nothing else by way of explanation: what it does is below.

```yaml
# Generated by chaps from .chaps/; edit there and run `chaps sync`.
# chapkit_ewars_model 1.0.0 (https://github.com/chap-models/chapkit_ewars_model)
services:
  chapkit-ewars-model:
    restart: unless-stopped
    image: ghcr.io/chap-models/chapkit_ewars_model:${CHAPKIT_EWARS_MODEL_IMAGE_TAG:-sha-fa880a1}
    # amd64-only image; the pin makes an arm64 host pull that variant.
    platform: linux/amd64
    init: true
    expose:
      - "8000"
    environment:
      # $$ is a literal $ for compose.
      SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register
      # Uncomment if chap has SERVICEKIT_REGISTRATION_KEY set:
      # SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    user: 1000:1000
    volumes:
      - type: tmpfs
        target: /tmp
        tmpfs:
          size: 2000000000 # 2GB for model workspaces
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    depends_on:
      chapkit-ewars-model-init:
        condition: service_completed_successfully
      chap:
        condition: service_healthy

  chapkit-ewars-model-init:
    # One-shot: chowns the data volume so the model can write to it, then exits.
    image: busybox:1.37
    command: ["sh", "-c", "chown -R 1000:1000 /app/data"]
    user: "0:0"
    volumes:
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    restart: "no"

volumes:
  ck_chapkit_ewars_model_data: {}
```

### What an overlay contains

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
  `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, a 2 GB tmpfs at
  `/tmp`, and a named volume for the model's data directory (the only writable
  path besides `/tmp`).
- **Runs it as the account its image declares.** `user: <uid>:<gid>`, in
  numbers, for an image that drops to an account of its own. An image that
  runs as root gets no `user:` line at all: the line exists to override the
  image, and overriding root with root only risks taking a permission the
  image's own binaries need. See
  [Data directories and users](#data-directories-and-users).
- **Hands the data volume to the model user**, through the init container
  below - again, only where the model is not root.
- **Ordering.** `depends_on`: the init container with
  `condition: service_completed_successfully` (where there is one) and `chap`
  with `condition: service_healthy`, so a model only starts once its volume is
  writable and chap-core can accept its registration.

Three things an overlay deliberately leaves out:

- no `networks:` key, so the service joins the compose default network only -
  which reaches chap-core but not PostgreSQL or Valkey;
- no `healthcheck:`, because chapkit images ship their own;
- no `depends_on` between models, because they are independent of each other.

## The init container, and why

Docker seeds a fresh named volume from whatever the image has at the mount
point, ownership included, so an image that never creates its data directory
yields a root-owned volume the unprivileged model cannot write to, and chapkit
dies on `sqlite3.OperationalError: unable to open database file`.

Compose cannot `chown` a volume, so the overlay of a model that runs as an
unprivileged account ships a one-shot `<service_id>-init` container (busybox,
as root, `restart: "no"`) that chowns the mount point, recursively, to the
model's **numeric** uid:gid before the model starts. Busybox resolves no `chapkit`
account, which is why the numbers matter, and why the `user:` line above
carries the same two numbers rather than the name. It is the one service in a
deployment that is deliberately not `restart: unless-stopped` (the `"no"` is
quoted because bare `no` is YAML's `false`).

**Every** model overlay gets one, root included, where the chown is to `0:0`.
A volume docker has just created is root-owned already, so on a fresh
deployment a root model's init container does nothing - but a volume that
already exists carries whoever owned it last. A deployment whose `chaps` said
`1000:1000` for a model that a newer `chaps` resolves as root has a volume
owned by 1000, and the model cannot write to it: the overlay drops every
capability, and `CAP_DAC_OVERRIDE` is the one that lets root ignore the
permission bits. One busybox one-shot costs a second on `chaps up` and makes
that upgrade heal itself, which is worth more than the line it saves.

The container has a second job: because it mounts the same named volume at the
same path as the model itself, `chaps backup` reads and writes model data
through it, whether the model is running, stopped, or brought down entirely.
See [Backup and restore](./backup.md).

## Data directories and users

Both differ per image, and both are read off the image itself when the model
is enabled - `chaps models enable`, `chaps init --models`, the browser's
toggle. The answers go into `.chaps/models.yaml`, so `chaps sync` renders the
overlay from what is recorded and never asks the network: the same state
renders the same bytes on any machine.

| Value | Where it comes from, in order |
| --- | --- |
| data dir | `--data-dir`, else `<WorkingDir>/data` from the image config, else the built-in table, else `/work/data`. |
| user | `--user`, else `config.User` from the image config on ghcr, else the same field from an image already pulled here (`docker image inspect --platform linux/amd64`), else the built-in table. |

`chaps models info <id>` and the browser's details pane name which of those
answered: `image config`, `docker probe`, `--user` or `table`. A run that could
reach neither ghcr nor a local copy says so, because the table is a snapshot
this binary was built with and an image can change what it runs as.

What the marketplace images declare today:

| Image | Data directory | User |
| --- | --- | --- |
| EWARS | `/app/data` | `chapkit`, rendered as `1000:1000` |
| The simple multistep model | `/app/data` | `chap`, rendered as `1001:1001` |
| Everything else | `/work/data` | `root` |

**An image that runs as root gets no `user:` line**; its init container is
rendered like any other model's and chowns the volume to `0:0`. Four of the six
marketplace images end their Dockerfile on `USER root`, and forcing an
unprivileged uid on one of them takes away a permission its own binaries need:
the Rwanda BYM model's INLA binaries are root-owned and mode 744, so every
prediction fails with `inla.run: Permission denied`. See
[`Permission denied` from a model's own binaries](./troubleshooting.md#permission-denied-from-a-models-own-binaries).

The init container needs an account name as numbers: `chapkit` is uid/gid 1000
and `chap` is 1001. A `--user` that is already numeric is passed through, and a
name `chaps` does not know falls back to `1000:1000` with a warning from
`chaps sync`. `--data-dir` and `--user` override everything above, for an image
whose config says something the deployment has to contradict.

`chaps doctor` re-checks each enabled model against the image on this machine
and warns when the two have drifted apart, which is what a marketplace model
moved to a new tag by hand looks like.

A model added with `chaps models add` is resolved the same way at the moment it
is added, and carries its answers in `.chaps/models-manual.yaml`. See
[Models outside the marketplace](#models-outside-the-marketplace).

## The amd64 pin, and why

Every overlay also pins `platform: linux/amd64`. The marketplace images are
published for amd64, as is chap-core itself, so an arm64 host such as Apple
silicon pulls that variant and runs it under emulation instead of failing with
`no matching manifest for linux/arm64`.
