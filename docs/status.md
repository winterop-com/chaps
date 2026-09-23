# Status and output

## What `chaps status` reports

One line for chap-core, one row per model, and one line saying what it adds up
to:

```text
chap-core   up   http://localhost:8000   v2.3.1   auth: on

MODEL                             STATE                    REACH                  LAST PING
chapkit-ewars-model               registered               http://localhost:5001  12s ago
chapkit-rwanda-malaria-bym-model  running, not registered  internal               -
auto-arima-chapkit                not running              internal               -
some-other-service                unmanaged                http://c0ffee:8000     3s ago

internal models are reachable through chap-core at http://localhost:8000/v2/services/<id>/run/

2 of 3 models are not registered.
  chapkit-rwanda-malaria-bym-model: restart it with `chaps docker run restart chapkit-rwanda-malaria-bym-model`
  auto-arima-chapkit: start the stack with `chaps up`, then `chaps logs auto-arima-chapkit`
```

The version is chap-core's own when it publishes one, and otherwise the tag
`.chaps/project.yaml` pins, marked `(pinned)` so nobody reads it as the running
build.

`auth: on` means `.env` sets `CHAP_API_TOKEN`, and that `status` sent it as
`Authorization: Bearer` on every request; `auth: off` means the API is open to
anyone who can reach the port. See [Authentication](./auth.md).

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
what its hint says.

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

## Exit codes and short-circuits

`chaps status` exits non-zero when the API is down or a model has not
registered, which makes it a health gate for a script. The table has already
named every model that is missing and what to do about each one, so that exit
adds nothing further; only an API that is not answering prints its one error
line.

A project whose containers do not exist at all skips the table and says

```text
stack is not running; start it with `chaps up`
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
  still there.
- `chaps up` ends with which services it started or recreated and which it left
  alone.
- `chaps docker pull` says how many images it pulled.
- `chaps docker ps` on a project with no containers says
  `nothing is running for this project` rather than printing a bare header.

Output that reads as status verifies what it claims rather than assuming it.
Where something is wrong there is one summary line and one hint per problem,
instead of the same fact three times over.

## `--json`

`--json` works everywhere and prints exactly one document on stdout. A line the
wrapper has to say for itself goes to stderr there, so a parser's stdin stays
that one document. Warnings always go to stderr for the same reason, and an
error under `--json` is a JSON object with `error` and `causes`.

```sh
chaps --json status | jq '.models[] | select(.state != "registered")'
chaps --json docker ps
chaps --json docker config        # the merged stack as JSON
```

`chaps ui` is the one command that rejects `--json`: it owns the terminal, so
there is nothing to serialise.

The Docker wrappers exit with Compose's own exit code, so `chaps up` is a
drop-in for `docker compose up` in a script.
