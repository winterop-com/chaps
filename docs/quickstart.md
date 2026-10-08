# Quickstart

## 0. Check the machine

After installing, run:

```sh
varde doctor
```

```text
ok    docker cli           docker 29.8.2
ok    docker daemon        Docker Engine 29.8.2
ok    docker compose       v5.5.1
warn  os and arch          macos/aarch64; Chap images are linux/amd64 only, so they run under emulation
      nothing to do: Rosetta runs the amd64 images, they are only slower
ok    disk space           583.7 GB free on /srv
ok    leftovers            no volumes left by removed deployments
ok    network ghcr.io      reachable (HTTP 401)
ok    network marketplace  reachable (HTTP 200)
ok    network releases     reachable, newest chap-core is v2.4.0
ok    github api           reachable, 60 of 60 requests left this hour (no token; set GITHUB_TOKEN for 5000)
ok    varde                v0.100.1, aarch64-apple-darwin, release archive

11 checks: 10 ok, 1 warn, 0 fail
```

There is one line per check: Docker, Compose, the CPU architecture, free disk,
and the hosts Chap pulls from. A check that is not `ok` has a second line that
tells you what to do. The run above is on a Mac, so the architecture check is
a `warn`. Inside a deployment directory, `doctor` also checks the deployment.
See [Doctor](./doctor.md).

## 1. Write the deployment directory

```sh
varde init mychap --models default
```

```text
created a deployment in /srv/mychap
components: chap-core
chap-core: v2.4.0, API on http://localhost:8700
enabled chapkit_ewars_model v1.0.4 at http://localhost:8700/v2/services/chapkit-ewars-model/run/
run `cd /srv/mychap && varde up` to start the deployment
```

`init` writes the compose files, `.env` and the state in `.varde/`, and starts
nothing. With `-v`, it also names each file it wrote.

Model services publish no host port. chap-core reaches them over the compose
network, and you reach them through chap-core at the URL on the `enabled`
line. That route accepts only GET (and HEAD) requests, for example
`/api/v1/info`. For anything else, `varde models expose ID` publishes a port of
the model's own (step 5).

`varde init mychap --with ocs` adds Open Climate Service beside chap-core, and
`--with ocs,s3` adds the object store with it. `--with dhis2` adds a demo or
development DHIS2 and a PostgreSQL of its own; its first start takes minutes
rather than seconds, and [DHIS2](./dhis2.md) says why and what else to know
first. See [Components](./components.md).

`--models default` enables `chapkit_ewars_model` on its `stable` channel.
`--models none` writes the base services only, and `--models a,b` takes an
explicit list of marketplace ids. `--interactive` opens the model browser
instead of reading `--models`.

A model the marketplace does not list yet is added from its repository or
image: `varde models add https://github.com/my-org/chapkit_dengue_model` (see
[Models outside the marketplace](./models.md#models-outside-the-marketplace)).

chap-core itself is pinned to the newest release, `v2.4.0` in the run above,
and `init` takes the `compose.ghcr.yml` that release publishes as the base
of the deployment. `--chap-tag` picks another tag; see [Updating](./updating.md).

The deployment above has no authentication: anything that can reach the port can
use the API. `varde init mychap --models default --api-token` generates a token
and protects the API with it. `varde auth enable` does the same to a deployment
that already exists. See [Authentication](./auth.md).

## 2. Start it

```sh
cd mychap
varde up
```

```text
...
 Container mychap-862350-chapkit-ewars-model-1 Started
started chap, chapkit-ewars-model, postgres, redis, worker
```

The first `up` on a machine pulls the images: about 6 GB to download and
about 20 GB on disk. The chap-worker image alone is 3 GB. On a slow line this
can take close to an hour (47 minutes in one test run). Compose shows
the pull progress while it works. A second deployment uses the same images and
starts in seconds.

`up` renders the compose files from `.varde/` first, then checks that every
host port Chap is about to publish is free, then calls
`docker compose up -d`. The last line names the services it started; a
recreated container counts as started. With `-v`, a hint also names the
services it left alone.

`up` returns when compose has started the containers, not when chap-core
answers. `varde up --wait` returns only once chap-core, every model, OCS and
DHIS2 answer. It also gives the models their configured models (step 3).

Every command that operates on a deployment finds its directory the way git
finds `.git`: from the current directory (or `-C DIR`) upwards to the nearest
`.varde/`, so `varde up` works from any subdirectory of the deployment.

## 3. Check it

```sh
varde status
```

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                STATE                       REACH          LAST PING
chapkit-ewars-model  registered, not configured  via chap-core  2s ago

1 model registered
chapkit-ewars-model: chap-core has no configured model for it, so nothing can run it; run `varde models configure`
```

`registered, not configured` is the normal state after a plain `varde up`.
chap-core v2.4.0 makes no configured model when a model registers. A backtest
and the Modeling App need one. Make them:

```sh
varde models configure
```

```text
chapkit_ewars_model: created configured model monthly_climate
chapkit_ewars_model: created configured model monthly_population_only
chapkit_ewars_model: created configured model monthly_region_seasonal
```

`varde up --wait` does the same step for you. See
[Configured models](./models.md#configured-models). Now `varde status` shows
the model as `registered`:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  0s ago

1 model registered
```

With `-v`, `status` also gives the route through chap-core and the next
command to try:

```text
hint: models without a host port are reachable through chap-core at http://localhost:8700/v2/services/<id>/run/
hint: run `varde models test --all` to check it can run
```

A model can also be `running, not registered` or `not running`, and then
`status` names the command that fixes it. A model that started a moment ago
is `running, not registered` until it is ready (step 5 shows this).

On a deployment whose containers do not exist at all, `status` skips the table
and says so:

```text
Chap is not running; start it with `varde up`
```

[Status and output](./status.md) explains every column and every state.

## 4. Add a model

```sh
varde ui
```

The browser shows the catalogue as one table, with a one-line summary of the
selected row under it. `space` enables or disables, `p` publishes a host port,
`v` switches channel, and `s` saves. `Enter` or `i` shows the full entry, and
`q` quits. Nothing is written until you save. See
[The browser](./models.md#the-browser).

The same thing without the browser:

```sh
varde models search multistep
varde models info chapkit_simple_multistep_model
varde models enable chapkit_simple_multistep_model --channel stable
```

`enable` changes only the files:

```text
enabled chapkit_simple_multistep_model v0.1.2 at http://localhost:8700/v2/services/chapkit-simple-multistep-model/run/
run `varde up` to apply
```

The STATUS column of `search` tells you how ready a model is. No model in the
marketplace is `production` yet. This model and CHAP-EWARS are `limited data`:
they show promise, but need careful evaluation. Read `varde models info` before
you enable a model for real work.

Start the model:

```sh
varde up
varde status
```

`up` says `started chapkit-simple-multistep-model`. Right after it, the new
model is not registered yet, and `status` exits with 1:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                           STATE                    REACH          LAST PING
chapkit-ewars-model             registered               via chap-core  9s ago
chapkit-simple-multistep-model  running, not registered  via chap-core  -

1 of 2 models is not registered.
chapkit-simple-multistep-model: started under two minutes ago and registers once it is ready; run `varde status` again in a minute
```

In the test run, the model registered within 30 seconds. Then it is
`registered, not configured`, as in step 3. Give it its configured models:

```sh
varde models configure
```

```text
chapkit_ewars_model: chap-core has a configured model of it already
chapkit_simple_multistep_model: created configured model monthly_climate
chapkit_simple_multistep_model: created configured model monthly_selfhistory
```

`varde status` now shows both models as `registered`:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                           STATE       REACH          LAST PING
chapkit-ewars-model             registered  via chap-core  5s ago
chapkit-simple-multistep-model  registered  via chap-core  8s ago

all 2 models registered
```

If you run `varde models configure` before the model registers, it says so
and tells you to run it again later.

`varde models list` shows the whole catalogue and what this deployment
enabled:

```text
ID                                SERVICE                           NAME                STATUS        STABLE  LATEST  PORT
chapkit_ewars_model               chapkit-ewars-model               CHAP-EWARS          limited data  1.0.4   1.0.4   via chap-core
chapkit_simple_multistep_model    chapkit-simple-multistep-model    Simple Multistep    limited data  0.1.2   0.1.2   via chap-core
auto_arima_chapkit                auto-arima-chapkit                Auto-ARIMA          experimental  1.0.2   1.0.2   -
chapkit_ghr_model                 chapkit-ghr-model                 GHRmodel            experimental  0.1.3   0.1.3   -
chapkit_rwanda_malaria_bym_model  chapkit-rwanda-malaria-bym-model  Rwanda Malaria BYM  not for use   0.1.3   0.1.3   -

5 listed, 2 enabled in this deployment
```

## 5. Reach a model from your own machine

Model services publish no host port of their own. Either go through chap-core,
which accepts only GET (and HEAD) requests:

```sh
curl http://localhost:8700/v2/services/chapkit-ewars-model/run/api/v1/info
```

or give the model a port:

```sh
varde models expose chapkit-ewars-model
```

```text
exposed chapkit-ewars-model on http://localhost:5001
run `varde up` to apply
```

```sh
varde up
```

`up` recreates the model container to publish the port. The model needs up to
a minute before it answers on `http://localhost:5001` and registers again.
Until then, `varde status` shows it as `running, not registered`:

```text
1 of 2 models is not registered.
chapkit-ewars-model: started under two minutes ago and registers once it is ready; run `varde status` again in a minute
```

See [Ports](./ports.md).

## 6. Back it up

```sh
varde backup create
```

```text
wrote /srv/mychap/varde-backup-mychap-20261008-024106.tar.gz (19.2 KB gzipped, 195.7 KB of data)
```

One `tar.gz` in the deployment directory holds the deployment files, a `pg_dump`
of the chap-core database and one tar per model data volume. The time in the
file name is UTC. The archive holds `.env` with the database password, so keep
it private. `varde backup restore ARCHIVE` puts it back. See
[Backup and restore](./backup.md).

## 7. Move the pins forward

```sh
varde update --dry-run    # the plan, writing nothing
varde update              # move the pins, pull the images
varde restart             # apply them to the services that are running
```

When nothing is newer, `update` gives one row per pin and one closing line:

```text
  chapkit_ewars_model             v1.0.4 (sha-964eea8)  unchanged
  chapkit_simple_multistep_model  v0.1.2 (sha-10b2bc7)  unchanged
  chap-core                       v2.4.0  unchanged, the newest release
already up to date
```

`restart` then shows the compose progress, and its last line is
`nothing needed a restart: every container matches its files`.

`varde update` never touches a container. When a pin moved, its closing line
says what moved and which running services are now behind it, and
`varde restart` recreates exactly those. On a deployment that is not running
there is nothing to restart; with `-v`, a hint says that `varde up` starts it
with the new versions. See [Updating](./updating.md).
