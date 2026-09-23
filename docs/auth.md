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
curl -H "Authorization: Bearer $TOKEN" http://localhost:8000/v2/services
```

Three paths stay open, because they are not credentials-bearing calls: `/health`
and `/health/ready`, which the container healthcheck calls with no headers at
all, and `/system/info`, which is how a client discovers that a token is
required in the first place. Everything else needs the token, `/v2/services`
and `/docs` included.

The token is the one thing a person has to carry around. In DHIS2 it goes into
the Modeling App's CHAP settings; `chaps auth show --reveal` prints it for
pasting.

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
the file Compose reads and the one to keep out of version control. A
`project.yaml` written before this field existed loads as both `false`, which is
what those deployments were.

A generated secret is 64 lowercase hex characters, the same thing
`openssl rand -hex 32` produces. An explicit `--api-token VALUE` is used
verbatim; below 32 characters `chaps` warns, and so does chap-core at startup,
because its API has no rate limiting and a short token is guessable by anyone
who can reach the port.

## Turning it on and off

| Command | What it does |
| --- | --- |
| `chaps auth show [--reveal]` | Whether each secret is set, and the token, abbreviated unless `--reveal`. |
| `chaps auth enable [--token VALUE]` | Write both secrets, record them, re-render the overlays. |
| `chaps auth disable` | Comment both lines out, keeping their values, and re-render. |
| `chaps auth rotate` | Replace both secrets with new ones. |

All four only ever touch those two `.env` lines. The database password, the
image pins, the comments and the blank lines come out byte for byte as they
went in, which is what makes them safe on a file an operator has edited.

```sh
chaps auth enable
```

```text
API authentication is on
  API token         d1f0a2... (`chaps auth show --reveal` prints it)
  Registration key  written to .env; every model overlay now sends it

written  compose.chapkit-ewars-model.yml

run `chaps up` to restart chap-core and the models with authentication
paste this token in the Modeling App's CHAP settings (`chaps auth show --reveal`)
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
the Modeling App's CHAP settings, and anything else calling the API.

The registration key rotates with the token, and the models pick it up from the
same `chaps up`, so there is nothing to do for them.

## Reading it back

```sh
chaps status
```

```text
chap-core   up   http://localhost:8000   v2.3.1   auth: on
```

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
