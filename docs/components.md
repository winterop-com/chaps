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
```

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
usable as a data source for CHAP: OCS's own configuration has a
`chirps-to-chap` workflow trigger for exactly that.

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
| [Copernicus Climate Data Store](https://cds.climate.copernicus.eu/) | `ECMWF_DATASTORES_URL`, `ECMWF_DATASTORES_KEY` | ERA5-Land, monthly |
| [Earth Data Hub](https://earthdatahub.destine.eu/) | `EDH_API_KEY` | ERA5-Land, hourly and daily |
| [Copernicus Data Space Ecosystem](https://dataspace.copernicus.eu/) | `CDSE_S3_ACCESS_KEY`, `CDSE_S3_SECRET_KEY` | the CLMS GPP dataset plugin |
| WorldPop, CHIRPS3 | none | population, precipitation |

`chaps` writes the five as commented placeholders into `.env` when the `ocs`
component is enabled, and appends the section to a `.env` that does not have it
yet - it never rewrites the file, exactly as with the image pins:

```ini
# OCS data sources (optional): ERA5-Land needs one of these; WorldPop and CHIRPS3 need none.
# ECMWF_DATASTORES_URL=https://cds.climate.copernicus.eu/api
# ECMWF_DATASTORES_KEY=
# EDH_API_KEY=
# CDSE_S3_ACCESS_KEY=
# CDSE_S3_SECRET_KEY=
```

Uncomment the ones you have, fill them in, and run `chaps up`: a container
reads `.env` when compose creates it, so a running OCS does not pick up a
credential that was added after it started.

`compose.ocs.yml` passes all five unconditionally, as `${VAR:-}`. That is safe
because OCS reads each of them as `os.getenv(...) or <the credentials file>`, so
a variable that arrives empty is one it does not have: a deployment that sets
none of them behaves exactly as one whose compose file never mentioned them.
Without either the variables or the file, an ERA5-Land ingestion fails with
`No such file or directory: '/home/ocs/.ecmwfdatastoresrc'` - that is the
container looking for the file form of the same credential.

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
over HTTP: ingestion and sync, the `/manage` console, stored process graphs and
batch jobs. The catalogue, the data endpoints, the map viewer and synchronous
`POST /result` stay open. This is how a public instance runs.

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
nothing extra is pulled, and creating a bucket that already exists answers 200 -
re-running it changes nothing.

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
leaves out the chap-core line, and `chaps doctor` judges the deployment by its
components alone.

Models need chap-core: a model service registers with chap-core and is reached
through it. So `--without chap-core` together with `--models` is refused, and
`chaps components disable chap-core` is refused while any model is enabled. The
message names the models in the way.

## What the other commands say

- **`chaps up`** checks the components' host ports in its preflight, alongside
  the API port, and names the command that moves each one.
- **`chaps status`** prints one line per enabled component under the chap-core
  line: OCS from its `/health` endpoint, the object store from whether its
  container is up. An OCS instance with no host port cannot be asked from out
  here at all, so it is judged by its container too, and the line says
  `internal (proxy: <base_url>)` and `read-only` where those apply. `--json`
  carries them under `components`, with `read_only` and `health_url`.
- **`chaps doctor`** adds a `components` line (which components are on, whether
  the OCS config is present and still the example, whether any data source
  credential is set, and how many plugin files there are), a port line per
  published component, and an image line per component image.
- **`chaps auth show`** adds an `OCS data sources` block, masked.
- **`chaps update`** reports each component's image. Both follow moving tags, so
  there is no pin to move, only a pull: `docker compose pull` takes whatever the
  tag points at today. An active `OCS_IMAGE_TAG` or `S3_IMAGE_TAG` line in
  `.env` is your own pin, and is reported as such. When the pull brings an image
  the machine did not have, the closing line names the component and
  `chaps restart` is what puts it in service.
- **`chaps ui`** is unchanged: it browses models, not components.
