# Concepts

## The project directory

`chaps init` writes a directory you own; nothing outside it is touched.

```text
mychap/
  .chaps/
    project.yaml               intent: schema version, chap-core tag and compose
                               source, registry URL, API port, whether the two
                               auth secrets are in use, the -f list, the model
                               port range, and which root files sync wrote
    models.yaml                intent: the enabled models (image, pinned version,
                               channel, host port or none, data dir, user,
                               platform, overlay name)
    components.yaml            intent: which components this deployment is made of
                               (chap-core, ocs, s3) and their settings
    compose.chap-core.<tag>.yml
                               chap-core's own compose.ghcr.yml at the pinned tag,
                               exactly as downloaded; compose.yml is rendered from it
  compose.yml                  artifact: the base services, from the file above
  compose.chaps.yml            artifact: chaps-owned overrides on top of it, the
                               API's host port
  compose.ocs.yml              artifact: the ocs component, when it is enabled
  compose.s3.yml               artifact: the s3 component, when it is enabled
  compose.marketplace.yml      artifact: include: list, one line per enabled model
  compose.<service_id>.yml     artifact: one overlay per enabled model
  ocs/climate-service.yaml     the OCS instance configuration; scaffolded once by
                               chaps and never rewritten
  .env                         written once by init; only pins, the two auth
                               secrets and the component settings are ever added
```

| File | What it is |
| --- | --- |
| `compose.yml` | The base of the CHAP stack, which is chap-core, its worker and database plus the enabled models: chap-core, worker, Valkey and PostgreSQL, with the models added on top by their overlays. chap-core's own `compose.ghcr.yml` at the pinned tag, so upstream stays the source of truth. Rendered by `sync` from `.chaps/compose.chap-core.<tag>.yml`, or from the copy compiled into the binary when there is none. |
| `compose.chaps.yml` | The chaps-owned settings that sit on top of the base file: today, the API's host port as `ports: !override`. It is a separate `-f` entry because a file in `include:` cannot override a service the main file defines. Rendered from `api_port` in `.chaps/project.yaml`. |
| `.chaps/compose.chap-core.<tag>.yml` | That upstream file as downloaded, one per tag the project has used. Deleting it does not break CHAP; it only means `sync` can no longer re-render `compose.yml`. |
| `.env` | PostgreSQL credentials (the password is 32 random hex characters generated once), the chap-core image tag, `CHAP_API_PORT` (an active line even at 8000, so the one published port is discoverable), the two authentication secrets (`CHAP_API_TOKEN` and `SERVICEKIT_REGISTRATION_KEY`, active lines when the deployment is protected and commented placeholders when it is not), and commented placeholders for `CHAP_DATABASE_URL` and the per-model image pins. |
| `compose.ocs.yml`, `compose.s3.yml` | One per enabled component other than chap-core, rendered from `.chaps/components.yaml`. They sit in the `-f` list between `compose.chaps.yml` and the umbrella, and are removed again when the component is disabled. See [Components](./components.md). |
| `ocs/climate-service.yaml` | The Open Climate Service instance configuration, scaffolded when the `ocs` component is first enabled. It is yours from that moment: `chaps` never rewrites it, and only re-creates it if it goes missing. |
| `compose.marketplace.yml` | An umbrella file whose `include:` list names one overlay per enabled model. With no models enabled it holds `services: {}` instead of an empty `include`. |
| `compose.<service_id>.yml` | One model service, rendered from its `models.yaml` entry. |
| `.chaps/project.yaml`, `.chaps/models.yaml`, `.chaps/components.yaml` | The intent, as above. All three open with a comment saying which commands manage them. A deployment created before `components.yaml` existed reads as chap-core alone, which is what it was. |

## Intent and artifacts

`.chaps/` is the **intent**. It is what `models enable`, `models disable`,
`components enable`, `components disable`, the browser and `update` edit, and it
is small enough to read and to diff.

The compose files at the project root are **artifacts** rendered from it by
`chaps sync`. They are plain Compose files with nothing `chaps`-specific in
them, so

```sh
docker compose -f compose.yml -f compose.chaps.yml -f compose.marketplace.yml up -d
```

works without `chaps` installed, which is the point of generating them.

`compose.yml` is an artifact too. `init` downloads chap-core's own
`compose.ghcr.yml` at the tag it pinned, keeps the raw copy as
`.chaps/compose.chap-core.<tag>.yml` and records its URL and SHA-256 in
`project.yaml`; `sync` renders `compose.yml` as two header lines plus that copy,
byte for byte. Rendering from the copy rather than the network keeps `sync`
offline and deterministic, and it is why a hand edit of `compose.yml` is drift
that `chaps sync` undoes. Edit the copy in `.chaps/` instead, and `sync` will
follow it, and say that the checksum no longer matches. When the copy is
missing, `sync` leaves `compose.yml` alone rather than guessing.

## `chaps sync`

```sh
chaps sync
```

```text
unchanged     compose.yml
unchanged     compose.chaps.yml
unchanged     compose.chapkit-ewars-model.yml
unchanged     compose.marketplace.yml
in sync: 0 written, 4 unchanged, 0 removed
```

`chaps up` runs `sync` before `docker compose up`, so the intent and the
artifacts never drift in normal use. Rendering is deterministic and every file
is compared before it is written, so a second sync reports everything as
unchanged.

- `chaps sync --check` reports drift without writing, for CI or a pre-commit
  hook, and exits non-zero when anything would change.
- A plain `chaps sync` re-creates a deleted overlay or picks up a hand edit of
  `models.yaml`.
- Sync only ever removes overlays it wrote itself; `project.yaml` keeps the
  list. A hand-written `compose.custom.yml` next to them is left alone. Add it
  to the umbrella by hand if you want it included.

## Pins

A deployment is reproducible because everything it runs is pinned.

- **Models** are pinned to an image tag, which for marketplace images is a
  `sha-<short commit>` build:
  `image: <repo>:${<ID>_IMAGE_TAG:-sha-fa880a1}`. A model that follows a
  channel records both the channel and the version the channel resolved to; one
  enabled with `--version` records the version alone and never moves.
- **chap-core** is pinned to a release tag. `init` resolves the default
  `--chap-tag latest` to the release it points at today, `v2.3.1` say, and
  writes that into `.env` and `.chaps/project.yaml`, so a deployment is
  reproducible rather than following a tag that changes under it. `--chap-tag
  master` or `dev` keeps a moving tag on purpose.

- **Components** other than chap-core follow a moving tag, because neither OCS
  nor the object store publishes a release feed: `${OCS_IMAGE_TAG:-main}` and
  `${S3_IMAGE_TAG:-latest}`. Set either variable in `.env` to pin a build of
  your own; `chaps update` says which of the two is in force. See
  [Components](./components.md).

Model pins only move in two ways: `chaps models enable ID` (with `--channel`
or `--version`) and `chaps update`. Nothing else, `up`, `expose` and `unexpose`
included, changes the version a model runs. See [Updating](./updating.md).

## The `.env` contract

`.env` is the one file that is written once and then left alone. `init` writes
it when the directory has none, and never rewrites it afterwards: a second
`init` keeps the file it finds, `--force` included, `sync` only appends missing
pin comments and `update` only moves the pin comments of models it changed.

That is deliberate. The file holds the database password the PostgreSQL volume
was created with, plus the API token and the registration key, and a rotated
password leaves chap-core unable to authenticate against its own data.

Those two secrets are the one exception to "written once", and a narrow one.
`chaps init --api-token` fills them in as it renders the file;
`chaps auth enable`, `disable` and `rotate` rewrite exactly those two lines
afterwards, leaving every other line, every comment and every blank line byte
for byte as it found them:

```text
CHAP_API_TOKEN=d1f0...                 the whole API, as `Authorization: Bearer`
SERVICEKIT_REGISTRATION_KEY=9a3c...    what each model sends when it registers
```

Commented out, as the generated file ships them, means no authentication at all.
The values live only here: `.chaps/project.yaml` records two booleans under
`auth` and never a secret, so the state directory stays safe to commit and to
paste into a bug report. See [Authentication](./auth.md).

Enabling a component appends its own section the same way `sync` appends a
model's pin: a commented `OCS_IMAGE_TAG`, and for the object store a generated
`S3_ACCESS_KEY` and `S3_SECRET_KEY`. Those two are written once and never
rewritten, for the same reason the database password is: the volume was created
with them.

`init --fresh-env` is the explicit override: it renders a new `.env` with a new
password, so an existing volume has to be dropped with

```sh
chaps docker run -- down -v
```

(or the role changed with `ALTER USER`) before CHAP will start again.
`init --no-env` writes no `.env` at all.

Compose reads `.env` automatically, so `${CHAP_IMAGE_TAG:-latest}` and the
per-model `${<ID>_IMAGE_TAG:-sha-xxxxxxx}` pins can be overridden there without
regenerating anything. Because Compose reads `.env` last, a line in it wins
over what `.chaps/` records; `CHAP_API_PORT` is the case that matters most, and
[Ports](./ports.md) covers it.

## Context-sensitive help

`chaps --help` adapts to where you are. Outside a deployment directory it lists
only the commands that can work there, and says that the rest appear inside
one:

```text
Commands:
  init      Create a deployment directory: compose files, .env and .chaps/
  models    Browse and manage marketplace models
  registry  Inspect and refresh the marketplace registry
  help      Print this message or the help of the given subcommand(s)

...

Inside a directory created by `chaps init`, more commands appear: up, down, logs, status, sync, update, ui, components, docker, backup, auth.
```

The hidden commands still run if you type them; they just tell you there is no
project. `chaps models` is split the same way: `list`, `search` and `info`
browse the marketplace and work anywhere, while `enable`, `disable`, `expose`
and `unexpose` change a project's model set and are hidden outside one. The
[command reference](./reference.md) lists all of them, because it is rendered
from the static command tree rather than from where you happen to be.

## Project discovery

Every command that operates on a project finds it the way git finds `.git`:
from the current directory, or from `-C DIR`, upwards to the nearest `.chaps/`.
So `chaps up` works from any subdirectory of the deployment, and `chaps -C
/srv/mychap status` works from anywhere at all.

The wrappers then always run

```sh
docker compose -f <dir>/compose.yml -f <dir>/compose.chaps.yml -f <dir>/compose.marketplace.yml ...
```

with one `-f` per enabled component inserted before the umbrella.

from the project directory, so they behave the same whatever your shell's
working directory is, and they exit with Compose's own exit code.
