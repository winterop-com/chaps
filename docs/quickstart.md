# Quickstart

## 0. Check the machine

After installing, run:

```sh
chaps doctor
```

One line per check: Docker, Compose, the CPU architecture, free disk, and
whether this machine can reach the hosts CHAP pulls from. Anything that is not
`ok` comes with the line that fixes it. It works anywhere, and inside a
deployment directory it checks the deployment too. See [Doctor](./doctor.md).

## 1. Write the deployment directory

```sh
chaps init mychap --models default
```

```text
Initialized a chaps project in /srv/mychap

Wrote:
  .chaps/compose.chap-core.v2.3.1.yml
  .env
  compose.yml
  compose.chaps.yml
  compose.chapkit-ewars-model.yml
  compose.marketplace.yml
  .chaps/project.yaml
  .chaps/models.yaml

chap-core: v2.3.1 (chap-core compose.ghcr.yml at v2.3.1)
API:       http://localhost:8000

Enabled:
  chapkit_ewars_model  CHAP-EWARS v1.0.2  internal

Model services publish no host port: chap-core reaches them over the
compose network, and you reach them through it at
http://localhost:8000/v2/services/<service_id>/run/. `chaps models expose ID` publishes one.

Next:
  cd /srv/mychap && chaps up
  chaps status
```

`chaps init mychap --with ocs` adds Open Climate Service beside chap-core, and
`--with ocs,s3` adds the object store with it. See [Components](./components.md).

`--models default` enables `chapkit_ewars_model` on its `stable` channel.
`--models none` writes the base services only, and `--models a,b` takes an
explicit list of marketplace ids. `--interactive` opens the model browser
instead of reading `--models`.

A model the marketplace does not list yet is added from its repository or
image: `chaps models add https://github.com/my-org/chapkit_dengue_model` (see
[Models outside the marketplace](./models.md#models-outside-the-marketplace)).

chap-core itself is pinned to the newest release, `v2.3.1` in the run above,
and `init` takes the `compose.ghcr.yml` that release publishes as the base
of the deployment. `--chap-tag` picks another tag; see [Updating](./updating.md).

The deployment above has no authentication: anything that can reach the port can
use the API. `chaps init mychap --models default --api-token` generates a token
instead and protects it, and `chaps auth enable` does the same to a project that
already exists. See [Authentication](./auth.md).

## 2. Start it

```sh
cd mychap
chaps up
```

`up` renders the compose files from `.chaps/` first, then checks that every
host port CHAP is about to publish is free, then calls
`docker compose up -d` and reports what started or was recreated and what it
left alone.

Every command that operates on a project finds it the way git finds `.git`:
from the current directory (or `-C DIR`) upwards to the nearest `.chaps/`, so
`chaps up` works from any subdirectory of the deployment.

## 3. Check it

```sh
chaps status
```

```text
chap-core   up   http://localhost:8000   v2.3.1   auth: off

MODEL                             STATE                    REACH               LAST PING
chapkit-ewars-model               registered               port 5001           12s ago
chapkit-rwanda-malaria-bym-model  running, not registered  via chap-core       -
auto-arima-chapkit                not running              via chap-core       -
some-other-service                unmanaged                http://c0ffee:8000  3s ago

models without a host port are reachable through chap-core at http://localhost:8000/v2/services/<id>/run/

2 of 3 models are not registered.
  chapkit-rwanda-malaria-bym-model: restart it with `chaps restart --all chapkit-rwanda-malaria-bym-model`
  auto-arima-chapkit: start CHAP with `chaps up`, then `chaps logs auto-arima-chapkit`
```

On a deployment whose containers do not exist at all, `status` skips the table
and says so:

```text
CHAP is not running; start it with `chaps up`
```

[Status and output](./status.md) explains every column and every state.

## 4. Add a model

```sh
chaps ui
```

The browser lists the catalogue on the left and the selected entry on the right.
`space` enables or disables, `p` publishes a host port, `v` switches channel,
`Enter` saves. Nothing is written until you save.

The same thing without the browser:

```sh
chaps models search malaria
chaps models info chapkit_rwanda_malaria_bym_model
chaps models enable chapkit_rwanda_malaria_bym_model --channel stable
chaps up                                     # apply it
```

```text
ID                                SERVICE                           NAME                STATUS        STABLE  LATEST  PORT
chapkit_ewars_model               chapkit-ewars-model               CHAP-EWARS          limited data  1.0.2   1.0.2   via chap-core
chapkit_rwanda_malaria_bym_model  chapkit-rwanda-malaria-bym-model  Rwanda Malaria BYM  not for use   0.1.1   0.1.1   -
chapkit_simple_multistep_model    chapkit-simple-multistep-model    Simple Multistep    limited data  0.1.1   0.1.1   -
auto_arima_chapkit                auto-arima-chapkit                Auto-ARIMA          experimental  1.0.1   1.0.1   -
chapkit_ghr_model                 chapkit-ghr-model                 GHRmodel            experimental  0.1.1   0.1.1   -

5 listed, 1 enabled in this project
```

## 5. Reach a model from your own machine

Model services publish no host port of their own. Either go through chap-core:

```sh
curl http://localhost:8000/v2/services/chapkit-ewars-model/run/api/v1/info
```

or give the model a port:

```sh
chaps models expose chapkit-ewars-model
```

```text
exposed chapkit-ewars-model on http://localhost:5001
written  compose.chapkit-ewars-model.yml
run `chaps up` to apply
```

```sh
chaps up
```

See [Ports](./ports.md).

## 6. Back it up

```sh
chaps backup create
```

One `tar.gz` holds the project files, a `pg_dump` of the chap-core database
and one tar per model data volume. `chaps backup restore ARCHIVE` puts it back.
See [Backup and restore](./backup.md).

## 7. Move the pins forward

```sh
chaps update --dry-run    # the plan, writing nothing
chaps update              # move the pins, pull the images
chaps restart             # apply them to the services that are running
```

`chaps update` never touches a container: it ends with one line saying what
moved and which running services are now behind it, and `chaps restart`
recreates exactly those. On a deployment that is not running there is nothing
to restart, and the same line says so: `chaps up` starts it with the new
versions. See [Updating](./updating.md).
