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
moves the host port it publishes, and nothing else is touched.

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
volume has just been removed, so `chaps docker run -- down -v` no longer
reaches it. `--purge` removes it with the component, after the containers, and
reports `removed volume <name>` or `volume <name> not found`; `--json` carries
`purged` and `kept_volumes`. `--purge` works on a component that is already
off, so a volume that was forgotten can still be removed by name.

`chaps components disable chap-core --purge` is refused: chap-core's volumes -
the database and its own data - are declared by upstream's compose file, which
this CLI renders but does not author, so there is no one volume `--purge` could
mean. `chaps docker run -- down -v` removes every volume of the deployment, and
is the honest way to ask for that.

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
```

`.chaps/components.yaml` is intent, like `project.yaml` and `models.yaml`:

```yaml
chap-core:
  enabled: true
ocs:
  enabled: true
  port: 9000
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

OCS's own documentation describes every other field, including the scheduler
and the workflow triggers: see the
[instance guide](https://dhis2.github.io/open-climate-service/instance_guide/).

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
  container is up. `--json` carries them under `components`.
- **`chaps doctor`** adds a `components` line (which components are on, and
  whether the OCS config is present and still the example), a port line per
  published component, and an image line per component image.
- **`chaps update`** reports each component's image. Both follow moving tags, so
  there is no pin to move, only a pull: `docker compose pull` takes whatever the
  tag points at today. An active `OCS_IMAGE_TAG` or `S3_IMAGE_TAG` line in
  `.env` is your own pin, and is reported as such. When the pull brings an image
  the machine did not have, the closing line names the component and
  `chaps restart` is what puts it in service.
- **`chaps ui`** is unchanged: it browses models, not components.
