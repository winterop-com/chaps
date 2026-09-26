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
2 host ports CHAP needs are already in use; nothing was started
  port 8000 is already in use on this machine (needed by chap); free it, or run
  `chaps init --api-port 8001 --force` here / set CHAP_API_PORT=8001 in .env
  port 5001 is already in use on this machine (needed by chapkit-ewars-model);
  free it, or run `chaps models unexpose chapkit-ewars-model` (the model stays
  reachable through chap-core) / `chaps models expose chapkit-ewars-model --port auto`
```

Pick one of the three: free the port, move the API with `CHAP_API_PORT` in
`.env`, or move (or drop) the model's port. Ports held by this project's own
running containers are not conflicts, so `chaps up` on a running deployment is still
a no-op. `chaps up --no-preflight` hands the question back to Docker.

## The port answers, but it is not chap-core

```text
chap-core   down   http://localhost:8000   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8000 is not responding: port 8000 answers
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

The token DHIS2 is sending is not the one chap-core is enforcing. Print the one
this deployment holds and paste it into the Modeling App's CHAP settings:

```sh
chaps auth show --reveal
```

Two ways to get here. Either `chaps auth rotate` was run and the clients were
never updated, which is the second half of rotating; or `chaps auth enable` was
run and `chaps up` was not, so chap-core is still running without the token
while `.env` already has one. `chaps status` tells the two apart:

```text
error: chap-core at http://localhost:8000 is not responding: port 8000 answers
/v2/services with HTTP 401: the API token in .env is not accepted
```

means the running chap-core has a different token, and

```text
chap-core   up   http://localhost:8000   v2.3.1   auth: on
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
