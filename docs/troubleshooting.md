# Troubleshooting

Start with `chaps doctor`. It runs the whole checklist in one pass - Docker,
Compose, the architecture, free disk, the hosts CHAP pulls from, and, inside a
deployment, the files, the ports, the pins, the images and whether CHAP is up -
and prints what to do about every line that is not `ok`. Most of the sections
below are one of its lines with the reasoning spelled out. See
[Doctor](./doctor.md).

When a command does something you did not expect, run it again with `-v`. It
prints every external command, every HTTP request with its status and timing,
which catalogue it loaded and which files `sync` compared, all on stderr and
all out of the way of `--json`. `-d` adds the response bodies and the resolved
project paths.

## A host port is already in use

`chaps up` refuses before it calls Docker, with one line per conflict:

```text
2 host ports this deployment needs are already in use; nothing was started
  port 8700 is already in use on this machine (needed by chap); free it, or set
  CHAP_API_PORT=8701 in `.env`
  port 5001 is already in use on this machine (needed by chapkit-ewars-model);
  free it, or run `chaps models unexpose chapkit-ewars-model` (the model stays
  reachable through chap-core) / `chaps models expose chapkit-ewars-model --port auto`
```

Pick one of the three: free the port, move the API with `CHAP_API_PORT` in
`.env`, or move (or drop) the model's port. Ports held by this project's own
running containers are not conflicts, so `chaps up` on a running deployment is still
a no-op. `chaps up --no-preflight` hands the question back to Docker.

When the port belongs to another chaps deployment, that one is named, with the
command that stops it:

```text
1 host port this deployment needs is already in use; nothing was started
  port 8790 (needed by ocs) is in use, and climate (/srv/climate) publishes it too; if that is what is up, stop it with `chaps -C /srv/climate down`, or move this one: run `chaps components enable ocs --port 8791`
  or run `chaps up --no-preflight` to hand the conflict to Docker
  or run `chaps up --replace` to stop climate first
```

At a terminal it asks instead of stopping there:

```text
climate is using these ports. Stop it and start this deployment instead? Its data is kept. [y/N]
```

`y` runs `chaps down` on the other deployment (its volumes and data stay; its
own `chaps up` brings it back) and then starts this one. `chaps up --replace`
is the same answer given in advance, for a script. The offer is only made
when every taken port belongs to another chaps deployment: a port some other
program holds is never chaps' to take away.

## The port answers, but it is not chap-core

```text
chap-core   down   http://localhost:8700   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8700 is not responding: port 8700 answers
but it is not chap-core (got text/html)
```

Something else holds the port: a dev server, a proxy, an older deployment.
`chaps status` reports it as down on purpose, because `up` has to mean that the
deployment works and not that the port is taken. Find the listener, or point
the deployment somewhere else with `CHAP_API_PORT` in `.env`.

## A model is running but not registered

```text
chapkit-rwanda-malaria-bym-model  running, not registered  internal  -
```

chapkit tries to register with chap-core five times during its startup and then
stops. A model that came up before chap-core was healthy therefore stays
invisible until it is restarted:

```sh
chaps restart --all chapkit-rwanda-malaria-bym-model
chaps status
```

`--all` because nothing about the service has changed: it is running the image
and the configuration it should be, and the restart is only there to make it
introduce itself again. A plain `chaps restart` recreates what moved, which
here is nothing.

The overlay's `depends_on: chap: {condition: service_healthy}` is there to stop
this happening in the first place, so a model in this state usually means
chap-core became unhealthy and came back after the model had given up.

If registration is failing rather than racing, and the deployment has
authentication on, see
[the models stopped registering](#the-models-stopped-registering-after-i-turned-authentication-on)
below.

## `chaps models test` says a model failed

```text
chapkit-rwanda-malaria-bym-model    FAIL   14s   predict: Error in file(file, "rb") : cannot open file 'model.rds': Permission denied
```

The row is one clause out of the model's own error stream, which is as much as
a row has room for. Three places have more, in the order worth reading them:

```sh
chaps models test chapkit_rwanda_malaria_bym_model -v   # the whole run, as it happens
chaps logs chapkit-rwanda-malaria-bym-model             # what the service itself said
```

`-v` streams everything `chapkit test` printed, which includes the phase that
failed, the diagnostic artifact it stored and the last lines of the model's
stderr. `chaps logs <service>` is the other half: a model that cannot open a
file usually said so on startup too.

With `--backtest` the work happened inside chap-core, so the log is a job's:

```sh
chaps jobs                          # the backtest that failed, newest first
chaps jobs logs <id>                # the traceback and the model's own output
```

The failure row already names that command with the id filled in. See
[Jobs and the API](./jobs.md).

The two levels fail for different reasons, which is what makes running both
worth it:

- Model level fails, `--backtest` not tried: the model itself is broken.
  Permission denied on its own files is the common one - see
  [`Permission denied` from a model's own binaries](#permission-denied-from-a-models-own-binaries)
  - followed by a missing runtime library, which is a fault in the image and
  needs a newer pin (`chaps update`).
- Model level passes, `--backtest` fails: the model works and the round trip
  does not. The usual cause is a covariate the model reads and does not
  declare, which shows up as a `KeyError` from chap-core rather than as
  anything the model said:

  ```text
  chapkit-simple-multistep-model    FAIL    8s   KeyError: "['mean_relative_humidity'] not in index"
  ```

  The sample data is generated from the covariates the service declares
  (`chaps models info <id>`, or
  `chaps api GET /v2/services/<service_id>`), so a model whose configured model
  in chap-core asks for more than that gets a frame without it. That is a fault
  in the model's own declaration, not in the deployment.
- Both fail the same way: the model, again. Fix it at the model level, where
  the loop is seconds rather than minutes.

A `--seed N` makes the generated data the same on every run, which is what to
add when a failure only happens sometimes.

## 401 from the Modeling App

```text
{"detail": "Missing or invalid API token"}
```

The token DHIS2's `chap` route is sending is not the one chap-core is
enforcing, or the route sends none. `chaps dhis2 connect` finds out by asking
through the route, and writes the token this deployment holds into it:

```sh
chaps dhis2 connect
```

`chaps dhis2 show` reports the same thing without changing anything:
``the `chap` route does not carry chap-core's API token``.

Two ways to get here. Either `chaps auth rotate` was run and the clients were
never updated, which is the second half of rotating; or `chaps auth enable` was
run and `chaps up` was not, so chap-core is still running without the token
while `.env` already has one. `chaps status` tells the two apart:

```text
chap-core   up, token rejected   http://localhost:8700   v2.3.1 (pinned)   auth: on
error: chap-core at http://localhost:8700 is up and did not accept the API token:
port 8700 answers /v2/services with HTTP 401: the API token in .env is not accepted
```

means the running chap-core has a different token - it is up, which is why no
container log is printed under it and no model table: the registry is behind
the same token, so whether a model registered cannot be read - and

```text
chap-core   up   http://localhost:8700   v2.3.1   auth: on
```

means `.env` and the running container agree, so the mismatch is in the client.
Either way `chaps up` is what hands a changed `.env` to the containers. See
[Authentication](./auth.md).

## The models stopped registering after I turned authentication on

```text
chapkit-ewars-model  running, not registered  internal  -
```

A protected chap-core rejects an unauthenticated registration like any other
request, so a model that came up without the shared secret never appears in
`GET /v2/services`. `chaps auth enable` writes `SERVICEKIT_REGISTRATION_KEY`
alongside the API token for exactly this reason, and re-renders every overlay
to pass it on, but the containers only read `.env` when Compose creates them:

```sh
chaps up
chaps status
```

If it persists, check that both ends carry the line:

```sh
grep SERVICEKIT_REGISTRATION_KEY compose.chapkit-ewars-model.yml compose.chaps.yml
```

An active `SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}` in
the overlay is what the model sends; a commented one means
`.chaps/project.yaml` does not know the project has a key, which
`chaps auth show` will report as a mismatch and `chaps auth enable` puts right.

The same line in `compose.chaps.yml` is what chap-core checks it against.
Upstream's `compose.ghcr.yml` passes only `CHAP_API_TOKEN` into the `chap`
service, so a `compose.chaps.yml` rendered by an older `chaps` leaves the
container with no key and the model's `X-Service-Key` is rejected as an invalid
API token - a 401 in `chaps logs chapkit-ewars-model`. `chaps doctor` reports
it on the `.env` line; `chaps sync` then `chaps restart` fixes it.

## `unable to open database file`

```text
sqlite3.OperationalError: unable to open database file
```

Docker seeds a fresh named volume from whatever the image has at the mount
point, ownership included, so an image that never creates its data directory
yields a root-owned volume the unprivileged model cannot write to.

Every model overlay ships a one-shot `<service_id>-init` container that chowns
the volume to the model's numeric uid:gid before the model starts (`0:0` for a
root image), so this is fixed by construction. If you see it anyway:

- the overlay was hand-edited, or the init container was removed. Run
  `chaps sync` to render it again.
- `--user` names an account `chaps` does not know, so it fell back to
  `1000:1000`. `chaps sync` warns when it does. Pass the numbers instead:
  `chaps models enable ID --user 1001:1001`.
- the model writes somewhere other than its data directory, on the read-only
  root filesystem. `--data-dir` points the volume at the right path; see
  [Data directories and users](./models.md#data-directories-and-users).

A model that crash-loops right after starting is almost always one of the last
two.

## `Permission denied` from a model's own binaries

```text
sh: /usr/local/lib/R/site-library/INLA/bin/linux/64bit/inla.run: Permission denied
```

The model started, registered, and then could not execute a file it ships
itself. The overlay is running it as an account the image does not use.

Some model images end their Dockerfile on `USER root` and keep their binaries
root-owned and not world-executable - the Rwanda BYM model's INLA binaries were
mode 744 up to 0.1.1 - so an overlay that hardens such an image down to
`user: 1000:1000` takes away the one permission it needed. The container comes
up either way, which is why this surfaces as a failed prediction rather than a
failed start.

`chaps models enable <id>` reads the account off the image again and rewrites
the overlay; `chaps up` then restarts the service with it:

```console
$ chaps models enable chapkit_rwanda_malaria_bym_model
$ chaps up
```

The rendered overlay should then carry no `user:` line for a root image, and
its `<service_id>-init` container should chown the volume to `0:0`. For an
image that does drop to an account of its own - which Rwanda BYM 0.1.2 now
does, as `chapkit` - it carries that account's `uid:gid` instead, and the init
container chowns to the same two numbers.
`chaps models info <id>` shows what was recorded and where it came from, and
`chaps doctor` has one `user <service>` line per enabled model that compares
the two whenever the image's amd64 variant is pulled here.

`--user <uid>:<gid>` overrides the image, for the rare case where the image is
wrong about itself. See
[Data directories and users](./models.md#data-directories-and-users).

### `attempt to write a readonly database` after upgrading chaps

```text
sqlalchemy.exc.OperationalError: (sqlite3.OperationalError) attempt to write a readonly database
```

An older `chaps` ran some models as `1000:1000` that this one resolves as
root, so their `ck_<id>_data` volumes - and the `chapkit.db` in them - are
owned by uid 1000 while the model now runs as root. Root would normally ignore
the permission bits, but the overlay drops every capability (`cap_drop: ALL`),
and `CAP_DAC_OVERRIDE` is the one that lets root do that.

The overlay's own init container is what fixes it, so upgrading is two
commands:

```sh
chaps sync     # re-render the overlays
chaps up       # the init container chowns the volume, then the model starts
```

`chaps sync` reports the overlays it rewrote, and `chaps up` recreates those
models; the one-shot chown runs before each one and hands the volume over.
`chaps status` should then show the model registered.

Only a deployment brought up by hand - `docker compose up` on the rendered
files, or `chaps up --no-preflight` on files that were never re-rendered -
needs the chown done by hand:

```sh
docker run --rm -v <project>_ck_<id>_data:/v busybox:1.37 chown -R 0:0 /v
chaps restart --all <service_id>
```

## `dependency failed to start: container ... is unhealthy`

`chaps up` reads that line and then says why, out of the failing container's
own log:

```text
dependency failed to start: container demo-1ab2c3-chap-1 is unhealthy

why chap is unhealthy:
  sqlalchemy.exc.OperationalError: (psycopg2.OperationalError) connection to server at
  "postgres" (192.168.32.2), port 5432 failed: FATAL:  password authentication failed for
  user "chap"
  ERROR:    Application startup failed. Exiting.
  the database volume holds a different password than .env (a previous deployment with the
  same name, or --fresh-env); run `chaps doctor`, or remove the volume with
  `chaps down --volumes` if this deployment's data can go
```

The same lines close `chaps status` and `chaps doctor`'s `health` check, and
the chap-core line reads `down (container unhealthy)` rather than plain `down`:
the container is there, and it is the container that is wrong.

The two causes worth knowing by name are below. `chaps logs chap` has the rest.

## `password authentication failed for user "chap"`

The PostgreSQL volume holds a role password that is not the one in `.env`.
There are two ways to get there.

**A volume from another deployment of the same name.** Before `chaps` recorded
a compose project name, Compose derived one from the directory, so two
deployments in directories both called `demo` shared `demo_chap-db` - on
different paths, and even when the first one had been deleted long ago. The
new deployment's `.env` has a freshly generated password; the inherited volume
still has the old role. `chaps doctor` says so:

```text
warn  project   compose project name is the directory name; volumes can collide with other
                deployments named demo
      run `chaps sync` to record it as `demo` in .chaps/project.yaml; it is the name compose
      already uses, so nothing is renamed
warn  volumes   the database volume demo_chap-db predates this deployment; if chap-core cannot
                log in, it belongs to an earlier deployment with the same name
      remove it with `chaps down --volumes` if this deployment's data can go, or keep both
      by giving one of them a name of its own
```

A deployment created by this version of `chaps` has a name of its own
(`demo-1ab2c3`) and cannot collide; see
[the compose project name](./concepts.md#the-compose-project-name). An older
one gets that name written down, unchanged, by the next `chaps sync`.

**`init --fresh-env`.** It rotates the PostgreSQL password on purpose, and the
existing volume was created with the old one.


Either way: drop the volume and start over,

```sh
chaps down --volumes
chaps up
```

or change the role to match the new password with `ALTER USER` inside the
running postgres container:

```sh
chaps docker exec postgres psql -U chap -d chap_core
```

This is why `init` never rewrites a `.env` it finds, `--force` included. See
the [`.env` contract](./concepts.md#the-env-contract).

## `Control server error: [Errno 30] Read-only file system: '/home/chap'`

```text
[ERROR] [gunicorn.error] Control server error: [Errno 30] Read-only file system: '/home/chap'
```

A log line and nothing more: the API starts and serves. chap-core's image runs
gunicorn 26, which opens a control socket at `$XDG_RUNTIME_DIR/gunicorn.ctl` and
falls back to `$HOME/.gunicorn/` when that variable is unset. The `chap` service
runs on a read-only root filesystem as a user with no home directory, so the
fallback path cannot be created and gunicorn reports it once per start.

A deployment rendered by this version of `chaps` sets `XDG_RUNTIME_DIR: /tmp` in
the `chap` service's environment in `compose.chaps.yml`, which puts the socket on
the tmpfs the service already mounts, so the line does not appear. An older
deployment has the override without it; re-render and recreate the container:

```sh
chaps sync
chaps restart
```

## `no matching manifest for linux/arm64`

The marketplace images and chap-core are published for amd64 only. Every
overlay `chaps` writes pins `platform: linux/amd64`, so an arm64 host such as
Apple silicon pulls that variant and runs it under emulation instead of
failing.

Seeing this error means the pin is missing: a hand-edited overlay, or a compose
file not written by `chaps`. Run `chaps sync` to render the overlays again, and
check `chaps docker config` for the service that has no `platform`.

## Compose is older than 2.24.4

```text
warning: docker compose 2.18.1 is older than 2.24.4; compose.chaps.yml uses `!override`, which needs 2.24.4 or newer, and compose.marketplace.yml uses `include:`, which needs 2.20.0
```

`compose.chaps.yml` uses `!override`, which arrived in Compose 2.24.4, and
`compose.marketplace.yml` uses `include:`, which arrived in 2.20. `chaps` warns
rather than failing, because the base services still run.

What an older Compose loses, in order: below 2.24.4 the API port override is
merged into chap-core's own `ports:` instead of replacing it, so the API is
published on two host ports; below 2.20 the model overlays do not load at all.
Upgrade Docker Compose.

## `needs chap-core, and this deployment has no chap-core`

```text
error: `chaps jobs` needs chap-core, and this deployment has no chap-core; `chaps components enable chap-core` adds it
```

The deployment was created without chap-core (`--without chap-core`), so there
is no chap-core to ask. `chaps jobs`, `chaps api` (without `--url`),
`chaps models test` and `chaps update --chap-tag` all say this. If you meant to
talk to a chap-core somewhere else, `chaps api --url <base>` sends the request
there. Otherwise add chap-core with `chaps components enable chap-core`, then
`chaps up`. See [Standalone OCS](./components.md#standalone-ocs).

## `registration.missing_orchestrator_url` in a model's log

```text
[error    ] registration.missing_orchestrator_url [servicekit.api.registration] env_var=SERVICEKIT_ORCHESTRATOR_URL
```

Expected in a deployment without chap-core, and harmless. The overlay leaves
`SERVICEKIT_ORCHESTRATOR_URL` out on purpose because there is nothing to
register with. servicekit logs this line at error level, then skips registration
and serves as usual. `chaps status` asks the model's own `/health`, which is the
check that matters there. In a deployment *with* chap-core the line means the
overlay is out of date: run `chaps sync`, then `chaps restart`.

## `running and not answering on /health`

```text
1 of 1 model is running and not answering on /health; run `chaps status` again in a moment, or read `chaps logs SERVICE`
```

A deployment without chap-core asks each model's own `/health` on its host
port. A model image can take a minute to start, so ask again first. If it stays,
`chaps logs <service_id>` shows why the service did not come up. A model enabled
with `--port none` has no host port to ask and is judged by its container alone.

## `App never became ready, skipping registration`

```text
[error    ] registration.aborted [servicekit.api.service_builder] message='App never became ready, skipping registration' port=5001
```

servicekit checks that the app answers at `127.0.0.1:<SERVICEKIT_PORT>` inside
its own container before it registers. With a chap-core elsewhere, chaps sets
`PORT` and `SERVICEKIT_PORT` to the model's host port so the two agree, and the
image has to listen on `PORT`. One that starts on a fixed 8000 (the EWARS image
does) cannot, and never registers with a chap-core elsewhere. Use a model whose
image honours `PORT`, or run chaps' own chap-core
(`chaps components enable chap-core`), where every model listens on 8000.
`chaps models enable` warns about the marketplace images known to do this
(EWARS), and `chaps status` points here when such a model stays
`running, not registered` while the chap-core elsewhere answers.

## A model registers with an external chap-core, and its jobs fail to connect

The model shows as `registered` in `chaps status`, but a job fails with a
connection error to `localhost:<port>` or `host.docker.internal:<port>`. The
registration worked, and chap-core cannot reach the address the model
registered under. That address is `models_host` in `.chaps/components.yaml`
plus the model's host port:

- chap-core runs as a process on this machine: `localhost` is right.
- chap-core runs in a container: run
  `chaps components enable chap-core --url URL --models-host host.docker.internal`,
  then `chaps up`.
- chap-core runs on another machine: use the name or address it reaches this
  machine by, and make sure the model ports (5001 up) are open to it.

The same applies the other way round, to a model you run from its checkout
against a chap-core in chaps: `SERVICEKIT_HOST` must be `host.docker.internal`
and the model must listen on `0.0.0.0`. See
[Your model from its checkout, with CHAP](./use-cases/model-on-host.md).

## `is not in the local image store for linux/amd64`

```text
error: my-model:dev is not in the local image store for linux/amd64; build it with `docker build --platform linux/amd64 -t my-model:dev .` in the model's checkout, then add it again
```

`chaps models add` read `my-model:dev` as a local image, because it names no
registry, and the local store has no amd64 build under that name. Either it was
never built, it was built for another platform (a plain `docker build` on an
Apple Silicon Mac makes an arm64 image), or the tag differs. Run the command in
the message. If you meant a published image, give its full reference, such as
`ghcr.io/my-org/my-model:sha-1eb8cf1`.

## `is not a chap-core checkout`

```text
error: /Users/me/src/chap is not a chap-core checkout: it has no Dockerfile.worker; pass the directory you cloned github.com/dhis2-chap/chap-core into
```

`chaps init --source` builds the chap-core API from `Dockerfile`, the worker
from `Dockerfile.worker`, and renders `compose.yml` from `compose.ghcr.yml`, all
at the top of the checkout. Point it at the directory `git clone` created, not
a subdirectory. A very old chap-core may predate one of the three files; check
out a newer commit.

## A hand edit disappeared

`chaps sync` re-renders the artifacts from `.chaps/`, so an edit to
`compose.yml`, `compose.chaps.yml`, `compose.marketplace.yml` or a
`compose.<service_id>.yml` is drift that the next `sync` (and therefore the next
`chaps up`) undoes.

- To change the base services, edit `.chaps/compose.chap-core.<tag>.yml`. `sync`
  follows it, and says that the recorded checksum no longer matches.
- To change a model, edit `.chaps/models.yaml` and run `chaps sync`.
- To add something of your own, write a `compose.custom.yml`. `sync` only ever
  removes overlays it wrote itself, so yours is left alone; add it to the `-f`
  list or to the umbrella by hand if you want it included.

`chaps sync --check` reports drift without writing, and exits non-zero, which
makes it a usable pre-commit or CI check.

## An OCS dataset plugin does not appear after a restart

You put a plugin in `ocs/plugins/datasets/`, ran `chaps restart` - or
`chaps restart --all ocs` - and OCS still serves the datasets it served before.
Nothing failed: the plugin directory was never mounted.

The mount is rendered from the directory being there, and `chaps up` is the only
wrapper that re-renders the compose files before it calls Docker. On a deployment
where `ocs` was enabled before `ocs/plugins/` existed, `compose.ocs.yml` carries
no plugin mount, so `chaps restart` compares the container against a file it
already matches and does nothing at all.

```sh
chaps up                                          # syncs first, then recreates ocs
chaps docker exec ocs ls /app/plugins/datasets    # the plugin is inside
```

`chaps doctor` finds this on its own, on the `compose files` line, which is
`chaps sync --check`:

```text
warn  compose files   out of date with .chaps/ (2 to write, 5 unchanged, 0 to remove)
      run `chaps sync`, or `chaps up`, which syncs first
```

Two files, because the mount in `compose.ocs.yml` and the `plugins_dir` key in
`ocs/climate-service.yaml` are written by the same sync.

`chaps restart --all ocs` is the right command for a change to
`ocs/climate-service.yaml`, and for the opposite reason: that file is a bind
mount, so its new text is already inside the container and the compose files have
not changed - all that is wrong is that OCS read the old text at startup. A
directory that did not exist when the compose file was rendered is the other
case: the file itself is out of date, and recreating a container cannot mount
what the file does not mention. See
[Adding a plugin to a running deployment](./components.md#adding-a-plugin-to-a-running-deployment).

## A DHIS2 that answers but 404s every API request

The login page loads, `chaps status` may even say `dhis2  up`, and every
`/api/*` request answers **404**. Nothing in the log says "error".

DHIS2 migrates its schema **forward only**, so this is almost always an older
image started against a database a newer one has already migrated. The Spring
context fails to come up, Tomcat keeps running and keeps serving pages, and every
API route is simply not registered. The other two causes have the same shape: a
`dhis.conf` DHIS2 could not read, and a Flyway checksum that does not match the
database.

```sh
chaps logs dhis2 | grep -iE "flyway|migrat|exception|failed"
docker compose config | grep "image: dhis2/core"    # the tag that is running
grep -n "image_tag" .chaps/components.yaml          # the tag that was asked for
grep -n "DHIS2_IMAGE_TAG" .env                      # and any override, which wins
```

Put the tag back where the database is, rather than migrating further:

```sh
chaps backup create                     # before anything, if there is data worth keeping
# set image_tag back in .chaps/components.yaml, or fix DHIS2_IMAGE_TAG in .env
chaps sync
chaps up
```

There is no way back down a migration. A database migrated by 2.42 does not work
under 2.41 again, so the choice is the newer image or a restore. `chaps` warns
before it happens - the note names `chaps backup` first for exactly this reason:

```text
note: the DHIS2 image moves from 2.42 to 2.41 and `dhis2_db` is already there: DHIS2 migrates a schema forward only, so run `chaps backup` first - an older image on a migrated database answers healthy while every API request 404s
```

A deployment with nothing in DHIS2 worth keeping is quicker to start again:
`chaps components disable dhis2 --purge` takes all three volumes, and the next
`chaps up` migrates or restores from scratch. See
[Changing the DHIS2 version](./dhis2.md#changing-the-dhis2-version).

## DHIS2 never becomes healthy

Two quite different things look the same from outside, and the first start is
where both show up.

**It is still migrating.** DHIS2 migrates its whole schema before it serves a
request, and on a seeded deployment it restores a dump first. That is about 40
seconds on a native arm64 host with an empty database and **8 to 15 minutes**
under emulation. The health check allows 240 seconds of `start_period` and 60
retries for that reason. Watch it rather than restarting it:

```sh
chaps logs -f dhis2
```

**It ran out of memory.** DHIS2 wants roughly **4 to 5 GB** for the analytics
populate phase, and below that the JVM is `SIGKILL`ed part-way through. That
failure looks like anything except memory: a container that exits with no Java
exception in the log, an analytics run that never ends, an instance that was
healthy a minute ago and is now gone.

```sh
docker inspect --format '{{.State.OOMKilled}} {{.State.ExitCode}}' \
  "$(chaps docker ps -- -q dhis2)"
docker info --format '{{.MemTotal}}'      # what the daemon has to give
```

`OOMKilled true`, or exit code 137, settles it. Raise the memory the Docker
daemon has - on Docker Desktop that is the VM's limit, in Settings, Resources -
rather than trimming the heap: a smaller `-Xmx` makes the JVM fail inside Java
instead of being killed, which is tidier and no more successful. The heap itself
is `DHIS2_JAVA_TOOL_OPTIONS` in `.env`, commented with the default
`-Xms2g -Xmx4g -XX:+UseG1GC`. See [Memory](./dhis2.md#memory).

## The Modeling App does not see CHAP

Because nothing has connected the two yet, and one command does all of it:

```sh
chaps dhis2 show        # which of the three pieces is missing
chaps dhis2 connect     # the route, the apps, then analytics
```

The app does not reach chap-core directly. It goes through a DHIS2 Route with
`code: "chap"`, and until that row points at this deployment the app redirects to
`/get-started`.

The trap is that it will **look** configured. The climate demo dumps ship a
`chap` route of their own - right code, right authority, not disabled - aimed at
an external CHAP server, so a seeded instance has a route by that name resolving
to somebody else's chap-core. `chaps dhis2 route` repoints it rather than skipping
it, and says where it pointed:

```text
repointed the `chap` route at http://chap:8000/**
  it pointed at http://158.39.75.126/stable/**
  verified chap-core answered through it: healthy
```

`chaps dhis2 show` is the one to run first: it names each piece that is missing
and changes nothing. If the route is right and the app still shows no figures,
analytics has not been generated - `chaps dhis2 analytics`. If there is no CHAP
entry in DHIS2's apps menu at all, the app is not installed -
`chaps dhis2 apps`. See
[Connecting the Modeling App to CHAP](./dhis2.md#connecting-the-modeling-app-to-chap).

Do not read a **silent** `chaps up` or `chaps status` as an answer here. Those
two name `chaps dhis2 connect` only until one has been recorded, and the record
is a note that the command ran rather than a check of the route - see
[`connected_at`, and what it is not](./dhis2.md#connected_at-and-what-it-is-not).
`chaps dhis2 show` is the command that asks DHIS2.

## `chaps has not connected this DHIS2 to CHAP`

`chaps up` and `chaps status` close with this while `.chaps/components.yaml`
records no `chaps dhis2 connect` for this deployment:

```text
chaps has not connected this DHIS2 to CHAP; run `chaps dhis2 connect`
```

Do what it says, once DHIS2 answers. The command is idempotent, so running it on
a deployment that is already connected repoints nothing, reinstalls nothing and
says so - which is also why the line is safe to print without asking DHIS2
anything.

It keeps coming back in three cases, all of them deliberate:

- **the component was disabled and enabled again.** The record is about a DHIS2
  instance, so it is forgotten with the component. A re-enabled `dhis2_db` can be
  restored from the seed dump, and [that dump ships a `chap` route of its
  own](#the-modeling-app-does-not-see-chap) pointing at an external server.
- **`chaps down --volumes` removed `dhis2_db`.** Same hazard, said on the spot:
  the record of `chaps dhis2 connect` went with `dhis2_db`.
- **the last `connect` found something wrong.** A route nothing answered on, or
  an app that would not install, clears the record and the report says
  `cleared` - `chaps dhis2 show` names which of the two it was. A run that could
  not look is not one of these: `chaps dhis2 connect --offline` skips the apps,
  judges nothing and leaves the record exactly as it found it.

Connecting the instance by hand, in DHIS2's own Route administration and App
Management, does not record anything either, and the line then stays. `chaps
dhis2 connect` against that instance is the way to settle it: it will find
nothing to change and record that it ran.

## The route is there but nothing answers through it

`chaps dhis2 route` wrote the row and then could not prove the path:

```text
warning: the `chap` route is in place but nothing answered through it: HTTP 502 Bad Gateway; run `chaps status` to see whether chap-core is up
```

The route is correct, so this is not a route problem. Three things it is, in the
order worth checking:

1. **chap-core is not running.** `chaps status` settles it, `chaps up` fixes it,
   and `chaps dhis2 route` then verifies on the next run.
2. **`route.remote_servers_allowed` does not list the target.** DHIS2 42 and later
   default that setting to `https://*` and refuse an `http://` target. The
   scaffolded `dhis2/dhis.conf` already permits `http://chap:8000`, so this is an
   instance whose file was narrowed or replaced; DHIS2 refuses the write outright
   in that case and the error names the line. After editing it,
   `chaps restart --all dhis2` - a plain `chaps restart` does not apply it, and
   says [nothing needed a restart](#an-edit-to-dhis2dhisconf-changes-nothing-after-a-restart).
3. **Something else answers on that hostname.** A 200 that is not chap-core's
   health document is reported as such rather than as success.

## An edit to `dhis2/dhis.conf` changes nothing after a restart

You corrected a value in `dhis2/dhis.conf` - `route.remote_servers_allowed`,
a `connection.*` line, one of the commented `server.https` proxy lines - ran
`chaps restart dhis2`, and DHIS2 behaves exactly as it did before. The restart
said so at the time:

```text
nothing needed a restart
```

Nothing failed, and the file is not being ignored. `chaps restart` is
`docker compose up -d`, which recreates only what no longer matches the compose
files, and `dhis.conf` is a **bind mount**: the new text is already inside the
container, so there is nothing for compose to compare and no container to
recreate. DHIS2 read the file once, at startup, and is still running on what it
read then.

```sh
chaps restart --all dhis2
```

`--all` with the service named force-recreates that one container and leaves
chap-core and the models alone. DHIS2 migrates before it serves a request again,
so it is not back when compose returns; the next `chaps dhis2` command waits for
it. Then re-run whatever reported the problem:

```sh
chaps dhis2 route
```

The same trap, for the same reason, catches `ocs/climate-service.yaml` - see
[Read-only instances](./components.md#read-only-instances) and
[`dhis2/dhis.conf`](./dhis2.md#applying-an-edit). It is not the same as
[An OCS dataset plugin does not appear after a restart](#an-ocs-dataset-plugin-does-not-appear-after-a-restart),
where the compose file itself is out of date: `--all` cannot fix that one and
`chaps up` can.

## `the App Hub publishes no version of ... that DHIS2 ... can run`

```text
failed Modeling App was not installed: the App Hub publishes no version of Modeling that DHIS2 2.42.6 can run; install it from DHIS2's own App Management page
```

**Upgrade `chaps`.** Up to and including 0.4.0 this sentence was wrong for every
instance there is. The App Hub sends a version bound it has not set as an empty
string rather than as an absent field, and `""` was read as a maximum: every
published version looked capped below the instance, so every one was filtered
out. `chaps` now treats a missing, empty, whitespace or unreadable bound as no
bound, on both sides.

If it appears on a current `chaps`, it is what it says: check the app's own page
on [apps.dhis2.org](https://apps.dhis2.org) for a version that lists this
instance's release under `minDhisVersion`. `chaps dhis2 apps -v` prints the
version it resolved and the id it installed.

`POST /api/appHub/{versionId}` is idempotent, so installing from DHIS2's own App
Management page in the meantime costs nothing: `chaps dhis2 apps` afterwards
reports the app as already installed rather than fighting it.

## `analytics may never have run on this deployment`

```text
analytics  2026-06-16T07:51:00.093 (unconfirmed on a seeded database)
missing: analytics may never have run on this deployment: a seeded database can carry the dump's timestamp, and no run has finished since DHIS2 started; run `chaps dhis2 analytics` to settle it
```

This is not an error, and it is not `chaps` saying the tables are missing. It is
`chaps` declining to say they are there.

`lastAnalyticsTableSuccess` is a row of DHIS2's own settings, so a
[seeded](./dhis2.md#the-seed) deployment restores it with the rest of the dump.
Measured on a 2.42.6 with the Laos climate demo, a freshly seeded instance
reported a last success of `2026-06-16T07:51:00.093` while `analytics_2024` did
not exist at all - the table was absent, not empty, and the Modeling App saw
nothing. The timestamp was a fact about the database the dump was taken from.

```sh
chaps dhis2 analytics
```

settles it, and the row afterwards reads `(a run finished on this deployment)`.
Restarting DHIS2 empties the notifier `chaps` reads that from, so the line can
come back on a deployment where analytics really has been generated; running it
again is idempotent and takes tens of seconds on demo data. See
[What `show` can say about analytics](./dhis2.md#what-show-can-say-about-analytics-and-what-it-cannot).

## The analytics tables are empty but the run reported success

Almost certainly `lastYears`. Measured on the climate demo,
`POST /api/resourceTables/analytics?lastYears=8` reported success in 16.9 seconds
and wrote **zero rows into every analytics table**; the same request without it
wrote 146,129 rows into `analytics_2024` in 15.7 seconds.

`chaps dhis2 analytics` never sends it, so a run made with

```sh
chaps dhis2 analytics
```

populates what the database holds. A reference deployment's own
`trigger-analytics.sh` does send `&lastYears=8`, which is where this comes from.

If the tables are still empty afterwards, the database has no data in the period
the apps are asking about: the [Climate App](./dhis2.md#the-apps-come-from-the-app-hub-server-side)
is what imports climate data into DHIS2, and analytics has to run again after an
import.

## `chaps dhis2` says DHIS2 did not accept the password

```text
error: DHIS2 at http://localhost:8780 did not accept the password for `admin` (the DHIS2 default password); set `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` in `.env`, or export `CHAPS_DHIS2_PASSWORD`
```

`chaps` holds no DHIS2 credentials of its own. On a DHIS2 it deployed it falls
back to `admin` / `district`, which is what a seeded demo dump and a
Flyway-bootstrapped empty database both give. So this is an instance whose
password has been changed, or one restored from a dump of your own. Either
uncomment and edit the lines in `.env`:

```ini
DHIS2_ADMIN_USERNAME=admin
DHIS2_ADMIN_PASSWORD=the-one-that-works
```

or keep it out of the file entirely:

```sh
export CHAPS_DHIS2_PASSWORD=the-one-that-works
```

or use a personal access token instead (`DHIS2_API_TOKEN` in `.env`, or
`CHAPS_DHIS2_TOKEN`). There is no `--password` flag, because a password on a
command line is in the shell history and in `ps`. A **403** rather than a 401
means the opposite problem: the credential is right, but that user is not
allowed to write a route or run analytics, which a DHIS2 superuser is. See
[The credentials](./dhis2.md#the-credentials-and-where-they-come-from).

## `chaps dhis2` says DHIS2 did not accept the API token

```text
error: DHIS2 at https://dhis2.example.org did not accept the API token (API token from `.env`): it is expired, revoked, or not allowed from this address; set a current one as `DHIS2_API_TOKEN` in `.env`, or export `CHAPS_DHIS2_TOKEN`
```

DHIS2 answers 401 for a token that has expired or been revoked, and for one
whose allowed IP addresses or referrers leave out the machine chaps runs on.
Create a new one under **Profile > Personal access tokens** in DHIS2. The 401
also covers a value that was never a token, such as one pasted with the
`ApiToken ` prefix: `DHIS2_API_TOKEN` holds the token alone.

## `chaps has no password for DHIS2 user`

```text
error: chaps has no password for DHIS2 user `alice`: `.env` and `CHAPS_DHIS2_USERNAME` give theirs to other users; export `CHAPS_DHIS2_PASSWORD` for this run
error: chaps has no password for DHIS2 user `ops`: `.env` names the user and sets no `DHIS2_ADMIN_PASSWORD`; set it there, or export `CHAPS_DHIS2_PASSWORD`
```

A password belongs to the user it was set with, and chaps will not send one
user's password in another's name. `--user alice` finds only a password that is
alice's, and the default `district` is `admin`'s alone. For a one-off run as
someone else, export theirs for the command:

```sh
CHAPS_DHIS2_PASSWORD=theirs chaps dhis2 show --user alice
```

## `chaps has no credentials for this DHIS2, and did not deploy it`

```text
error: chaps has no credentials for this DHIS2, and did not deploy it, so there is no default to try; set `DHIS2_API_TOKEN` in `.env`, or export `CHAPS_DHIS2_TOKEN`
```

The DHIS2 was recorded with `chaps dhis2 use`, so `admin` / `district` is not
known to be anybody's password there, and chaps does not try it. Set a personal
access token (or a username and password) as in
[The credentials](./dhis2.md#the-credentials-and-where-they-come-from), then run
`chaps dhis2 use` again to check that DHIS2 accepts it.

## `this deployment already uses the external DHIS2`

```text
error: this deployment already uses the external DHIS2 at https://dhis2.example.org; run `chaps dhis2 use --clear` first to deploy one of its own
```

A deployment has one DHIS2, either the `dhis2` component or an external one, so
`chaps dhis2` never has to guess which. The opposite direction is refused the
same way (`the dhis2 component is on, and it is this deployment's DHIS2`). Clear
the one you no longer want, then add the other. See
[A DHIS2 that runs elsewhere](./dhis2.md#a-dhis2-that-runs-elsewhere).

## `chaps dhis2` waited twenty minutes and gave up

```text
error: DHIS2 at http://localhost:8780 did not answer /api/ping within 20 minutes; `chaps logs dhis2` is where the migration shows, and `--wait SECONDS` waits longer
```

Either DHIS2 is still migrating - under emulation a first start is a quarter of an
hour, and a seeded one is that plus the restore - or it has stopped. `chaps logs
dhis2` tells the two apart, and
[DHIS2 never becomes healthy](#dhis2-never-becomes-healthy) is the rest of the
list. `--wait 3600` waits an hour.

## `chaps update` said a restart is needed

That is the whole design, not a failure. `chaps update` moves the pins and
pulls the images and stops there: it never decides for you when a deployment
goes down. The closing line names the running services that are now behind
what the files say, and

```sh
chaps restart
```

recreates exactly those, leaving the rest running. If the line says CHAP is not
running instead, `chaps up` starts it with the new versions. See
[Updating](./updating.md).

## `chaps update` fails offline

By design. `chaps update` fetches the registry from the network with no cache
and no fallback, `--dry-run` included, because a plan made from a stale
catalogue is not a plan. Every other command falls back to the cache and then
to the snapshot compiled into the binary.

## `GitHub's rate limit is used up`

```text
skip  registry pin chapkit_ewars_model  could not ask https://github.com/chap-models/chapkit_ewars_model
      for its default branch: GitHub's rate limit is used up (unauthenticated, 60 requests an hour
      per address); set GITHUB_TOKEN for 5000, or wait until 15:04
```

GitHub allows 60 REST requests an hour to an address that sends no
credential, and `chaps` spends them on questions it has to ask somebody:
chap-core's release list, the default branch and commits of every model
repository behind a `registry pin <id>` line, and both lookups of a
`chaps models add`. One `chaps doctor` on a deployment with five models is a
dozen of them. On a shared egress address - an office, a VPN, a CI runner -
the hour is gone before `chaps` asks for anything at all.

Nothing is broken and nothing failed: every check that needs GitHub skips with
this reason and the rest of the report is exactly as it was. Either wait until
the clock time on the line, which is when the window resets, or hand `chaps` a
token:

```sh
export GITHUB_TOKEN=$(gh auth token)   # or GH_TOKEN, whichever is already set
chaps doctor
```

That is 5000 requests an hour instead of 60. A token with no scopes is enough,
because every question `chaps` asks GitHub is a public read. `chaps doctor`'s
`github api` line says which limit is in force and how much of it is left:

```text
ok    github api  reachable, 4990 of 5000 requests left this hour (token)
```

The token is read from the environment only. `chaps` never stores it and never
prints it - a `-v` trace shows `Authorization: Bearer <token>` with nothing
behind it - and no command-line flag sets one, because a token on a command
line is a token in the shell history.

A 403 that arrives *with* a token reads differently, because the advice to set
one would be wrong:

```text
GitHub refused the request (HTTP 403 with a token; check GITHUB_TOKEN's scopes or the rate limit)
```

That is an expired, revoked or misspelled token, one whose fine-grained
permissions do not include reading public repositories, or a token whose own
5000 are gone. `gh auth status` says which.

## `Can't locate revision identified by` in `chaps logs chap`

```text
ERROR [chap_core.database.database] Error during Alembic migrations:
Can't locate revision identified by 'b4c5d6e7f8a3'
```

The database was migrated by a newer chap-core than the one now running: the
Alembic revision it was stamped with does not exist in this build. It is what
`chaps update --chap-tag` warns about before a move backwards, and the reason
it asks for an answer:

```text
warning: moving chap-core from dev to v2.3.1 can run an older schema against a
database migrated by the newer one; run `chaps backup create` first
```

chap-core logs the error and starts anyway, so `chaps status` says `up` while
the schema is not the one this build expects. Either go back to the newer tag

```sh
chaps update --chap-tag master   # or whichever it was
chaps restart
```

or restore the backup taken before the move (`chaps backup restore`) and start
again from there. See
[Switching chap-core's tag](./updating.md#switching-chap-cores-tag).

## `the pull failed after the pins moved`

```text
Error response from daemon: failed to resolve reference
"ghcr.io/dhis2-chap/chap-worker:dev": ghcr.io/dhis2-chap/chap-worker:dev: not found
error: the pull failed after the pins moved; chap-core is now pinned to dev, and
`chaps update --chap-tag v2.3.1 --yes` puts it back
```

chap-core is two images, `chap-core` and `chap-worker`, and a tag that exists
for one of them does not have to exist for the other: at the time of writing
ghcr serves `chap-core:dev` and has no `chap-worker:dev` at all. The pin has
already moved when the pull runs, so the deployment is left describing images
Docker cannot fetch - the containers keep running what they had. The line names
the way back; `chaps update --list-tags` shows what else there is to move to.
