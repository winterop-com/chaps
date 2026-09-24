# Status and output

## What `chaps status` reports

One line for chap-core, one line per enabled component, one row per model, and
one line saying what it adds up to:

```text
chap-core   up   http://localhost:8000   v2.3.1   auth: on
ocs         up   http://localhost:9000   3 datasets   212.0 MB data
s3          up   internal

MODEL                             STATE                    REACH                  LAST PING
chapkit-ewars-model               registered               http://localhost:5001  12s ago
chapkit-rwanda-malaria-bym-model  running, not registered  internal               -
auto-arima-chapkit                not running              internal               -
some-other-service                unmanaged                http://c0ffee:8000     3s ago

internal models are reachable through chap-core at http://localhost:8000/v2/services/<id>/run/

2 of 3 models are not registered.
  chapkit-rwanda-malaria-bym-model: restart it with `chaps restart --all chapkit-rwanda-malaria-bym-model`
  auto-arima-chapkit: start CHAP with `chaps up`, then `chaps logs auto-arima-chapkit`
```

The version is chap-core's own when it publishes one, and otherwise the tag
`.chaps/project.yaml` pins, marked `(pinned)` so nobody reads it as the running
build.

`auth: on` means `.env` sets `CHAP_API_TOKEN`, and that `status` sent it as
`Authorization: Bearer` on every request; `auth: off` means the API is open to
anyone who can reach the port. See [Authentication](./auth.md).

Each enabled [component](./components.md) other than chap-core gets a line of
its own, directly under it and in the order the compose files are rendered. OCS
is asked over HTTP at `http://localhost:<port>/health`; the object store
publishes no host port, so the only thing that can be said about it from out
here is whether its container is up.

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
`status` does not exit non-zero over an API that is not meant to be there.

Every model the project enables gets a row, registered or not, with its state
taken from the registry and `docker compose ps` together.

| State | Meaning |
| --- | --- |
| `registered` | The container is up and chap-core knows about it. |
| `running, not registered` | The container is up but chap-core has never heard from it. |
| `not running` | The project enables it, but no container exists or it is stopped. |
| `unmanaged` | chap-core has a service registered that this project does not enable. |

`internal` in `REACH` means the model publishes no host port of its own; the
way in is printed once, under the table, rather than repeated per row. See
[Ports](./ports.md).

A model whose container is up but which never registered is the interesting
case. chapkit stops trying five attempts into its startup, so one that came up
before chap-core was healthy stays invisible until it is restarted, which is
what its hint says. The hint asks for `chaps restart --all <id>` rather than a
plain `chaps restart`: nothing about the service has changed, so there is
nothing a restart would recreate on its own, and `--all` is what recreates it
anyway.

## What `up` means

`up` means chap-core answered *as* chap-core: `/health` has to be JSON with a
`status` field, and `/v2/services` has to parse as `{count, services}`. A bare
200 from something else holding the port, a dev server, a proxy, an older
deployment, is reported as down, naming what answered instead:

```text
chap-core   down   http://localhost:8000   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8000 is not responding: port 8000 answers
but it is not chap-core (got text/html)
```

The check is lenient about the fields chap-core sends, since it may add fields,
rename optional ones or leave one empty and none of that should turn
`chaps status` into a crash, and strict about who is answering.

A chap-core whose own container is up and failing its healthcheck is a
different answer from a port nobody is listening on, and it says so:

```text
chap-core   down (container unhealthy)   http://localhost:8000   v2.3.1 (pinned)   auth: off
...
error: chap-core at http://localhost:8000 is not responding: ...
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

`chaps status` exits non-zero when the API is down or a model has not
registered, which makes it a health gate for a script. The table has already
named every model that is missing and what to do about each one, so that exit
adds nothing further; only an API that is not answering prints its one error
line.

A project whose containers do not exist at all skips the table and says

```text
CHAP is not running; start it with `chaps up`
```

`--url` turns that short-circuit off, because then the question is about that
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
registry: https://raw.githubusercontent.com/... from the cache (2 hours old) (6 models)
asking chap-core at http://localhost:8000
GET http://localhost:8000/health -> 200 in 12ms
GET http://localhost:8000/v2/services -> 200 in 8ms
chap-core   up   http://localhost:8000   v2.3.1   auth: on
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
