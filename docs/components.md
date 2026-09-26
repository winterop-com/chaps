# Components

A **component** is a piece a deployment is made of: a service, or a small group
of services, that `chaps sync` renders one compose file for. chap-core is a
component like the others and is on unless you turn it off; the rest are opt-in.

| Component | What it is | On by default |
| --- | --- | --- |
| `chap-core` | CHAP itself: chap-core, its worker, Valkey and PostgreSQL, plus the model overlays on top. | yes |
| `ocs` | [Open Climate Service](https://github.com/dhis2/open-climate-service), beside chap-core, reachable inside the deployment at `http://ocs:9000`. | no |
| `s3` | [RustFS](https://github.com/rustfs/rustfs), an S3-compatible object store for OCS to keep its objects in. | no |

```sh
chaps components list
```

```text
COMPONENT  STATE    REACH                  WHAT IT IS
chap-core  enabled  http://localhost:8000  CHAP itself: chap-core, its worker, Valkey and PostgreSQL
ocs        enabled  http://localhost:9000  Open Climate Service: climate data, reachable at http://ocs:9000
s3         off      -                      RustFS, an S3-compatible object store OCS will keep objects in
```

## Turning one on and off

At creation time:

```sh
chaps init mychap --with ocs          # chap-core and OCS
chaps init mychap --with ocs,s3       # both, plus the object store
chaps init mychap --with ocs,s3 --ocs-port 9010 --s3-port 9002
chaps init mychap --with ocs --ocs-port none --ocs-read-only
```

`--ocs-port` and `--s3-port` take a port number or `none`, and
`--ocs-read-only` starts the instance refusing every write over HTTP. Each one
needs its component: a setting for something `--with` did not ask for is
refused rather than quietly dropped. There is no `--ocs-read-write` to match,
so a `--force` over a directory whose `ocs/climate-service.yaml` already says
`read_only: true` has no init-side way back - `chaps components enable ocs
--read-write` is the route, and the file is the operator's either way. See
[Read-only instances](#read-only-instances).

Afterwards, in the deployment directory:

```sh
chaps components enable ocs           # publishes it on 9000
chaps components enable ocs --port 9010
chaps components enable ocs --port none  # no host port; see below
chaps components enable s3            # internal only
chaps components disable ocs
chaps components disable ocs --purge  # and its data
```

Both commands edit `.chaps/components.yaml` and then run `chaps sync`, so the
compose files on disk always match. `chaps up` applies them. `disable` also
stops and removes the containers of the component it is taking away, while
compose still has the definition to name them by, and says so in a `note:`
line - so the host port it published is free straight away.

Enabling a component that is already on is how its settings change: `--port`
moves the host port it publishes, `--port none` takes it away, and nothing else
is touched. `--port none` reads the same on both components that take one, so
there is one flag to remember rather than a verb per component.

### What `enable` tells you

Every run says what it wrote and what to do next, and adds a `note:` line per
thing that is worth knowing but is nobody's fault. Turning OCS on for the first
time is the noisiest of them:

```text
enabled ocs on http://localhost:9000
written  compose.ocs.yml
written  .env
note: port 9000 is already in use on this machine (needed by ocs); free it, or run `chaps components enable ocs --port <free>`
note: wrote ocs/climate-service.yaml; it is yours to edit, and chaps never rewrites it
note: OCS will soon need an S3-compatible object store; `chaps components enable s3` adds one, and the OCS service then gets the S3_* variables it will read
note: the OCS data source variables are now in `.env`, commented out: ERA5-Land needs one or both of ECMWF_DATASTORES_* and EDH_API_KEY, per dataset; WorldPop and CHIRPS3 need none. `chaps auth show` reports which are set
run `chaps up` to apply
```

The port line is the same probe `chaps init` makes, and a warning for the same
reason: nothing is being started here, the listener is often something you are
about to stop, and `chaps up` is where a taken port becomes a refusal. A port
one of this deployment's own running services already publishes is not a
conflict. See [Component ports](./ports.md#component-ports).

The data source line appears on the run that appends the credential block to
`.env`, because until then the only sign of it was `written .env` and which
dataset needs which key is the one thing the variable names do not say. See
[Data source credentials](#data-source-credentials).

The other two are the soft dependency between OCS and the store, in both
directions. Enabling `s3` on a deployment with no OCS says so, because nothing
else in a chaps deployment writes to it:

```text
note: the object store is for OCS to keep its objects in, and this deployment has no OCS; `chaps components enable ocs` adds one, and nothing else here writes to the store
```

and disabling `s3` while OCS is on says what the next sync takes off the OCS
service, which is a change to a service you did not name:

```text
note: the OCS service loses its S3_* variables on this sync; OCS does not read them yet, and `chaps components enable s3` puts them back
```

Neither is a refusal. A store with nothing to put in it and an OCS with no
store are both states an operator may well be passing through on purpose.

### The data volume

`disable` keeps the component's data, exactly as it keeps a disabled model's,
and names the volume it kept:

```text
disabled ocs
removed compose.ocs.yml
note: kept volume mychap-1ab2c3_ocs_data; remove it with `chaps components disable ocs --purge` or `docker volume rm mychap-1ab2c3_ocs_data`
note: the ocs/ directory is left alone; it is yours
run `chaps up` to apply
```

`ocs` keeps `ocs_data` and `s3` keeps `s3_data`, each prefixed with the compose
project name. Naming it is the whole point: the compose file that declared the
volume has just been removed, so `chaps down --volumes` no longer reaches it. `--purge` removes it with the component, after the containers, and
reports `removed volume <name>` or `volume <name> not found`; `--json` carries
`purged` and `kept_volumes`. `--purge` works on a component that is already
off, so a volume that was forgotten can still be removed by name.

`chaps components disable chap-core --purge` is refused: chap-core's volumes -
the database and its own data - are declared by upstream's compose file, which
this CLI renders but does not author, so there is no one volume `--purge` could
mean. `chaps down --volumes` removes every volume of the deployment, and is the
honest way to ask for that.

`chaps doctor` reports a component volume whose component is off as a leftover;
see [Doctor](./doctor.md).

### Re-initialising with `--force`

`chaps init --force` over an existing deployment sets the component set from
the flags it is given, exactly as `--models` sets the model set: `--with` and
`--without` are the whole answer, and a component the directory had but the new
run does not ask for is going. It is reset-from-flags, not a merge, so the way
to keep a component across a `--force` is to name it again.

Each one that goes gets its own warning before anything is written:

```text
warning: this directory had the ocs component and this run does not ask for it, so compose.ocs.yml is removed and its data volume is left behind; re-run with `--with ocs` to keep it
warning: this directory had the s3 component and this run does not ask for it, so compose.s3.yml is removed and its data volume is left behind; re-run with `--with s3` to keep it
```

So `chaps init . --force --with ocs` keeps OCS and drops the store, and
`chaps init . --force --with ocs,s3` keeps both. A component named in
`--without` is not warned about: you said so.

What a dropped component loses is its compose file and its containers, which
are stopped and removed while the compose file that names them is still on disk
- a service whose definition has just been deleted cannot be stopped by name
afterwards, and one left running would hold its host port against a deployment
that no longer asks for it. What it keeps is its data volume, on the same terms
[`disable` keeps one](#the-data-volume): nothing declares it any more, so
`chaps down --volumes` no longer reaches it and `docker volume rm <name>` or
`chaps components disable <name> --purge` is what removes it. The `ocs/`
directory is yours and is never touched.

`.env` is kept as it always is, so the database password, the API token and any
OCS credentials survive a `--force` whatever happens to the components.

## The files

```text
mychap/
  .chaps/
    components.yaml      intent: which components are on, and their settings
  compose.ocs.yml        artifact: the ocs service and its data volume
  compose.s3.yml         artifact: the object store, its health check and the
                         one-shot that creates the bucket
  ocs/
    climate-service.yaml the OCS instance configuration - yours, written once
    plugins/             optional dataset plugins - yours, mounted if present
```

`.chaps/components.yaml` is intent, like `project.yaml` and `models.yaml`:

```yaml
chap-core:
  enabled: true
ocs:
  enabled: true
  port: 9000
  base_url: null
  read_only: false
  image_tag: main
s3:
  enabled: true
  port: null
```

Every field has a default, so a deployment created before components existed
loads as "chap-core on, nothing else" - which is exactly what it was. There is
nothing to migrate; the file simply appears the next time `chaps sync` runs.

The component compose files sit in the `-f` list between `compose.chaps.yml`
and `compose.marketplace.yml`:

```sh
docker compose -f compose.yml -f compose.chaps.yml -f compose.ocs.yml \
  -f compose.s3.yml -f compose.marketplace.yml up -d
```

## OCS

The `ocs` component runs `ghcr.io/dhis2/open-climate-service`. OCS publishes no
release tags yet, so the pin follows the moving `main` tag; `OCS_IMAGE_TAG` in
`.env` pins a build of your own, and `chaps update` reports which of the two is
in force. The image is multi-arch, so no `platform:` pin is needed and an arm64
host runs it natively - unlike chap-core and the model images, which are amd64
only. It ships its own `HEALTHCHECK`, so the compose file declares none.

OCS serves a web interface as well as an API, so it is published on a host port
by default (9000). chap-core reaches it inside the deployment at
`http://ocs:9000`, on the compose default network, which is what makes OCS
usable as a data source for CHAP: OCS's own example configuration carries a
commented-out `chirps-to-chap` workflow trigger for exactly that, and the file
scaffolded here leaves it out, so adding it is yours.

There is no `depends_on` between OCS and chap-core. They are independent
services and either one is useful without the other.

### `ocs/climate-service.yaml`

Each OCS instance is configured for one country or region. `chaps` scaffolds
that file when the component is first enabled and **never rewrites it**: it is
yours from that moment on. It is mounted read-only at
`/app/climate-service.yaml`, which is where `CLIMATE_SERVICE_CONFIG` points.

Without any flags it holds OCS's own Sierra Leone example values and a note
saying so. `chaps doctor` warns while that note is there, so an instance is
never deployed as the wrong country by accident; edit the file and delete the
note.

The three flags fill it in directly, on `init` and on `components enable` alike:

```sh
chaps init mychap --with ocs \
  --ocs-name Malawi --ocs-country MWI --ocs-bbox 32.6,-17.2,35.9,-9.3
```

```yaml
id: malawi-climate-service
name: Malawi Climate Service

extent:
  name: Malawi
  bbox: [32.6, -17.2, 35.9, -9.3]   # xmin, ymin, xmax, ymax
  country_code: MWI

data_dir: /app/data
```

A western or southern extent starts with a minus sign, and `--ocs-bbox` takes
it without the `=`:

```sh
chaps components enable ocs --ocs-bbox -13.5,6.9,-10.1,10.0
```

OCS's own documentation describes every other field, including the scheduler
and the workflow triggers: see the
[instance guide](https://dhis2.github.io/open-climate-service/instance_guide/).

### Data source credentials

Two of the dataset families OCS can ingest need an account. WorldPop and CHIRPS3
need nothing at all, so a deployment that only wants those needs none of this.

| Source | Variables | Datasets |
| --- | --- | --- |
| [Copernicus Climate Data Store](https://cds.climate.copernicus.eu/) | `ECMWF_DATASTORES_URL`, `ECMWF_DATASTORES_KEY` | ERA5-Land monthly, and the recent tail of the daily ones |
| [Earth Data Hub](https://earthdatahub.destine.eu/) | `EDH_API_KEY` | ERA5-Land daily, and the 1991-2020 day-of-year normals |
| [Copernicus Data Space Ecosystem](https://dataspace.copernicus.eu/) | `CDSE_S3_ACCESS_KEY`, `CDSE_S3_SECRET_KEY` | the CLMS GPP dataset plugin |
| WorldPop, CHIRPS3 | none | population, precipitation |

The two ERA5-Land rows are not alternatives. The monthly datasets come from the
Climate Data Store alone and the day-of-year normals from the Earth Data Hub
alone, but every daily dataset reads both, choosing per period: the Earth Data
Hub store for the history, and the Climate Data Store for the roughly four weeks
the hub has not published yet. So a daily ingestion that reaches into the present
needs both accounts, while one that stops short of the hub's coverage needs only
the hub.

`chaps` writes the five as commented placeholders into `.env` when the `ocs`
component is enabled, and appends the section to a `.env` that does not have it
yet - it never rewrites the file, exactly as with the image pins:

```ini
# OCS data sources (optional): ERA5-Land needs one or both of ECMWF_DATASTORES_*
# and EDH_API_KEY, per dataset; WorldPop and CHIRPS3 need none.
# ECMWF_DATASTORES_URL=https://cds.climate.copernicus.eu/api
# ECMWF_DATASTORES_KEY=
# EDH_API_KEY=
# CDSE_S3_ACCESS_KEY=
# CDSE_S3_SECRET_KEY=
```

Uncomment the ones you have, fill them in, and run `chaps up`: a container
reads `.env` when compose creates it, so a running OCS does not pick up a
credential that was added after it started.

A deployment whose `.env` already had the section keeps the heading it was
written with, so one created before this wording still carries it as a single
line saying `ERA5-Land needs one of these` - the old summary, and the wrong one:
it made the two ERA5-Land rows look like alternatives. What says the section is
already there is the five variables, not the heading, so an older `.env` is left
exactly as it is rather than appended to a second time. Nothing below the heading
changed, and `chaps` never rewrites `.env`: the stale line is a comment, so
correct it by hand or leave it.

`compose.ocs.yml` passes all five unconditionally, as `${VAR:-}`. That is safe
because each of them is read as `os.getenv(...) or <a credentials file>`, so a
variable that arrives empty is one nothing has: a deployment that sets none of
them behaves exactly as one whose compose file never mentioned them. Which code
does the reading, and which file it falls back to, differs per source:

| Variables | Read by | Falls back to |
| --- | --- | --- |
| `ECMWF_DATASTORES_URL`, `ECMWF_DATASTORES_KEY` | the `ecmwf-datastores-client` library OCS calls | `~/.ecmwfdatastoresrc` |
| `EDH_API_KEY` | OCS itself | `~/.netrc`, for `api.earthdatahub.destine.eu` |
| `CDSE_S3_ACCESS_KEY`, `CDSE_S3_SECRET_KEY` | the CLMS GPP dataset plugin, which is yours rather than part of OCS | the `[cdse]` profile in `~/.aws/credentials` |

Nothing in OCS reads the `CDSE_S3_*` pair - the plugin that does is one you
bring. That plugin reads two more variables of its own, `CDSE_S3_PROFILE` and
`CDSE_S3_ENDPOINT`; `chaps` writes neither into `.env`, and `compose.ocs.yml`
names only the five above, so neither reaches the container. A plugin that needs
a profile or an endpoint other than its own default has to carry it itself.

Without either the variables or the file, an ERA5-Land ingestion that reaches the
Climate Data Store fails with
`No such file or directory: '/home/ocs/.ecmwfdatastoresrc'` - that is the
container looking for the file form of the same credential, in the home directory
of the image's `ocs` user.

`chaps auth show` reports which of them `.env` sets, masked:

```text
OCS data sources
  ECMWF_DATASTORES_URL  set https:/...
  ECMWF_DATASTORES_KEY  set 012345...
  EDH_API_KEY           unset
  CDSE_S3_ACCESS_KEY    unset
  CDSE_S3_SECRET_KEY    unset
```

Masked whatever `--reveal` asks for: these are accounts with Copernicus and
Earth Data Hub rather than this deployment's own secret, so there is nothing to
paste into a client here - only the question of whether a dataset will ingest.
`chaps doctor` says `ERA5-Land: credentials unset (WorldPop and CHIRPS3 work
without them)` on its `components` line, as information. It is not a warning: a
deployment with no credentials is a working deployment.

### Behind a reverse proxy

A public instance normally sits behind a proxy that terminates TLS, and then two
things change. The host port should not be published at all, so the proxy is the
only way in, and OCS has to be told its public origin - otherwise it builds its
STAC and openEO links from the request it received, which names the internal
address:

```sh
chaps components enable ocs --port none --base-url https://ocs.example.org
chaps init mychap --with ocs --ocs-base-url https://ocs.example.org   # at creation
```

The overlay then `expose`s the container port and publishes nothing:

```yaml
services:
  ocs:
    expose:
      - "9000"
    environment:
      CLIMATE_SERVICE_BASE_URL: ${CLIMATE_SERVICE_BASE_URL:-https://ocs.example.org}
```

It is rendered as a `${VAR:-default}`, like the image pin, so the recorded value
is the default and a `CLIMATE_SERVICE_BASE_URL` line in `.env` still moves it
without a sync. The URL has to be absolute, with a scheme: OCS refuses a
schemeless value, logs a warning and goes back to composing links from the
request, which is the setting quietly not working.

`chaps status` then shows where the instance actually is:

```text
ocs  up  internal (proxy: https://ocs.example.org)
```

`--port 9010` publishes it again, and the address is then the address: it is
where this machine reaches it, whatever the proxy is called. `--base-url ""`
clears the setting.

Point the proxy at the container on the compose network. Give it an
**allowlist** of the open routes rather than a denylist, so a route added in a
later OCS release is not permitted by default, and add request timeouts,
body-size limits and per-IP rate limiting: read-only protects integrity, not
availability, and `POST /result` is still an unbounded compute endpoint.

### Read-only instances

`read_only: true` in `ocs/climate-service.yaml` makes OCS refuse every write
over HTTP: ingestion and sync under `/manage` - `POST /manage/ingest` and
`POST /manage/sync`, which are what the data source and dataset pages post their
forms to rather than a console of their own - plus stored process graphs, batch
jobs and operator export delivery. `/manage`, `/jobs` and `/exports` are closed
as whole trees, `GET` included, so nothing added under one of them in a later
release is open by default. The catalogue, the data endpoints, the map viewer and
synchronous `POST /result` stay open, and so do the pages themselves, which
simply leave their forms out. This is how a public instance runs.

```sh
chaps components enable ocs --read-only
chaps restart --all ocs
chaps components enable ocs --read-write   # and back again
```

Both flags edit the one `read_only` key in `ocs/climate-service.yaml` and leave
every other byte of it alone - your comments, your dataset list, your scheduler
- adding the key at the end when the file does not have it. The value is also
recorded in `components.yaml`, and `chaps status` prints `read-only` next to the
OCS line:

```text
ocs  up  http://localhost:9000   read-only
```

OCS reads the file and not the record, and it reads it once at startup, so the
instance has to be recreated. **`chaps restart --all ocs`, not a plain
`chaps restart`**: the plain form is `docker compose up -d`, which recreates
only what no longer matches the compose files, and the instance config is a bind
mount - the new text is already visible inside the container, and there is
nothing for compose to compare. `--all` with the service named forces that one
service to be recreated and leaves chap-core and the models alone. The note
`chaps components enable` prints says as much.

Check it after every deploy. `read_only` is an ordinary config key, so a build
of open-climate-service that predates read-only mode ignores it silently and
serves a fully writable instance:

```sh
curl -s http://localhost:9000/info | grep read_only   # must be true
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://localhost:9000/ingestions -d '{}'   # 403
```

Read-only applies to HTTP, so ingestion becomes an operator task. Ingest through
a throwaway container, which reads the credentials from your shell and never
gives them to the running service:

```sh
set -a; . ./.env; set +a
chaps docker run -- run --rm --no-deps \
  -e ECMWF_DATASTORES_URL -e ECMWF_DATASTORES_KEY \
  ocs python -c "
from open_climate_service.ingestions.processes import execute_ingestion
execute_ingestion(dataset_id='era5land_temperature_monthly', start='2020-01', end='2024-12')
"
```

Ingest a whole range in one call. Extending an existing dataset with a second,
adjacent ingestion writes the new periods and then fails the coverage check,
leaving the registry pointing at the old range.

On a host that runs read-only, **keep the credentials out of `.env`**. Read-only
makes them unusable there, so their only effect is to sit in the process
environment of a service that cannot ingest; the throwaway container gets them
from wherever you run it from instead.

### Dataset plugins

OCS loads extra datasets from a plugin directory. Put them in `ocs/plugins/`:

```text
mychap/
  ocs/
    climate-service.yaml
    plugins/
      datasets/
        clms_gpp.py
        clms_gpp.yaml
```

The directory existing is the whole declaration. `chaps sync` then renders the
mount

```yaml
    volumes:
      - ./ocs/climate-service.yaml:/app/climate-service.yaml:ro
      - ./ocs/plugins:/app/plugins:ro
```

and adds `plugins_dir: /app/plugins` to `ocs/climate-service.yaml` if the file
does not already name one - a mount OCS is not pointed at is a mount it never
looks in. A `plugins_dir` you set yourself is never moved. `chaps doctor`
reports the file count on its `components` line, as information.

The mount is conditional because compose refuses to start a service whose bind
source does not exist, so a project without plugins must not carry the line.
Nothing creates the directory for you, and `chaps` never writes into it: the
plugins are yours, and a plugin with dependencies the image does not have needs
an image of its own.

### Adding a plugin to a running deployment

Run `chaps up`, not `chaps restart`. `chaps up` is the only wrapper that
re-renders the compose files before it calls Docker; `chaps restart`,
`chaps logs`, `chaps docker ps` and the rest run against the files exactly as
they are on disk. So on a deployment where `ocs` was enabled before
`ocs/plugins/` existed, `compose.ocs.yml` carries no plugin mount, and a
`chaps restart` finds a container that already matches that file: nothing
happens at all - no error, no mount, no plugin.

```sh
mkdir -p ocs/plugins/datasets
cp clms_gpp.py clms_gpp.yaml ocs/plugins/datasets/
chaps up      # syncs first, so the mount is rendered and then ocs is recreated
```

`chaps sync` followed by `chaps restart` is the same thing in two steps: once
the mount is in the file, the running container no longer matches it and compose
recreates it.

**This is the one place where `chaps restart --all ocs` is the wrong reach.**
That form belongs to [`ocs/climate-service.yaml`](#read-only-instances), and for
the opposite reason: that file is a bind mount, so an edit to it is already
visible inside the container and there is nothing to re-render - the only
problem is that OCS read the old text at startup, which a forced recreate of
that one service fixes. A directory that did not exist at render time is the
other case entirely: the compose file itself is out of date, and no amount of
recreating mounts a directory it does not mention.

`chaps doctor` is the safety net, because its `compose files` line is
`chaps sync --check`:

```text
warn  compose files   out of date with .chaps/ (2 to write, 5 unchanged, 0 to remove)
      run `chaps sync`, or `chaps up`, which syncs first
```

Two files to write, because the mount in `compose.ocs.yml` and the `plugins_dir`
key in `ocs/climate-service.yaml` are rendered by the same sync.

## The object store

OCS will soon require an S3-compatible object store. It does not read any `S3_*`
variable yet, so `chaps` does not force one on a deployment: enabling `ocs`
prints a one-line note that the store is coming and that
`chaps components enable s3` adds it.

When `s3` is on, the OCS service gets four forward-looking variables -
`S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET_KEY` and `S3_BUCKET` - with a comment
in the file saying that OCS does not read them yet. The exact contract is not
published, so those names may still change.

The store itself is RustFS. `chaps` generates `S3_ACCESS_KEY` and
`S3_SECRET_KEY` (32 hex characters each) into `.env` once, and never rewrites
them: the data volume is created with them, and changing them later locks the
store's own contents away. The compose file substitutes them into the image's
own `RUSTFS_ACCESS_KEY` and `RUSTFS_SECRET_KEY`, so the credentials live in
`.env` and nowhere else.

A one-shot `s3-init` service creates the `ocs` bucket once the store is healthy.
It runs the same RustFS image and signs the request with `curl --aws-sigv4`, so
nothing extra is pulled: the image is Alpine-based and ships both curl and wget,
which is what the `s3` health check uses on `GET /health` as well. Creating a
bucket that already exists answers 200, so every later `chaps up` runs the
one-shot again and changes nothing.

The store publishes no host port: OCS reaches it at `http://s3:9000` on the
compose default network. `chaps components enable s3 --port 9002` publishes one
for an S3 client of your own.

## Standalone OCS

`--without chap-core` is the same mechanism with CHAP left out:

```sh
chaps init climate --with ocs,s3 --without chap-core
```

Neither `compose.yml` nor `compose.chaps.yml` is rendered, and the `-f` list is
just the component files and the (empty) marketplace umbrella. `chaps status`
leaves out the chap-core line and counts components rather than models in its
closing line; a deployment with nothing running is told that nothing in it is
running, never that CHAP is not, because there is no CHAP here to be running.
`chaps doctor` judges the deployment by its components alone.

Models need chap-core: a model service registers with chap-core and is reached
through it. So `--without chap-core` together with a `--models` list is refused,
and `chaps components disable chap-core` is refused while any model is enabled.
The message names the models in the way. The default model set is a default and
not something you asked for, so `--without chap-core` on its own starts with no
models rather than making you type `--models none` as well; the `init` summary
then says why there are none and names `chaps components enable chap-core`,
rather than naming `chaps models enable`, which such a deployment refuses.

## What the other commands say

- **`chaps up`** checks the components' host ports in its preflight, alongside
  the API port, and names the command that moves each one.
- **`chaps status`** prints one line per enabled component under the chap-core
  line, each judged by its container first, exactly as `chaps doctor` does: a
  component with no container reads `not running`, and only OCS is asked
  anything beyond that - its `/health` endpoint, and only while its container is
  up. That gate is what keeps a stopped OCS from being reported `up` because
  another process answered on its host port: OCS defaults to 9000, so two
  deployments on one machine collide there, which is the clash the `up`
  preflight and `chaps doctor` already warn about. An OCS instance with no host
  port cannot be asked from out here at all, so its container is the whole
  answer, and the line says `internal (proxy: <base_url>)` and `read-only`
  where those apply. An OCS
  instance that answered also reports what it holds, `3 datasets` and
  `212.0 MB data`. The rows come from `.chaps/components.yaml` rather than from
  docker, so they are printed with nothing running too, each reading
  `not running` beside the address it will answer on. `--json` carries the rows
  under `components`, with `read_only`, `health_url`, `datasets` and
  `data_bytes`.
- **`chaps doctor`** adds a `components` line - which components are on, whether
  the OCS config is present and still the example, whether any data source
  credential is set, how many plugin files there are and, while the instance is
  up, what it holds - plus a port line per published component and an image line
  per component image. Those last facts ride on the warning about an unedited
  config as much as on an `ok` line: a deployment that has just enabled `ocs`
  still has the example config, and that is the run in which a missing ERA5-Land
  key is most worth reading.
- **`chaps auth show`** adds an `OCS data sources` block, masked.
- **`chaps update`** reports each component's image. Both follow moving tags, so
  there is no pin to move, only a pull: `docker compose pull` takes whatever the
  tag points at today. An active `OCS_IMAGE_TAG` or `S3_IMAGE_TAG` line in
  `.env` is your own pin, and is reported as such. When the pull brings an image
  the machine did not have, the closing line names the component and
  `chaps restart` is what puts it in service.
- **`chaps ui`** has a components page beside the models one: `Tab` moves
  between them, the rows are the ones `chaps components list` prints, and
  `space`, `p` and `P` do there what they do for a model. One `s` saves both
  pages. The two OCS settings it deliberately does not edit - `--base-url` and
  `--read-only` - are named in the component's `i` overlay rather than hidden,
  because they write to `ocs/climate-service.yaml`, which is yours. See
  [The components page](./models.md#the-components-page).
