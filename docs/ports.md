# Ports

## One published port

A deployment publishes **one** host port: chap-core's API, 8000 by default and
`chaps init --api-port N` otherwise. Nothing else needs one.

chap-core reaches each model over the compose default network at
`http://<service_id>:8000`, and that internal URL is what a model registers
with. PostgreSQL and Valkey sit on a second network that model services never
join.

So a model service only gets `expose: ["8000"]`, which declares the container
port without asking the host for anything.

That one port is open to whatever can reach it, unless the deployment has an API
token: see [Authentication](./auth.md).

## Reaching a model

Two ways:

```sh
# through chap-core's read-only proxy, no host port needed
curl http://localhost:8000/v2/services/chapkit-ewars-model/run/api/v1/info
open http://localhost:8000/v2/services/chapkit-ewars-model/run/docs

# or give that model a port of its own
chaps models expose chapkit-ewars-model            # lowest free port, 5001 up
chaps models expose chapkit-ewars-model --port 5010
chaps models unexpose chapkit-ewars-model          # and take it away again
chaps up                                           # apply either change
```

`expose` and `unexpose` only rewrite the overlay's `ports:`; they never
re-resolve the version, so they are safe on a deployment running a build you do
not want moved. `chaps models enable ID --port N|auto` does the same thing
while enabling.

`chaps models list` and `chaps status` show either the port or `internal`, and
`chaps models info ID` prints the proxy URL for an internal-only model. In
`chaps ui`, `p` toggles publishing for the row under the cursor.

Model ports come from the range 5001 to 5999, with 5001 the default lowest
(`init --port-base` moves it). A port has to be free twice over: unclaimed by
`.chaps/models.yaml` and by every `compose*.yml` in the directory, and with
nothing on the machine listening on it. So two models never collide, and
neither does a model and something else you are running. `--port N` claims one
explicitly, and the command fails if it is taken.

## How the API port is set

The API's port is not an edit of `compose.yml`. That file is upstream's, byte
for byte. `chaps sync` renders a small `compose.chaps.yml` next to it and puts
it in the `-f` list:

```yaml
services:
  chap:
    ports: !override
      - "${CHAP_API_PORT:-8000}:8000"
```

`!override` (Compose 2.24+) replaces chap's own mapping rather than adding to
it, which is why the API ends up on exactly one port; a file listed in
`include:` could not do that at all, which is why `compose.chaps.yml` is a
separate `-f` entry rather than part of the umbrella.

The port comes from `api_port` in `.chaps/project.yaml`, and `CHAP_API_PORT` in
`.env` overrides it without touching `.chaps/`.

Because Compose reads `.env` last, that line wins. `init` never rewrites a
`.env` it finds, since the database password in it outlives the rest, so

```sh
chaps init --api-port 9000 --force
```

over an existing deployment moves `.chaps/project.yaml` and `compose.chaps.yml`
and then warns that the older `CHAP_API_PORT=` line is still what CHAP
will use. Edit that line, or pass `--fresh-env` (which rotates the database
password too; see the [`.env` contract](./concepts.md#the-env-contract)).

`CHAP_API_PORT` is written as an active line even at 8000, so the one published
port is discoverable by reading `.env`.

## The preflight

`chaps up` checks that every host port CHAP is about to publish is free
before it calls Docker, and refuses with one line per conflict:

```text
2 host ports CHAP needs are already in use; nothing was started
  port 8000 is already in use on this machine (needed by chap); free it, or run
  `chaps init --api-port 8001 --force` here / set CHAP_API_PORT=8001 in .env
  port 5001 is already in use on this machine (needed by chapkit-ewars-model);
  free it, or run `chaps models unexpose chapkit-ewars-model` (the model stays
  reachable through chap-core) / `chaps models expose chapkit-ewars-model --port auto`
```

Docker finds the same conflict eventually, several seconds in and named after a
container rather than a port.

- Ports held by this project's own running containers are skipped, so
  `chaps up` on a running deployment stays a no-op.
- `chaps up --no-preflight` hands the question back to Docker.
- `chaps init` probes the API port too, but only warns: the process holding it
  is often a previous deployment you are about to replace.

The probe is a bind, not a connect, so it needs no privileges and leaves
nothing behind: a listening socket has no `TIME_WAIT`. It takes four binds
rather than one, because `std` sets `SO_REUSEADDR` on Unix and on BSD (macOS
included) that lets a wildcard bind succeed next to a loopback-only listener
and the other way round. Only the same address reliably collides, so every
address CHAP could be published on is tried in turn.

## Internal-only models and the proxy URL

`internal` in `chaps status` and `chaps models list` means the model publishes
no host port of its own. The way in is chap-core:

```text
http://localhost:8000/v2/services/<service_id>/run/
```

`chaps status` prints that line once, under the table, rather than repeating a
URL per row. `chaps models info ID` prints the model's own proxy URL. The
model's chapkit endpoints hang off it, so `/run/api/v1/info` and `/run/docs`
both work.
