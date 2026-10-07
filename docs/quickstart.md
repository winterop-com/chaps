# Quickstart

## 0. Check the machine

After installing, run:

```sh
varde doctor
```

One line per check: Docker, Compose, the CPU architecture, free disk, and
whether this machine can reach the hosts Chap pulls from. Anything that is not
`ok` comes with the line that fixes it. It works anywhere, and inside a
deployment directory it checks the deployment too. See [Doctor](./doctor.md).

## 1. Write the deployment directory

```sh
varde init mychap --models default
```

```text
Initialized a varde project in /srv/mychap

Wrote:
  .varde/compose.chap-core.v2.3.1.yml
  .env
  compose.yml
  compose.varde.yml
  compose.chapkit-ewars-model.yml
  compose.marketplace.yml
  .varde/project.yaml
  .varde/models.yaml
  .varde/components.yaml

components: chap-core

chap-core: v2.3.1 (chap-core compose.ghcr.yml at v2.3.1)
API:       http://localhost:8700

Enabled:
  chapkit_ewars_model  CHAP-EWARS v1.0.3  internal

Model services publish no host port: chap-core reaches them over the
compose network, and you reach them through it at
http://localhost:8700/v2/services/<service_id>/run/. `varde models expose ID` publishes one.

Next:
  cd /srv/mychap && varde up
  varde status
```

The route through chap-core accepts only GET (and HEAD) requests, for example
`/api/v1/info`. For anything else, `varde models expose ID` publishes a port of
the model's own.

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

chap-core itself is pinned to the newest release, `v2.3.1` in the run above,
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

`up` renders the compose files from `.varde/` first, then checks that every
host port Chap is about to publish is free, then calls
`docker compose up -d` and reports what started or was recreated and what it
left alone.

Every command that operates on a deployment finds its directory the way git
finds `.git`: from the current directory (or `-C DIR`) upwards to the nearest
`.varde/`, so `varde up` works from any subdirectory of the deployment.

## 3. Check it

```sh
varde status
```

```text
chap-core   up   http://localhost:8700   2.3.1   auth: off

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  4s ago

models without a host port are reachable through chap-core at http://localhost:8700/v2/services/<id>/run/

1 model registered
  run `varde models test --all` to check it can run
```

A model can also be `running, not registered` or `not running`, and then
`status` names the command that fixes it. [Status and output](./status.md)
shows every state.

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

The browser lists the catalogue on the left and the selected entry on the right.
`space` enables or disables, `p` publishes a host port, `v` switches channel,
and `s` saves. `Enter` or `i` shows the full entry. Nothing is written until
you save.

The same thing without the browser:

```sh
varde models search malaria
varde models info chapkit_rwanda_malaria_bym_model
varde models enable chapkit_rwanda_malaria_bym_model --channel stable
varde up                                     # apply it
```

`varde models list` shows the whole catalogue and what this project enabled:

```text
ID                                SERVICE                           NAME                STATUS        STABLE  LATEST  PORT
chapkit_ewars_model               chapkit-ewars-model               CHAP-EWARS          limited data  1.0.3   1.0.3   via chap-core
chapkit_simple_multistep_model    chapkit-simple-multistep-model    Simple Multistep    limited data  0.1.1   0.1.1   -
auto_arima_chapkit                auto-arima-chapkit                Auto-ARIMA          experimental  1.0.1   1.0.1   -
chapkit_ghr_model                 chapkit-ghr-model                 GHRmodel            experimental  0.1.2   0.1.2   -
chapkit_rwanda_malaria_bym_model  chapkit-rwanda-malaria-bym-model  Rwanda Malaria BYM  not for use   0.1.2   0.1.2   -

5 listed, 1 enabled in this project
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

See [Ports](./ports.md).

## 6. Back it up

```sh
varde backup create
```

One `tar.gz` holds the project files, a `pg_dump` of the chap-core database
and one tar per model data volume. `varde backup restore ARCHIVE` puts it back.
See [Backup and restore](./backup.md).

## 7. Move the pins forward

```sh
varde update --dry-run    # the plan, writing nothing
varde update              # move the pins, pull the images
varde restart             # apply them to the services that are running
```

`varde update` never touches a container: it ends with one line saying what
moved and which running services are now behind it, and `varde restart`
recreates exactly those. On a deployment that is not running there is nothing
to restart, and the same line says so: `varde up` starts it with the new
versions. See [Updating](./updating.md).
