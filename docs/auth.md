# Authentication

A deployment is unprotected by default: anything that can reach chap-core's
port can use its API. Turning authentication on is one flag at `init` time, or
one command afterwards.

```sh
chaps init mychap --models default --api-token   # at creation
chaps auth enable                                # on a project that already exists
```

## What the token protects

`CHAP_API_TOKEN` gates the whole API. Clients send it as an HTTP header:

```sh
curl -H "Authorization: Bearer $TOKEN" http://localhost:8700/v2/services
```

Three paths stay open, because they are not credentials-bearing calls: `/health`
and `/health/ready`, which the container healthcheck calls with no headers at
all, and `/system/info`, which is how a client discovers that a token is
required in the first place. Everything else needs the token, `/v2/services`
and `/docs` included.

The token is the one thing a client has to carry. For DHIS2 that is the `chap`
route rather than the Modeling App: the app calls DHIS2, DHIS2 proxies the
call to chap-core, and `chaps dhis2 connect` writes the token into the route's
`auth` as an `Authorization: Bearer` header, which DHIS2 stores and never lists
back. Anything else calling the API needs it from `chaps auth show --reveal`.

## How the models authenticate

`SERVICEKIT_REGISTRATION_KEY` is a second, independent secret. It is not an API
token: chap-core accepts it only under `/v2/services`, which is where a chapkit
service registers itself and sends its keepalive pings. A model presents it as
`X-Service-Key`, because servicekit can send no other header.

`chaps` writes both secrets together, and this is why: once the API is
protected, an unauthenticated registration is rejected like any other request,
so a model service that sends nothing never appears in `chaps status`. The
registration key is what it sends instead.

Nothing has to be copied for that to work. Every model overlay carries

```yaml
    environment:
      SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register
      SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}
```

and Compose substitutes the value from the `.env` beside the compose files, the
same file chap-core reads its own copy from. The two ends cannot disagree.
`chaps sync` writes that line when `.chaps/project.yaml` says the project has a
key, and comments it out again when it does not.

chap-core needs the same value, and upstream's `compose.ghcr.yml` passes only
`CHAP_API_TOKEN` into the `chap` service - so the key never reached the
container and the API answered every registration with

```text
{"detail": "Missing or invalid API token"}
```

`compose.chaps.yml` is where `chaps` puts that right:

```yaml
services:
  chap:
    environment:
      SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}
```

Compose merges `environment` maps across `-f` files additively, so this adds
the one variable and leaves upstream's file untouched. It is rendered whether
or not the deployment has a key: with none in `.env` the variable arrives
empty, which chap-core reads as no key at all.

## Where the secrets live

In `.env`, and nowhere else:

```text
CHAP_API_TOKEN=d1f0...
SERVICEKIT_REGISTRATION_KEY=9a3c...
```

`.chaps/project.yaml` records two booleans and no values:

```yaml
auth:
  api_token: true
  registration_key: true
```

That split is deliberate. `.chaps/` is intent, it is small, and it is the kind
of thing that gets committed, archived and pasted into a bug report; `.env` is
the file Compose reads and the one to keep out of version control.

A generated secret is 64 lowercase hex characters, the same thing
`openssl rand -hex 32` produces. An explicit `--api-token VALUE` (or `chaps
auth enable --token VALUE`) is used verbatim; below 32 characters `chaps`
warns, and so does chap-core at startup, because its API has no rate limiting
and a short token is guessable by anyone who can reach the port.

Verbatim includes `$`: a token with characters Compose would read as anything
but themselves is written in single quotes (`CHAP_API_TOKEN='$abc def'`), which
Compose reads literally. Written bare, `$abc` would be expanded, to an empty
string when no such variable is set, and an empty `CHAP_API_TOKEN` is
authentication off. A quote, a backslash or a control character cannot be
written that way, so a token containing one is refused.

`chaps` reads `.env` by Compose's rules as well: `CHAP_API_PORT=8701 # mine` is
port `8001` to both, because a `#` after a space starts a comment. A `#`
without a space before it (`val#ue`) is part of the value.

## Turning it on and off

| Command | What it does |
| --- | --- |
| `chaps auth show [--reveal]` | Whether each secret is set, and the token only with `--reveal`; plus whether each OCS data source is set, never its value. |
| `chaps auth enable [--token VALUE]` | Write both secrets, record them, re-render the overlays. |
| `chaps auth disable` | Comment both lines out, keeping their values, and re-render. |
| `chaps auth rotate` | Replace both secrets with new ones. |

`chaps auth show` reads both ends: `.chaps/project.yaml` for what the
deployment intends and `.env` for what is actually set, and says so when the two
disagree - a token recorded as in use but commented out in `.env`, say. The
token itself is printed only when `--reveal` asks for it.

On a deployment with the `ocs` component it also lists the OCS data source
credentials, which live in the same `.env`:

```text
OCS data sources
  ECMWF_DATASTORES_URL  set
  ECMWF_DATASTORES_KEY  set
  EDH_API_KEY           unset
  CDSE_S3_ACCESS_KEY    unset
  CDSE_S3_SECRET_KEY    unset
```

Those five are read and never written: they are accounts with Copernicus and
Earth Data Hub rather than this deployment's own secrets, so `enable`, `disable`
and `rotate` do not touch them, and their values stay hidden even under `--reveal` -
there is nothing to paste into a client, only the question of whether a dataset
will ingest. `--json` carries them under `ocs_data_sources`. See
[Components](./components.md#data-source-credentials).

All four only ever touch those two `.env` lines. The database password, the
image pins, the comments and the blank lines come out byte for byte as they
went in, which is what makes them safe on a file an operator has edited.

They read and write the file by Compose's rules, which is what makes the answer
they report the answer Compose will act on: the **last** active assignment of a
variable is the live one, `export KEY=` and quoted values are read as Compose
reads them, and a write rewrites the variable's first active assignment and
removes every later duplicate of it. So a `.env` that somehow carries two
`CHAP_API_TOKEN=` lines comes out of `chaps auth rotate` with one, holding the
new token - rather than with the old, lower line still deciding what chap-core
enforces. See [the `.env` contract](./concepts.md#the-env-contract).

```sh
chaps auth enable
```

```text
API authentication is on
  API token         written to .env; `chaps auth show --reveal` prints it
  Registration key  written to .env; every model overlay now sends it

written  compose.chapkit-ewars-model.yml

run `chaps up` to restart chap-core and the models with authentication
the DHIS2 `chap` route carries it once `chaps dhis2 connect` has run; any other client needs it from `chaps auth show --reveal`
```

`chaps up` is not optional. chap-core and the model containers read `.env` when
Compose creates them, so nothing changes for a container that is already
running until it is recreated.

`chaps auth disable` comments the lines out rather than deleting them:

```text
# CHAP_API_TOKEN=d1f0...
```

so the token your clients are configured with is still recoverable. `chaps auth
enable` afterwards picks those values back up rather than generating new ones,
which makes an accidental `disable` a round trip.

## Rotation

```sh
chaps auth rotate
chaps up
```

Rotating is two steps, and the second one matters: until `chaps up` recreates
the containers, chap-core is still enforcing the old token, and after it every
client that has not been updated gets a 401. Update them in the same sitting:
`chaps dhis2 connect` rewrites the DHIS2 route with the new token, and anything
else calling the API needs it by hand.

The registration key rotates with the token, and the models pick it up from the
same `chaps up`, so there is nothing to do for them.

## Reading it back

```sh
chaps status
```

```text
chap-core   up   http://localhost:8700   v2.3.1   auth: on
```


For a script rather than a screen, `chaps auth token` prints the token and
nothing else:

```sh
TOKEN=$(chaps auth token)
curl -H "Authorization: Bearer $TOKEN" http://localhost:8700/v2/services
```

On a deployment with authentication off it prints nothing on stdout, says so on
stderr and exits 1, so a caller that captured an empty string stops rather than
sending an empty header. `chaps api` needs none of this - it reads `.env`
itself - and `chaps auth show --reveal` is the spelling for reading the value
rather than capturing it. See [Jobs and the API](./jobs.md).

`chaps status` reads the token out of `.env` and sends it on every request, so
it keeps working on a protected deployment, and the last cell of the chap-core
line says which mode the deployment is in. A 401 is reported as a token problem
rather than as "not chap-core": see
[Troubleshooting](./troubleshooting.md#401-from-the-modeling-app).

## `--fresh-env` regenerates it

`init --fresh-env` renders a whole new `.env`, which means a new database
password and, if `--api-token` is passed with it, a new token and registration
key. Without `--api-token` the new file has both secrets commented out, so the
deployment comes back unprotected. `init` on a directory that already has a
`.env` keeps that file, `--force` included, so `--api-token` there does nothing
but warn and point at `chaps auth enable`. See the
[`.env` contract](./concepts.md#the-env-contract).
