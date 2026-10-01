# Status and output

## What `chaps status` reports

One line for chap-core, one line per enabled component, one row per model, and
one line saying what it adds up to:

```text
chap-core   up   http://localhost:8700   v2.3.1   auth: on
ocs         up   http://localhost:8790   3 datasets   212.0 MB data
s3          up   internal

MODEL                             STATE                    REACH               LAST PING
chapkit-ewars-model               registered               port 5001           12s ago
chapkit-rwanda-malaria-bym-model  running, not registered  via chap-core       -
auto-arima-chapkit                not running              via chap-core       -
some-other-service                unmanaged                http://c0ffee:8000  3s ago

models without a host port are reachable through chap-core at http://localhost:8700/v2/services/<id>/run/

2 of 3 models are not registered.
  chapkit-rwanda-malaria-bym-model: restart it with `chaps restart --all chapkit-rwanda-malaria-bym-model`
  auto-arima-chapkit: start Chap with `chaps up`, then `chaps logs auto-arima-chapkit`
```

In a deployment without chap-core nothing registers, so the STATE column says
what the model answered itself: `up` when `GET /health` on its host port
answered, `running, not answering` when its container is up and `/health` did
not answer (still starting, or failed: read `chaps logs <id>`), and
`not running`. The closing line counts those instead of registrations, and the
exit code is non-zero while any model is not `up`.

There is one hint per row that needs doing something about, and - when no row
does - a single hint pointing at the one check `status` cannot make itself. A
model whose container started under two minutes ago is not stuck yet, so its
hint is to run `chaps status` again in a minute rather than to restart it. See
[When everything registered](#when-everything-registered) below.

The version is chap-core's own when it publishes one, and otherwise the tag
`.chaps/project.yaml` pins, marked `(pinned)` so nobody reads it as the running
build.

A deployment pinned to a moving tag (`latest`, `master`, `dev`) gets one more
cell, because the tag alone does not say which build is behind it: the digest
the running image was pulled at, and the commit chap-core reports at
`/system/info`. Either is left off when it could not be had. See
[Switching chap-core's tag](./updating.md#switching-chap-cores-tag).

```text
chap-core   up   http://localhost:8190   2.4.0.dev0   master: running cc09e3654ff2, revision 7bf2a98739f4   auth: off
```

`auth: on` means `status` had a token - `CHAP_API_TOKEN` from `.env`, or from
the environment when `.env` sets none - and sent it as `Authorization: Bearer`
on every request; `auth: off` means the API is open to
anyone who can reach the port. See [Authentication](./auth.md).

A chap-core that answers `401` to that token is `up, token rejected` rather than
`down`: it answered, and its log says it started fine. The registry is behind
the same token, so the model table is left out rather than filled with
`not registered` rows it could not check, and the error line under it is about
the token. `status` exits non-zero, and `--json` carries `"state": "rejected"`.
See [401 from the Modeling App](./troubleshooting.md#401-from-the-modeling-app).

Each enabled [component](./components.md) other than chap-core gets a line of
its own, directly under it and in the order the compose files are rendered. OCS
is asked over HTTP at `http://localhost:<port>/health`; the object store
publishes no host port, so the only thing that can be said about it from out
here is whether its container is up.

A `dhis2` line is judged by its container first, exactly as the OCS one is, and
only then asked anything: one request to `/api/ping`, the single route DHIS2
answers without credentials, and the same route its container's healthcheck is
built out of - judged by the status code, not the body. That request is there for
a failure OCS does not have. A Spring context that failed to come up - an
unreadable `dhis2/dhis.conf`, a PostgreSQL extension the image cannot find, a
Flyway checksum that does not match the database - leaves Tomcat running and
serving pages while every `/api/*` request answers 404, so a container that is up
is no evidence at all that the instance works. A 200 from `/api/ping` is, and a
container that is up while the ping goes unanswered reads `starting` rather than
`up`. An instance with no host port cannot be asked from out here and is judged
by its container alone, like the object store.

The line carries no version, and deliberately: no component line does. DHIS2
tells only a logged-in session which version it is running - `/api/system/info`
answers nothing else, and `chaps` holds credentials for no instance - and the
`image_tag` in `.chaps/components.yaml` is not that version either. It is intent:
the tag the next `chaps up` would start, while the container standing there was
created, and migrated `dhis2_db`, at whatever tag was in force when it was
created. Printing the recorded one beside a live container would name a version
that container is not. The recorded tag is `chaps doctor`'s `image dhis2` line,
and moving it is an edit to `.chaps/components.yaml` and a `chaps sync`; see
[Changing the DHIS2 version](./dhis2.md#changing-the-dhis2-version).

A `dhis2` line that reads `up` gets one more line under the verdict while
nothing has recorded a `chaps dhis2 connect` for this deployment:

```text
no models enabled; run `chaps models enable ID` to add one
  chaps has not connected this DHIS2 to Chap; run `chaps dhis2 connect`
```

Being `up` is not being connected: the Modeling App reaches chap-core through a
DHIS2 route that nothing creates on its own, and a deployment can otherwise sit
with both rows `up` and the app unable to see Chap at all. The line is read off
`.chaps/components.yaml` and asks DHIS2 nothing, which is why it says what
*chaps* has recorded; it waits for the row to read `up`, because a DHIS2 that is
not answering cannot be connected to anything. It is not printed on a deployment
without chap-core, where `chaps dhis2 connect` refuses. Once a connect is
recorded the line stops - which is a suppressed hint and not a verdict on the
route, and [`connected_at`, and what it is
not](./dhis2.md#connected_at-and-what-it-is-not) says what that record is worth
and which command actually asks DHIS2.

See [DHIS2](./dhis2.md) for the component, and [a DHIS2 that answers but 404s
every API request](./troubleshooting.md#a-dhis2-that-answers-but-404s-every-api-request)
for what that failure looks like from the outside.

| State | Meaning |
| --- | --- |
| `up` | Answering its health endpoint, or - for a component with no endpoint to ask - running. |
| `starting` | Its container is up but it is not answering yet. |
| `not running` | No container, so nothing to answer. `chaps up` starts it. |

An OCS line carries two more facts when they can be had, which are the two an
operator would otherwise open its landing page for:

| Cell | Where it comes from |
| --- | --- |
| `3 datasets` | `GET /datasets?f=json`, asked only when `/health` has just answered, with a three-second timeout of its own. |
| `212.0 MB data` | `du -sk /app/data` inside the running container (`docker compose exec -T ocs`), bounded at ten seconds. |

Both are silent when they could not be had - an instance that is down, an older
OCS without the JSON dataset list, a docker that could not be asked - so the
line is the one it has always been rather than one with holes in it. A size is
only read from a container that is running; what the data volume holds while
the container is down is [`chaps doctor`](./doctor.md)'s to report, where the
question is how much data a `chaps down --volumes` would destroy. Under
`--json` the two are `components[].datasets` and `components[].data_bytes`,
both `null` where the line says nothing.

A deployment with `chap-core` disabled has no chap-core line at all, and
`status` does not exit non-zero over an API that is not meant to be there. Its
closing line counts components rather than models - `both components are up;
`chaps open ocs` opens it`,
or `1 of 2 components is not running; start it with `chaps up`` - because it
has no models to count and the models line would name `chaps models enable`,
the one command such a deployment refuses. See
[Standalone OCS](./components.md#standalone-ocs).

Every model the project enables gets a row, registered or not, with its state
taken from the registry and `docker compose ps` together.

| State | Meaning |
| --- | --- |
| `registered` | The container is up and chap-core knows about it. |
| `running, not registered` | The container is up but chap-core has never heard from it. |
| `not running` | The project enables it, but no container exists or it is stopped. |
| `unmanaged` | chap-core has a service registered that this project does not enable. |

`via chap-core` in `REACH` means the model publishes no host port of its own;
the way in is printed once, under the table, rather than repeated per row. A
model that does publish one reads `port 5001`. `--json` is unchanged: it
carries `internal` and the full `http://localhost:5001` as it always has. See
[Ports](./ports.md).

A model whose container is up but which never registered is the interesting
case. chapkit stops trying five attempts into its startup, so one that came up
before chap-core was healthy stays invisible until it is restarted, which is
what its hint says. The hint asks for `chaps restart --all <id>` rather than a
plain `chaps restart`: nothing about the service has changed, so there is
nothing a restart would recreate on its own, and `--all` is what recreates it
anyway.

### When everything registered

When every model is registered there is nothing to fix and still one thing to
do, on its own indented line under the closing line:

```text
all 5 models registered
  run `chaps models test --all` to check they can run
```

Registration is a heartbeat. It says the service is alive and talking to
chap-core, not that it can train or predict - a model whose runtime is broken,
or whose account cannot write, registers exactly like a working one and fails
on the first real job. `chaps status` cannot settle that without running the
models, which takes minutes; `chaps models test` does, at two levels. See
[Testing a model](./models.md#testing-a-model).

## What `up` means

`up` means chap-core answered *as* chap-core: `/health` has to be JSON with a
`status` field, and `/v2/services` has to parse as `{count, services}`. A bare
200 from something else holding the port, a dev server, a proxy, an older
deployment, is reported as down, naming what answered instead:

```text
chap-core   down   http://localhost:8700   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8700 is not responding: port 8700 answers
but it is not chap-core (got text/html)
```

The check is lenient about the fields chap-core sends, since it may add fields,
rename optional ones or leave one empty and none of that should turn
`chaps status` into a crash, and strict about who is answering.

A chap-core whose own container is up and failing its healthcheck is a
different answer from a port nobody is listening on, and it says so:

```text
chap-core   down (container unhealthy)   http://localhost:8700   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8700 is not responding: ...
why chap is unhealthy:
  sqlalchemy.exc.OperationalError: (psycopg2.OperationalError) ... password authentication
  failed for user "chap"
  the database volume holds a different password than .env (a previous deployment with the
  same name, or --fresh-env); run `chaps doctor`, or remove the volume with
  `chaps down --volumes` if this deployment's data can go
```

The lines are the tail of the container's own log, filtered to the ones that
look like the failure. See
[Troubleshooting](./troubleshooting.md#dependency-failed-to-start-container--is-unhealthy).

## Exit codes and short-circuits

`chaps status` exits non-zero when anything this deployment declares is not
where it should be: the API down, a model not registered, or a component not
`up`. One rule for every deployment shape, because a script polling it as a
health gate cannot know which shape it is polling, and a component that died
has to be as visible as a model that did not register.

The rows have already named what is wrong - which models are missing and what
to do about each one, which component is not up - so that exit adds nothing
further; only an API that is not answering prints its one error line, because
nothing else on the screen says why. `starting` counts as not up: a component
whose container is running but not answering is a deployment that is not ready,
whatever it will be a moment later.

A project whose containers do not exist at all skips the model table: nothing
can have registered with a chap-core that has never started. A deployment that
is chap-core and nothing else is then the one line

```text
Chap is not running; start it with `chaps up`
```

A deployment that has components prints their rows first and puts that line
under them, in the shape a running deployment has. The rows are read from
`.chaps/components.yaml` rather than from docker, so they are known whether
anything is up or not, and they say the one thing the single line cannot: which
components this deployment is made of, and where each of them will answer.

```text
ocs   not running   http://localhost:8790
s3    not running   internal

nothing in this deployment is running; start it with `chaps up`
```

That wording is the other half of it: a deployment chap-core is not a component
of is never told that Chap is not running, because there is no Chap in it to be
running.

`--url` turns the short-circuit off, because then the question is about that
API and not about this machine. `--url` also overrides the default of
`http://localhost:<api_port>`, and `--timeout SECONDS` (5 by default) bounds
each request.

## Never silent

Every command ends with a line saying what it did or found, plus the next step
when there is one; empty output is a bug.

- `chaps logs` on a deployment that was never started says so instead of
  printing nothing, and fails, where bare `docker compose logs` prints nothing
  and exits 0.
- `chaps logs SERVICE` lists the services when `SERVICE` is not one of them.
- `chaps down` reports how many containers it stopped, and that the volumes are
  still there, under the compose project name they are prefixed with;
  `chaps down --volumes` reports the volumes it removed, by name.
- `chaps up` ends with which services it started or recreated and which it left
  alone.
- `chaps docker pull` says how many images it pulled.
- `chaps docker ps` on a project with no containers says
  `nothing is running for this project` rather than printing a bare header.

- `chaps models test` prints a header saying which level is being run, one row
  per model with the verdict, the time and what happened, and a closing line
  that counts the three buckets; a model that could not be tested says why and
  what to do about it rather than being left out.

Output that reads as status verifies what it claims rather than assuming it.
Where something is wrong there is one summary line and one hint per problem,
instead of the same fact three times over.

## Output

On a terminal the answer is coloured and errors arrive in a rounded box; piped
into a file or another program it is the same plain text it has always been, so
nothing that parses `chaps` output has to change. `--no-color`, or `NO_COLOR`
in the environment, turns the colour off and keeps the shapes.

`-v` (`--verbose`) narrates what a command does on the way to its answer, on
stderr, dimmed and never on stdout:

```text
$ chaps -v status
project: /srv/chapx (state in /srv/chapx/.chaps/project.yaml)
registry: https://raw.githubusercontent.com/... from the cache (2 hours old) (7 models)
asking chap-core at http://localhost:8700
GET http://localhost:8700/health -> 200 in 12ms
GET http://localhost:8700/v2/services -> 200 in 8ms
chap-core   up   http://localhost:8700   v2.3.1   auth: on
```

`-d` (`--debug`) implies `-v` and adds what came back: response bodies cut to
2 KB, the raw `docker compose ps` JSON, and the resolved path of the project
state a command read. Both are global, so they work with `--json` too: stdout
stays exactly one document either way.

## `--json`

`--json` works everywhere and prints exactly one document on stdout. A line the
wrapper has to say for itself goes to stderr there, so a parser's stdin stays
that one document. Warnings always go to stderr for the same reason, and an
error under `--json` is a JSON object with `error` and `causes`.

```sh
chaps --json status | jq '.models[] | select(.state != "registered")'
chaps --json status | jq '.components'   # one entry per enabled component
chaps --json docker ps
chaps --json docker config        # the merged configuration as JSON
```

`chaps ui` is the one command that rejects `--json`: it owns the terminal, so
there is nothing to serialise.

The Docker wrappers exit with Compose's own exit code, so `chaps up` is a
drop-in for `docker compose up` in a script.
