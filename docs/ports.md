# Ports

## One published port

A deployment of chap-core and models publishes **one** host port: chap-core's
API, 8700 by default and `varde init --api-port N` otherwise. Nothing else in it
needs one. The opt-in [components](./components.md) are the exception, and each
one publishes a well-known port of its own: see
[Component ports](#component-ports).

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
curl http://localhost:8700/v2/services/chapkit-ewars-model/run/api/v1/info
open http://localhost:8700/v2/services/chapkit-ewars-model/run/docs

# or give that model a port of its own
varde models expose chapkit-ewars-model            # lowest free port, 5001 up
varde models expose chapkit-ewars-model --port 5010
varde models unexpose chapkit-ewars-model          # and take it away again
varde up                                           # apply either change
```

In a deployment without chap-core there is no proxy, so a model gets a host
port when it is enabled, from the same range; see
[Model services without chap-core](./components.md#model-services-without-chap-core).

`expose` and `unexpose` only rewrite the overlay's `ports:`; they never
re-resolve the version, so they are safe on a deployment running a build you do
not want moved. `varde models enable ID --port N|auto` does the same thing
while enabling.

`varde models list` shows either the port or `via chap-core`, `varde status`
says the same in its `REACH` column, and
`varde models info ID` prints the proxy URL for a model with no port of its
own. In `varde ui`, `p` opens a prompt on the row under the cursor that takes
a port number, `auto`, or nothing at all, and `P` takes the port away.

`p` works the same way on the browser's components page, with a narrower
prompt: a number, or an empty line or `none`, and **not** `auto`. `auto` means
the lowest free port in the model range, and a component publishes a well-known
port of its own outside it, so there is nothing for `auto` to pick from. `p` on
`chap-core` is refused rather than prompted - its host port is the API port,
which `CHAP_API_PORT` in `.env` sets - and the footer names that line instead. See
[The components page](./models.md#the-components-page).

Model ports come from the range 5001 to 5999, with 5001 the default lowest
(`init --port-base` moves it). A port has to be free twice over: unclaimed by
`.varde/models.yaml`, by every `compose*.yml` in the directory and by
chap-core's API port as `.env` sets it, and with nothing on the machine
listening on it. So two models never collide, and
neither does a model and something else you are running. `--port N` claims one
explicitly, and the command fails if it is taken.

## Which address a model port is published on

Compose publishes a host port on every address the machine has, so a model
with a port is reachable from the network unless a firewall says otherwise.
Models have no login, so on a laptop or a shared server that is often more
than you meant. `--bind` names the address instead:

```sh
varde models enable chapkit_ewars_model --port auto --bind 127.0.0.1
varde models expose chapkit_ewars_model --bind 127.0.0.1   # an enabled one
varde models add ghcr.io/org/model:latest --port auto --bind 127.0.0.1
```

The overlay then publishes `"127.0.0.1:5001:8000"`, which only this machine's
own processes can reach: `docker port <container>` shows `127.0.0.1:5001`. A
model's own `--bind` is recorded as `bind:` in `.varde/models.yaml` and wins over
the deployment's `model_bind:` in `.varde/project.yaml`. The deployment that
`varde run` keeps sets `model_bind:` to `127.0.0.1`. With neither, the port is
published on every address as before. `--bind 0.0.0.0` asks for every
address explicitly, which is how one model leaves a loopback default. An IPv6
address is written bare (`--bind ::1`) and rendered in brackets.
[`varde run`](./run.md) groups publish every model on `127.0.0.1`.

The URLs varde prints say `localhost`, which is right for every address and
for loopback. For a model bound to one LAN address, use that address instead.

A model registered with a chap-core elsewhere must stay reachable from it:
chap-core calls it back on its host port, and a loopback-only port does not
answer a container. `varde sync` warns when a model is set up that way and
names the `expose --bind 0.0.0.0` that undoes it.

## Component ports

A component publishes a well-known port of its own rather than one from the
model range:

| Component | Default host port | Why that one |
| --- | --- | --- |
| `ocs` | 8790 | OCS serves a web interface as well as an API, so it is published. |
| `s3` | none | OCS reaches the object store at `http://s3:9000` inside the deployment; nothing out here needs it. |
| `dhis2` | 8780 | Not the container's 8080, which a DHIS2 already running on this machine likely holds. DHIS2 is a web application people log into. |

A component port that one of this deployment's own models already publishes
is warned about when it is set, whether or not that model is up, with the next
free port as the way out: `varde up` could not start both.

Each is set at creation time or afterwards, and `none` is how any of them is
kept off the host entirely:

```sh
varde init mychap --with ocs,s3 --ocs-port 9010 --s3-port 9002
varde init mychap --with ocs --ocs-port none        # reachable only inside
varde init mychap --with dhis2 --dhis2-port 18080
varde components enable ocs --port 9010             # or afterwards
varde components enable dhis2 --port none           # behind a reverse proxy
```

`--ocs-port`, `--s3-port` and `--dhis2-port` each need their component: asking
for a port for something this deployment is not getting is refused rather than
silently ignored, because a port that quietly did nothing would leave you
waiting for a service on an address no file mentions.

`varde open NAME` opens the component at that port in a browser - and says so
rather than opening anything when the component publishes none. See
[Reaching a component from a browser](./components.md#reaching-a-component-from-a-browser).

8000, 8080 and 9000 are what most development servers default to, so varde's
own services start one block away from them: chap-core on 8700, DHIS2 on 8780
and OCS on 8790, while inside the deployment they keep their container ports
(`http://chap:8000`, `http://ocs:9000`). Two deployments on the same defaults
still collide with each other. Neither the `init` probe nor `varde components
enable` refuses a taken port; both warn, name a free one, and put the way out
on the same line:

```text
warning: port 8780 is also used by demo (/home/me/demo), which is not running; both cannot be up at once. Keep it, or run `varde components enable dhis2 --port 8781`
```

`varde components enable` probes the port it is given the same way `init` does,
and warns rather than refuses:

```text
enabled ocs on http://localhost:8790
warning: port 8790 is already in use on this machine (needed by ocs); free it, or run `varde components enable ocs --port 8791`
run `varde up` to apply
```

A warning, because the listener is often something you are about to stop, and
because nothing is started here: `varde up` is where a taken port becomes a
refusal. A port one of this deployment's own running services already publishes
is not a conflict at all. As at `init`, a port no one is listening on but
another deployment on this machine claims gets the same line with that
deployment named.

## How the API port is set

The API's port is not an edit of `compose.yml`. That file is upstream's, byte
for byte. `varde sync` renders a small `compose.varde.yml` next to it and puts
it in the `-f` list:

```yaml
services:
  chap:
    ports: !override
      - "${CHAP_API_PORT:-8700}:8000"
```

`!override` (Compose 2.24.4+, which is why that is the supported minimum)
replaces chap's own mapping rather than adding to it, which is why the API ends
up on exactly one port; a file listed in `include:` could not do that at all,
which is why `compose.varde.yml` is a separate `-f` entry rather than part of
the umbrella.

The port comes from `api_port` in `.varde/project.yaml`, and `CHAP_API_PORT` in
`.env` overrides it without touching `.varde/`.

Because Compose reads `.env` last, that line wins - and `varde` follows it, so
the CLI and Compose always talk about the same port. It is the port
`varde status` probes (unless `--url` names another), the port `varde up`'s
preflight reserves, and the port `varde doctor` checks - and the checklist names
the file it came from when the two disagree:

```text
ok    api port                   18000 is free (from .env, over the 8700 recorded in .varde/project.yaml)
```

`varde status --json` carries the same answer as two fields, `api_port` and
`api_port_source` (`env` or `project`). A `CHAP_API_PORT=` line that is
commented out, empty or not a port is not an override, and the recorded value
stands.

`init` never rewrites a `.env` it finds, since the database password in it
outlives the rest, so

```sh
varde init --api-port 8710 --force
```

over an existing deployment moves `.varde/project.yaml` and `compose.varde.yml`
and then warns that the older `CHAP_API_PORT=` line is still what Chap
will use. Edit that line, or pass `--fresh-env` (which rotates the database
password too; see the [`.env` contract](./concepts.md#the-env-contract)).

That is why every message about a busy API port - from `init`, `up`'s
preflight and `doctor` - names that `.env` line and not `init --force`: the
line is there from the first `init` on, so it is the one edit that moves the
port.

`CHAP_API_PORT` is written as an active line even at 8700, so the one published
port is discoverable by reading `.env`.

## The preflight

`varde up` checks that every host port this deployment is about to publish is free
before it calls Docker, and refuses with one line per conflict:

```text
2 host ports this deployment needs are already in use; nothing was started
  port 8700 is already in use on this machine (needed by chap); free it, or set
  CHAP_API_PORT=8701 in `.env`
  port 5001 is already in use on this machine (needed by chapkit-ewars-model);
  free it, or run `varde models unexpose chapkit-ewars-model` (the model stays
  reachable through chap-core) / `varde models expose chapkit-ewars-model --port auto`
  or run `varde up --no-preflight` to hand the conflict to Docker
```

Docker finds the same conflict eventually, several seconds in and named after a
container rather than a port.

- Ports held by this project's own running containers are skipped, so
  `varde up` on a running deployment stays a no-op.
- `varde up --no-preflight` hands the question back to Docker.
- A port another varde deployment publishes is named with that deployment and
  the command that stops it, and at a terminal `up` offers to stop it and start
  this one instead; `varde up --replace` does that without asking. See
  [A host port is already in use](./troubleshooting.md#a-host-port-is-already-in-use).
- `varde init` probes the API port too, but only warns: the process holding it
  is often a previous deployment you are about to replace. It warns as well
  when nothing is listening and another deployment already uses that port -
  one in a directory beside the new one, or one Docker has started at least
  once from anywhere - naming it and the first port above that neither a
  listener nor another deployment holds. Both stay warnings, and 8700 stays
  the default: `varde init hello1 && varde init hello2` writes two deployments
  on one port that take turns, and `--api-port N` or `CHAP_API_PORT` in `.env`
  moves either one. The same goes for a component's port, such as the one
  `ocs` publishes. A deployment written somewhere else and never started is in
  neither search, so that collision still waits for `varde up`.

The probe is a bind, not a connect, so it needs no privileges and leaves
nothing behind: a listening socket has no `TIME_WAIT`. It takes four binds
rather than one, because `std` sets `SO_REUSEADDR` on Unix and on BSD (macOS
included) that lets a wildcard bind succeed next to a loopback-only listener
and the other way round. Only the same address reliably collides, so every
address Chap could be published on is tried in turn.

## Internal-only models and the proxy URL

`via chap-core` - in `varde status`, in `varde models list` and in the
browser's `PORT` column - means the model publishes no host port of its own.
The way in is chap-core:

```text
http://localhost:8700/v2/services/<service_id>/run/
```

`varde status` prints that line once, under the table, rather than repeating a
URL per row. `varde models info ID` prints the model's own proxy URL. The
model's chapkit endpoints hang off it, so `/run/api/v1/info` and `/run/docs`
both work.
