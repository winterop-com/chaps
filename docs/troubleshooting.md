# Troubleshooting

Start with `chaps doctor`. It runs the whole checklist in one pass - Docker,
Compose, the architecture, free disk, the hosts CHAP pulls from, and, inside a
deployment, the files, the ports, the pins, the images and the running stack -
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

Every overlay `chaps` writes ships a one-shot `<service_id>-init` container
that chowns the volume to the model's numeric uid:gid before the model starts,
so this is fixed by construction. If you see it anyway:

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
  `chaps docker run -- down -v` if this deployment's data can go
```

The same lines close `chaps status` and `chaps doctor`'s `stack` check, and
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
      remove it with `chaps docker run -- down -v` if this deployment's data can go, or keep
      both by giving one of them a name of its own
```

A deployment created by this version of `chaps` has a name of its own
(`demo-1ab2c3`) and cannot collide; see
[the compose project name](./concepts.md#the-compose-project-name). An older
one gets that name written down, unchanged, by the next `chaps sync`.

**`init --fresh-env`.** It rotates the PostgreSQL password on purpose, and the
existing volume was created with the old one.

Either way: drop the volume and start over,

```sh
chaps docker run -- down -v
chaps up
```

or change the role to match the new password with `ALTER USER` inside the
running postgres container:

```sh
chaps docker exec postgres psql -U chap -d chap_core
```

This is why `init` never rewrites a `.env` it finds, `--force` included. See
the [`.env` contract](./concepts.md#the-env-contract).

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
