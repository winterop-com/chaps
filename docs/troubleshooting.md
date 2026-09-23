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
chaps docker run restart chapkit-rwanda-malaria-bym-model
chaps status
```

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

If it persists, check that the overlay carries the line:

```sh
grep SERVICEKIT_REGISTRATION_KEY compose.chapkit-ewars-model.yml
```

An active `SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}` is
what a protected deployment needs; a commented one means `.chaps/project.yaml`
does not know the project has a key, which `chaps auth show` will report as a
mismatch and `chaps auth enable` puts right.

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

## `password authentication failed` after `--fresh-env`

`init --fresh-env` rotates the PostgreSQL password, but the existing data
volume was created with the old one, so chap-core can no longer authenticate
against its own data.

Either drop the volume and start over:

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

## Compose is older than 2.20

```text
warning: docker compose 2.18.1 is older than 2.20.0; compose.marketplace.yml uses `include:`, which needs 2.20.0 or newer
```

`compose.marketplace.yml` uses `include:`, which arrived in Compose 2.20, and
`compose.chaps.yml` uses `!override`, which arrived in 2.24. `chaps` warns
rather than failing, because the base services still run, but the model overlays
are the part that will not load. Upgrade Docker Compose.

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

## `chaps update` fails offline

By design. `chaps update` fetches the registry from the network with no cache
and no fallback, `--dry-run` included, because a plan made from a stale
catalogue is not a plan. Every other command falls back to the cache and then
to the snapshot compiled into the binary.
