# Components

A **component** is a piece a deployment is made of: a service, or a small group
of services, that `varde sync` renders one compose file for. chap-core is a
component like the others and is on unless you turn it off; the rest are opt-in.

| Component | What it is | On by default |
| --- | --- | --- |
| `chap-core` | Chap itself: chap-core, its worker, Valkey and PostgreSQL, plus the model overlays on top. | yes |
| `ocs` | [Open Climate Service](https://github.com/dhis2/open-climate-service), beside chap-core, reachable inside the deployment at `http://ocs:9000`. | no |
| `s3` | [RustFS](https://github.com/rustfs/rustfs), an S3-compatible object store for OCS to keep its objects in. | no |
| `dhis2` | A demo or development [DHIS2](https://dhis2.org) and its own PostgreSQL, for a deployment that wants one beside Chap. See [DHIS2](./dhis2.md). | no |

```sh
varde components list
```

```text
COMPONENT  STATE    REACH                  WHAT IT IS
chap-core  enabled  http://localhost:8700  Chap itself: chap-core, its worker, Valkey and PostgreSQL
ocs        enabled  http://localhost:8790  Open Climate Service: climate data, reachable at http://ocs:9000
s3         off      -                      RustFS, an S3-compatible object store OCS will keep objects in
dhis2      enabled  http://localhost:8780  DHIS2 and its own database, for a deployment that wants one
```

## Turning one on and off

At creation time:

```sh
varde init mychap --with ocs          # chap-core and OCS
varde init mychap --with ocs,s3       # both, plus the object store
varde init mychap --with ocs,s3 --ocs-port 9010 --s3-port 9002
varde init mychap --with ocs --ocs-port none --ocs-read-only
varde init mychap --with dhis2 --dhis2-port 18080 --dhis2-seed none
varde init mychap --with dhis2 --dhis2-tag 2.41   # another DHIS2 version
varde init mychap --with dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master   # the next, unreleased
```

`--ocs-port`, `--s3-port` and `--dhis2-port` take a port number or `none`,
`--ocs-read-only` starts the instance refusing every write over HTTP, and
`--dhis2-seed` says what the DHIS2 database is restored from the first time it is
created. Each one needs its component: a setting for something `--with` did not
ask for is refused rather than quietly dropped. There is no `--ocs-read-write` to
match,
so a `--force` over a directory whose `ocs/climate-service.yaml` already says
`read_only: true` has no init-side way back - `varde components enable ocs
--read-write` is the route, and the file is the operator's either way. See
[Read-only instances](#read-only-instances).

Afterwards, in the deployment directory:

```sh
varde components enable ocs           # publishes it on 8790
varde components enable ocs --port 9010
varde components enable ocs --port none  # no host port; see below
varde components enable s3            # internal only
varde components enable dhis2         # publishes it on 8780
varde components enable dhis2 --tag 2.43   # and picks its version
varde components disable ocs
varde components disable ocs --purge  # and its data
```

Both commands edit `.varde/components.yaml` and then run `varde sync`, so the
compose files on disk always match. `varde up` applies them. `disable` also
stops and removes the containers of the component it is taking away, while
compose still has the definition to name them by, and says so in a line of
its output - so the host port it published is free straight away.

Enabling a component that is already on is how its settings change: `--port`
moves the host port it publishes, `--port none` takes it away, and nothing else
is touched. `--port none` reads the same on every component that takes a port, so
there is one flag to remember rather than a verb per component. `dhis2` has two
settings that are not `--port`: the seed and the image. `--tag` and `--image`
change the image (see
[Changing the DHIS2 version](./dhis2.md#changing-the-dhis2-version)). `--seed SPEC`
changes the seed (`default`, `none`, a URL or a path); it applies only to a
new `dhis2_db` (see [The seed](./dhis2.md#the-seed)).

### What `enable` tells you

Every run says what it did and what to do next. A problem comes as a
`warning:` line on stderr. With `-v`, the files it wrote and the background
come as `hint:` lines (see [Levels](./status.md#levels)). Turning OCS on for
the first time is the noisiest of them:

```text
$ varde -v components enable ocs
enabled ocs on http://localhost:8790
warning: port 8790 is already in use on this machine (needed by ocs); free it, or run `varde components enable ocs --port 8791`
hint: wrote ocs/climate-service.yaml; it is yours to edit, and varde never rewrites it
hint: OCS will soon need an S3-compatible object store; `varde components enable s3` adds one, and the OCS service then gets the S3_* variables it will read
hint: the OCS data source variables are now in `.env`, commented out: ERA5-Land needs one or both of ECMWF_DATASTORES_* and EDH_API_KEY, per dataset; WorldPop and CHIRPS3 need none. `varde auth show` reports which are set
hint: wrote compose.ocs.yml
hint: wrote .env
run `varde up` to apply
```

Without `-v`, the same run prints the first line, the warning and the last
line.

The port line is the same probe `varde init` makes, and a warning for the same
reason: nothing is being started here, the listener is often something you are
about to stop, and `varde up` is where a taken port becomes a refusal. A port
one of this deployment's own running services already publishes is not a
conflict. See [Component ports](./ports.md#component-ports).

The data source line appears on the run that appends the credential block to
`.env`, because until then the only sign of it was `written .env` and which
dataset needs which key is the one thing the variable names do not say. See
[Data source credentials](#data-source-credentials).

The other two are the soft dependency between OCS and the store, in both
directions. Enabling `s3` on a deployment with no OCS says so, because nothing
else in a varde deployment writes to it:

```text
hint: the object store is for OCS to keep its objects in, and this deployment has no OCS; `varde components enable ocs` adds one, and nothing else here writes to the store
```

and disabling `s3` while OCS is on says what the next sync takes off the OCS
service, which is a change to a service you did not name:

```text
hint: the OCS service loses its S3_* variables on this sync; OCS does not read them yet, and `varde components enable s3` puts them back
```

Neither is a refusal. A store with nothing to put in it and an OCS with no
store are both states an operator may well be passing through on purpose.

Turning `dhis2` on says three more things, because none of them is visible
from the compose file:

```text
$ varde -v components enable dhis2
enabled dhis2 on http://localhost:8780
hint: wrote dhis2/dhis.conf; it is yours to edit, and varde never rewrites it
hint: the first `varde up` restores https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz into `dhis2_db`, once, on the database it creates; after that only `varde components disable dhis2 --purge` makes it happen again; the restore empties the data values that analytics cannot read as a number, and `varde logs dhis2-db` lists them
hint: the first `varde up` takes minutes before DHIS2 answers - it migrates its schema on the way up - and `varde logs dhis2` is where that shows
the Modeling App reaches chap-core through a DHIS2 route, and this deployment has none yet; once DHIS2 answers, `varde dhis2 connect` adds it, generates analytics and installs the apps
hint: wrote compose.dhis2.yml
hint: wrote .env
run `varde up` to apply
```

The first is the restore happening **once**, on a data directory PostgreSQL has
just created, which is the half that decides whether changing the seed later does
anything. The second is the first start taking minutes rather than seconds, which
is the half that decides whether the first `varde status` reads as a broken
deployment. The third comes only on a deployment with chap-core: the Modeling App
needs a DHIS2 route to chap-core, and `varde dhis2 connect` adds it (see
[Connecting the Modeling App to Chap](./dhis2.md#connecting-the-modeling-app-to-chap)).
See [DHIS2](./dhis2.md).

### The data volume

`disable` keeps the component's data, exactly as it keeps a disabled model's,
and names the volume it kept:

```text
$ varde -v components disable ocs
disabled ocs
kept volume mychap-1ab2c3_ocs_data; remove it with `varde components disable ocs --purge` or `docker volume rm mychap-1ab2c3_ocs_data`
hint: the ocs/ directory is left alone; it is yours
hint: removed compose.ocs.yml
hint: `varde status` shows what runs now
```

`ocs` keeps `ocs_data`, `s3` keeps `s3_data`, and `dhis2` keeps `dhis2_home`,
`dhis2_db` and `dhis2_dump`, each prefixed with the compose project name. A
component is not limited to one volume, so every one it declares is named on a
line of its own - `dhis2_dump` included, which is a download cache rather than
data but is still a name someone would otherwise have to find by hand. Naming
them is the whole point: the compose file that declared the volume has just been
removed, so `varde down --volumes` no longer reaches it. A volume docker does
not have - the component was never started - is not named, since nothing was
kept. `--purge` removes them
with the component, after the containers, and
reports `removed volume <name>` or `volume <name> not found` per volume; `--json` carries
`purged` and `kept_volumes`. `--purge` works on a component that is already
off, so a volume that was forgotten can still be removed by name.

`varde components disable chap-core --purge` is refused: chap-core's volumes -
the database and its own data - are declared by upstream's compose file, which
this CLI renders but does not author, so there is no one volume `--purge` could
mean. `varde down --volumes` removes every volume of the deployment, and is the
honest way to ask for that.

`varde doctor` reports a component volume whose component is off as a leftover;
see [Doctor](./doctor.md).

### Re-initialising with `--force`

`varde init --force` over an existing deployment sets the component set from
the flags it is given, exactly as `--models` sets the model set: `--with` and
`--without` (or `--only`) are the whole answer, and a component the directory had but the new
run does not ask for is going. It is reset-from-flags, not a merge, so the way
to keep a component across a `--force` is to name it again.

Each one that goes gets its own warning before anything is written:

```text
warning: this directory had the ocs component and this run does not ask for it, so compose.ocs.yml is removed and its data volume is left behind; re-run with `--with ocs` to keep it
warning: this directory had the s3 component and this run does not ask for it, so compose.s3.yml is removed and its data volume is left behind; re-run with `--with s3` to keep it
```

So `varde init . --force --with ocs` keeps OCS and drops the store, and
`varde init . --force --with ocs,s3` keeps both. A component named in
`--without` is not warned about: you said so.

What a dropped component loses is its compose file and its containers, which
are stopped and removed while the compose file that names them is still on disk
- a service whose definition has just been deleted cannot be stopped by name
afterwards, and one left running would hold its host port against a deployment
that no longer asks for it. What it keeps is its data volume, on the same terms
[`disable` keeps one](#the-data-volume): nothing declares it any more, so
`varde down --volumes` no longer reaches it and `docker volume rm <name>` or
`varde components disable <name> --purge` is what removes it. The `ocs/`
directory is yours and is never touched.

`.env` is kept as it always is, so the database password, the API token and any
OCS credentials survive a `--force` whatever happens to the components.

## The files

```text
mychap/
  .varde/
    components.yaml      intent: which components are on, and their settings
  compose.ocs.yml        artifact: the ocs service and its data volume
  compose.s3.yml         artifact: the object store, its health check and the
                         one-shot that creates the bucket
  compose.dhis2.yml      artifact: DHIS2, its own PostgreSQL, two one-shots and
                         three volumes
  ocs/
    climate-service.yaml the OCS instance configuration - yours, written once
    plugins/             optional dataset plugins - yours, mounted if present
  dhis2/
    dhis.conf            the DHIS2 instance configuration - yours, written once,
                         and DHIS2 does not start without it
```

`.varde/components.yaml` is intent, like `project.yaml` and `models.yaml`:

```yaml
chap-core:
  enabled: true
ocs:
  enabled: true
  port: 8790
  base_url: null
  read_only: false
  image_tag: main
s3:
  enabled: true
  port: null
dhis2:
  enabled: true
  port: 8780
  image_tag: '2.42'
  image: dhis2/core
  seed: default
  connected_at: null
```

Every field has a default, so a missing field, or a missing file, loads as
"chap-core on, nothing else".

Two keys in there are records rather than intent, and each is marked as such
where it appears. `ocs.read_only` mirrors what `ocs/climate-service.yaml` says,
because OCS reads that file and not this one; `dhis2.connected_at` notes when
`varde dhis2 connect` last finished, and its only job is to stop `varde up` and
`varde status` asking for a connect that has already happened. Neither is ever
read as evidence - see [`connected_at`, and what it is
not](./dhis2.md#connected_at-and-what-it-is-not), which names the command that
does ask DHIS2.

The component compose files sit in the `-f` list between `compose.varde.yml`
and `compose.marketplace.yml`:

```sh
docker compose -f compose.yml -f compose.varde.yml -f compose.ocs.yml \
  -f compose.s3.yml -f compose.dhis2.yml -f compose.marketplace.yml up -d
```

## Reaching a component from a browser

`varde components list` says where each component answers. `varde open NAME`
takes you there:

```sh
varde open dhis2
varde open ocs
varde open chap-core
varde open              # what is there to open, and at which address
```

```text
opening the DHIS2 user interface at http://localhost:18080
```

Before that line, varde sends one request to that address, with a
three-second limit. Any HTTP answer counts, a 401 from a protected `/docs`
included. With `-v`, a hint gives the result:

```text
hint: dhis2 answered at that address; `varde status` reports the rest of this deployment
```

A container that is up and not serving yet gives the warning `the dhis2
container is running and did not answer yet` instead.

The address is not always the component's origin, because the origin is not
always the useful page:

| Component | What `varde open` opens | Why |
| --- | --- | --- |
| `chap-core` | `/docs` on the API port | The API answers JSON everywhere else; `/docs` is its interactive documentation, and the one page on that port a person reads. |
| `ocs` | the root of its host port | OCS serves a web interface there as well as its API. |
| `dhis2` | the root of its host port | That is the DHIS2 login page, and the whole of its UI hangs off it. |
| `s3` | nothing | The object store speaks the S3 API and serves no web interface; a browser aimed at it gets an XML error document. |

chap-core's address follows `CHAP_API_PORT` and `CHAP_ROOT_PATH` in `.env`, so a
deployment served under a path prefix opens `/master/docs` rather than `/docs`:
the prefix moves every route chap-core serves. See
[How the API port is set](./ports.md#how-the-api-port-is-set) and
[What `.env` holds](./concepts.md#what-env-holds). A deployment with an API token
also gets a line that says that `/docs` is behind it, and that
`varde auth show --reveal` prints the token to paste into the page's Authorize
button; see [Authentication](./auth.md).

With no name it lists every component and what each one opens, which is both the
answer to "what is there" and a reminder of the addresses. A second table lists
each enabled model that publishes a host port:

```text
COMPONENT  OPENS                       WHAT IT IS
chap-core  http://localhost:8700/docs  chap-core's API documentation
ocs        http://localhost:8790       the OCS web interface
s3         -                           an S3 API; no web interface to open
dhis2      -                           not a component of this deployment

MODEL                OPENS                       WHAT IT IS
chapkit_ewars_model  http://localhost:5001/docs  the model's API documentation

3 of them can be opened: run `varde open NAME`
```

### What it refuses, and what it only warns about

Three answers open nothing, and each names what is true instead:

```text
error: ocs is not a component of this deployment, so there is nothing to open; run `varde components enable ocs` to add it
error: ocs publishes no host port, so there is nothing to open from this machine: it is reached at http://ocs:9000 inside the deployment; run `varde components enable ocs --port N` to publish one
error: the object store speaks the S3 API and serves no web interface, so there is nothing a browser can open; run `varde components enable s3 --port N` to publish it for an S3 client of your own, and `varde status` says whether it is running
```

A component with no host port is reached somewhere else rather than nowhere: see
[Component ports](./ports.md#component-ports). The one exception is an OCS
instance with no host port and a recorded `--base-url` - the public origin it
builds its links from - which *is* an address a browser can be pointed at, so
that is what `varde open ocs` opens, with a line saying it is the proxy and not a
port on this machine. See
[Behind a reverse proxy](#behind-a-reverse-proxy).

A component that is enabled but **not running** is a warning rather than a
refusal, and the page is opened anyway:

```text
opening chap-core's API documentation at http://localhost:8700/docs
warning: no chap container is running, so the page will not load yet; run `varde up` to start this deployment
```

The container is what decides that, exactly as it does for `varde status` and
`varde doctor`. Three things follow from asking docker rather than the port:

- A component whose container is up gets one request to its address, with a
  three-second limit. The closing line says `answered at that address` or
  `did not answer yet`. `varde status` is the full check.
- A component with no container gets the warning above. It is a warning and
  not a refusal because the browser reloads: the tab is already at the right address
  when `varde up` has finished, and DHIS2 in particular takes minutes to answer
  after its container starts.
- A machine where docker cannot be asked at all - no CLI, no daemon - gets
  `docker could not be asked whether dhis2 is running, so this is the address and
  not a promise`, and the page is still opened. Refusing on a probe that could
  not be made would refuse everywhere docker is missing, which is not an answer
  about the component.

### On a server with no browser

The URL is handed to `open` on macOS, `start` on Windows and `xdg-open`
elsewhere, spawned and not waited for. A machine with none of them - a server
over ssh, which is where `varde` mostly runs - is told the address rather than
that something failed, and the exit code stays 0:

```text
the DHIS2 user interface is at http://localhost:18080
```

With `-v`, a hint names the opener that is missing:

```text
hint: there is no `xdg-open` on this machine to open it with, so the address above is the whole of it
```

`--no-browser` asks for that answer on a machine that does have an opener: the
address and the notes, and no browser window. A script or an AI agent working
for someone at another screen wants this, because a window appearing on the
machine the command runs on helps nobody:

```text
$ varde open ocs --no-browser
the OCS web interface is at http://localhost:8790
```

`varde open NAME --json` carries `url`, `page`, `opened`, `no_browser`, `running`
and the notes, and `messages` with every closing line and its level. That is
the shape to read from a script, and it never opens a
browser: `no_browser` is always `true` under `--json`. `varde open --json` carries
the listing under `components`.

### From the browser

`o` on the browser's components page does the same thing for the row under the
cursor, and `Open web interface` in the palette does it by name. It opens what
this deployment publishes **now**, so a component enabled in this session, or one
whose port was changed and not yet saved, is said in the footer rather than
opened: nothing is running behind an address `varde sync` has not written yet.
The browser reads no `.env`, so `CHAP_ROOT_PATH` is `varde open`'s to honour and
not the page's. See [The components page](./models.md#the-components-page).

## OCS

The `ocs` component runs `ghcr.io/dhis2/open-climate-service`. OCS publishes no
release tags yet, so the pin follows the moving `main` tag; `OCS_IMAGE_TAG` in
`.env` pins a build of your own, and `varde update` reports which of the two is
in force. The image is multi-arch, so no `platform:` pin is needed and an arm64
host runs it natively - unlike chap-core and the model images, which are amd64
only. It ships its own `HEALTHCHECK`, so the compose file declares none.

OCS serves a web interface as well as an API, so it is published on a host port
by default (8790). chap-core reaches it inside the deployment at
`http://ocs:9000`, on the compose default network, which is what makes OCS
usable as a data source for Chap: OCS's own example configuration carries a
commented-out `chirps-to-chap` workflow trigger for exactly that, and the file
scaffolded here leaves it out, so adding it is yours.

There is no `depends_on` between OCS and chap-core. They are independent
services and either one is useful without the other.

### `ocs/climate-service.yaml`

Each OCS instance is configured for one country or region. `varde` scaffolds
that file when the component is first enabled and **never rewrites it**: it is
yours from that moment on. It is mounted read-only at
`/app/climate-service.yaml`, which is where `CLIMATE_SERVICE_CONFIG` points.

Without any flags it holds example values for Laos, the country of the DHIS2
demo database, and a note saying so. `varde doctor` warns while that note is there, so an instance is
never deployed as the wrong country by accident; edit the file and delete the
note.

The three flags fill it in directly, on `init` and on `components enable` alike:

```sh
varde init mychap --with ocs \
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
varde components enable ocs --ocs-bbox -13.5,6.9,-10.1,10.0
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

`varde` writes the five as commented placeholders into `.env` when the `ocs`
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

Uncomment the ones you have, fill them in, and run `varde up`: a container
reads `.env` when compose creates it, so a running OCS does not pick up a
credential that was added after it started.

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
`CDSE_S3_ENDPOINT`; `varde` writes neither into `.env`, and `compose.ocs.yml`
names only the five above, so neither reaches the container. A plugin that needs
a profile or an endpoint other than its own default has to carry it itself.

Without either the variables or the file, an ERA5-Land ingestion that reaches the
Climate Data Store fails with
`No such file or directory: '/home/ocs/.ecmwfdatastoresrc'` - that is the
container looking for the file form of the same credential, in the home directory
of the image's `ocs` user.

`varde auth show` reports which of them `.env` sets, never their values:

```text
OCS data sources
  ECMWF_DATASTORES_URL  set
  ECMWF_DATASTORES_KEY  set
  EDH_API_KEY           unset
  CDSE_S3_ACCESS_KEY    unset
  CDSE_S3_SECRET_KEY    unset
```

No value is shown, whatever `--reveal` asks for: these are accounts with Copernicus and
Earth Data Hub rather than this deployment's own secret, so there is nothing to
paste into a client here - only the question of whether a dataset will ingest.
`varde doctor` says `ERA5-Land: credentials unset (WorldPop and CHIRPS3 work
without them)` on its `components` line, as information. It is not a warning: a
deployment with no credentials is a working deployment.

### Behind a reverse proxy

A public instance normally sits behind a proxy that terminates TLS, and then two
things change. The host port should not be published at all, so the proxy is the
only way in, and OCS has to be told its public origin - otherwise it builds its
STAC and openEO links from the request it received, which names the internal
address:

```sh
varde components enable ocs --port none --base-url https://ocs.example.org
varde init mychap --with ocs --ocs-base-url https://ocs.example.org   # at creation
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

`varde status` then shows where the instance actually is:

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
varde components enable ocs --read-only
varde restart ocs
varde components enable ocs --read-write   # and back again
```

Both flags edit the one `read_only` key in `ocs/climate-service.yaml` and leave
every other byte of it alone - your comments, your dataset list, your scheduler
- adding the key at the end when the file does not have it. The value is also
recorded in `components.yaml`, and `varde status` prints `read-only` next to the
OCS line:

```text
ocs  up  http://localhost:8790   read-only
```

OCS reads the file and not the record, and it reads it once at startup, so the
instance has to be recreated. The instance config is a bind mount, which compose
does not compare, so `varde restart` checks the file itself: written after the
`ocs` container was created, it recreates that one service and leaves chap-core
and the models alone. When `ocs` is already on, `varde components enable` says
so in its closing lines:

```text
ocs is now read-only
run `varde restart ocs` to apply
```

A second run with the same flag says `ocs is already read-only` and changes
nothing. Until the restart, `varde status` warns while
`ocs/climate-service.yaml` is newer than the `ocs` container:

```text
warning: `ocs/climate-service.yaml` changed after the ocs container started, so ocs may still use the old settings; run `varde restart ocs` to apply it
```

Check it after every deploy. `read_only` is an ordinary config key, so a build
of open-climate-service that predates read-only mode ignores it silently and
serves a fully writable instance:

```sh
curl -s http://localhost:8790/info | grep read_only   # must be true
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://localhost:8790/ingestions -d '{}'   # 403
```

Read-only applies to HTTP, so ingestion becomes an operator task. Ingest through
a throwaway container, which reads the credentials from your shell and never
gives them to the running service:

```sh
set -a; . ./.env; set +a
varde docker run -- run --rm --no-deps \
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

The directory existing is the whole declaration. `varde sync` then renders the
mount

```yaml
    volumes:
      - ./ocs/climate-service.yaml:/app/climate-service.yaml:ro
      - ./ocs/plugins:/app/plugins:ro
```

and adds `plugins_dir: /app/plugins` to `ocs/climate-service.yaml` if the file
does not already name one - a mount OCS is not pointed at is a mount it never
looks in. A `plugins_dir` you set yourself is never moved. `varde doctor`
reports the file count on its `components` line, as information.

The mount is conditional because compose refuses to start a service whose bind
source does not exist, so a project without plugins must not carry the line.
Nothing creates the directory for you, and `varde` never writes into it: the
plugins are yours, and a plugin with dependencies the image does not have needs
an image of its own.

### Adding a plugin to a running deployment

Run `varde up`, not `varde restart`. `varde up` is the only wrapper that
re-renders the compose files before it calls Docker; `varde restart`,
`varde logs`, `varde docker ps` and the rest run against the files exactly as
they are on disk. So on a deployment where `ocs` was enabled before
`ocs/plugins/` existed, `compose.ocs.yml` carries no plugin mount, and a
`varde restart` finds a container that already matches that file: nothing
happens at all - no error, no mount, no plugin.

```sh
mkdir -p ocs/plugins/datasets
cp clms_gpp.py clms_gpp.yaml ocs/plugins/datasets/
varde up      # syncs first, so the mount is rendered and then ocs is recreated
```

`varde sync` followed by `varde restart` is the same thing in two steps: once
the mount is in the file, the running container no longer matches it and compose
recreates it.

**This is the one place where `varde restart ocs` is the wrong reach.** That
belongs to [`ocs/climate-service.yaml`](#read-only-instances), and for the
opposite reason: that file is a bind mount, so an edit to it is already visible
inside the container and there is nothing to re-render - the only problem is
that OCS read the old text at startup, which recreating that one service fixes. A directory that did not exist at render time is the
other case entirely: the compose file itself is out of date, and no amount of
recreating mounts a directory it does not mention.

`varde doctor` is the safety net, because its `compose files` line is
`varde sync --check`:

```text
warn  compose files   out of date with .varde/ (2 to write, 5 unchanged, 0 to remove)
      run `varde sync`, or `varde up`, which syncs first
```

Two files to write, because the mount in `compose.ocs.yml` and the `plugins_dir`
key in `ocs/climate-service.yaml` are rendered by the same sync.

## The object store

OCS will soon require an S3-compatible object store. It does not read any `S3_*`
variable yet, so `varde` does not force one on a deployment: enabling `ocs`
prints a one-line note that the store is coming and that
`varde components enable s3` adds it.

When `s3` is on, the OCS service gets four forward-looking variables -
`S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET_KEY` and `S3_BUCKET` - with a comment
in the file saying that OCS does not read them yet. The exact contract is not
published, so those names may still change.

The store itself is RustFS. `varde` generates `S3_ACCESS_KEY` and
`S3_SECRET_KEY` (32 hex characters each) into `.env` once, and never rewrites
them: the data volume is created with them, and changing them later locks the
store's own contents away. The compose file substitutes them into the image's
own `RUSTFS_ACCESS_KEY` and `RUSTFS_SECRET_KEY`, so the credentials live in
`.env` and nowhere else.

A one-shot `s3-init` service creates the `ocs` bucket once the store is healthy.
It runs the same RustFS image and signs the request with `curl --aws-sigv4`, so
nothing extra is pulled: the image is Alpine-based and ships both curl and wget,
which is what the `s3` health check uses on `GET /health` as well. Creating a
bucket that already exists answers 200, so every later `varde up` runs the
one-shot again and changes nothing.

The store publishes no host port: OCS reaches it at `http://s3:9000` on the
compose default network. `varde components enable s3 --port 9002` publishes one
for an S3 client of your own. A published port is for a client and not for a
browser: RustFS serves no web interface, so `varde open s3` says that rather than
opening an XML error document. See
[Reaching a component from a browser](#reaching-a-component-from-a-browser).

## DHIS2

The `dhis2` component is a demo or development DHIS2 and a PostgreSQL of its own,
published on 8780 by default, for a deployment that wants one beside Chap. Chap
is often deployed with DHIS2 and not always, so this is one way to get one rather
than something a deployment needs.

It is the largest component here - four services, three volumes, a mandatory
instance config and a seeded database - and the only one with settings the browser
and the `--port` flag do not cover. It has a chapter of its own:

- **[DHIS2](./dhis2.md)**, which covers why it brings its own PostgreSQL,
  `dhis2/dhis.conf` and what DHIS2 will not start without, the seed dump and the
  one chance a deployment gets to apply it, the first start taking minutes, the
  4 to 5 GB the analytics populate phase wants, and why moving the image tag
  starts with `varde backup`.

One thing to read before deploying it:
[connecting the Modeling App to Chap](./dhis2.md#connecting-the-modeling-app-to-chap)
is a step of its own. The app reaches chap-core through a DHIS2 Route with
`code: "chap"`, `varde up` does not create it, and a seeded instance carries a
`chap` route from the demo dump that points at somebody else's server - so it
looks configured and is not. `varde dhis2 connect` is the command that settles
all of it.

## Standalone OCS

`--only` names the whole component set, and chap-core is in it only when you
name it:

```sh
varde init climate --only ocs,s3      # OCS and its object store, no Chap
varde init dhis --only dhis2          # a DHIS2 on its own
```

`--only ocs,s3` is `--with ocs,s3 --without chap-core` in one flag, and it
cannot be combined with either of them. `--only none` is no component at all:
a directory to add components to later, or, with `--models`, model services on
their own. The same set spelled out:

```sh
varde init climate --with ocs,s3 --without chap-core
```

Neither `compose.yml` nor `compose.varde.yml` is rendered, and the `-f` list is
just the component files and the (empty) marketplace umbrella. `varde status`
leaves out the chap-core line and counts components rather than models in its
closing line; a deployment with nothing running is told that nothing in it is
running, never that Chap is not, because there is no Chap here to be running.
`varde doctor` judges the deployment by its components alone.

`init` asks GitHub nothing about chap-core for such a deployment: no release is
looked up and no `compose.ghcr.yml` is downloaded. The commands that only
chap-core can answer (`varde jobs`, `varde api` without `--url`,
`varde models test`, and `varde update --chap-tag` or `--pin-chap-core`) refuse
with one line naming `varde components enable chap-core`. `varde update` on its
own still pulls the components' images. `varde backup create` has no database to
dump and says so on its `database` line. Turning chap-core on later renders
`compose.yml` from the copy built into varde and says so;
`varde update --pin-chap-core` then moves it to the newest release.

## Model services without chap-core

A model service does not need chap-core to run. With chap-core in the
deployment it registers there and is reached through it; without chap-core it
runs on its own and is reached on a host port of its own:

```sh
varde init ewars --only none --models chapkit_ewars_model
varde up
curl http://localhost:5001/health
open http://localhost:5001/docs
```

What changes for a model when the deployment has no chap-core:

- **Its overlay registers nowhere.** `SERVICEKIT_ORCHESTRATOR_URL` and the
  registration key are left out, and servicekit skips registration when the URL
  is unset. The overlay also stops waiting for `chap` in `depends_on`. See
  [What an overlay contains](./models.md#what-an-overlay-contains).
- **It gets a host port by default,** the lowest free one from 5001 up, because
  nothing else in the deployment can reach it. `--port N` picks one,
  `--port none` asks for none (the service is then only reachable from other
  containers on the compose network).
- **`varde status` asks the model itself.** Each row is judged by its container
  and then by `GET /health` on its host port: `up`, `running, not answering` or
  `not running`. `varde doctor` counts the same rows.
- **The chap-core commands refuse**: `varde models test` and `varde jobs` run
  models through chap-core, so they name `varde components enable chap-core`.

It works in both directions on an existing deployment. `varde components disable
chap-core` under enabled models keeps them, publishes a host port for each one
that had none, and says where each one now answers. `varde components enable
chap-core` puts the orchestrator URL back, and the models register on the next
`varde up`; the host ports they were given stay until you
`varde models unexpose` them.

The default model set is a default for a Chap deployment, not something you
asked for, so `--without chap-core` or `--only` with no `--models` starts with
no models. Name the ones you want with `--models ID,...`.

## A chap-core elsewhere

A deployment can use a chap-core it does not run: one started from a chap-core
checkout on this machine, another compose project, or a server. Its model
services then register there, and the chap-core commands talk to it.

```sh
varde init models --chap-core-url http://localhost:8000 --models chapkit_simple_multistep_model
varde components enable chap-core --url http://localhost:8000     # on an existing one
varde components enable chap-core --url http://localhost:8000 --models-host host.docker.internal
```

`.varde/components.yaml` records it as its own block, and `varde components
list` shows chap-core as `external`, at that URL:

```yaml
chap-core:
  enabled: false
chap-core-external:
  url: http://localhost:8000
  models_host: localhost
```

- **The overlays register there.** `SERVICEKIT_ORCHESTRATOR_URL` is the URL
  with a loopback host (`localhost`, `127.0.0.1`) turned into
  `host.docker.internal`, because inside a container the loopback is the
  container. Each overlay maps that name to the host gateway, which Docker
  Desktop does anyway and Linux needs.
- **chap-core calls the models back at `models_host` and their host port**,
  through `SERVICEKIT_HOST` and `SERVICEKIT_PORT`. Where the app listens in
  its container depends on the image, and varde reads that off the image's
  command when it enables the model:
  - An image that starts uvicorn with `--port 8000` (every marketplace model
    but one) listens on 8000, so the host port maps to 8000. servicekit 3,
    which these images have, checks 8000 before it registers.
  - An image whose command names no port reads `PORT` (the Simple Multistep
    model). Its servicekit 2 checks only `SERVICEKIT_PORT`, so varde sets
    `PORT` to the host port and maps that port on both sides.

  Every model gets a host port,
  as in any deployment without chap-core of its own. `localhost` is right for a
  chap-core process on this machine; a chap-core in a container needs
  `--models-host host.docker.internal`, and one on another machine the name it
  reaches this one by. `init` and `components enable` pick
  `host.docker.internal` themselves when a running container publishes the
  URL's port, and say so. `varde status` checks the way back through chap-core's
  proxy and shows a model it cannot reach as `registered, unreachable`.
- **`varde status`, `jobs`, `api` and `models test` use that URL** in place of
  `http://localhost:<API port>`. `varde status` shows only the version that
  chap-core reports. If chap-core does not answer, the error line says that
  this deployment does not run it, so start it there, or set another URL with
  `varde components enable chap-core --url URL`.

It is one or the other: `components enable chap-core --url` is refused while
this deployment runs its own chap-core (disable that first, which stops it),
`components enable chap-core` without `--url` forgets the external one and runs
varde's own again, and `components disable chap-core` forgets it too.

## What the other commands say

- **`varde up`** checks the components' host ports in its preflight, alongside
  the API port, and names the command that moves each one.
- **`varde status`** prints one line per enabled component under the chap-core
  line, each judged by its container first, exactly as `varde doctor` does: a
  component with no container reads `not running`, and one that has a container
  is then asked whatever it answers - OCS its `/health` endpoint, and only while
  that container is up. That gate is what keeps a stopped OCS from being reported `up` because
  another process answered on its host port: OCS defaults to 8790, so two
  deployments on one machine collide there, which is the clash the `up`
  preflight and `varde doctor` already warn about. An OCS instance with no host
  port cannot be asked from out here at all, so its container is the whole
  answer, and the line says `internal (proxy: <base_url>)` and `read-only`
  where those apply. An OCS
  instance that answered also reports what it holds, `3 datasets` and
  `212.0 MB data`. The rows come from `.varde/components.yaml` rather than from
  docker, so they are printed with nothing running too, each reading
  `not running` beside the address it will answer on. `--json` carries the rows
  under `components`, with `read_only`, `health_url`, `datasets` and
  `data_bytes`. A `dhis2` row is judged the same way and then asked one thing,
  `/api/ping`: a DHIS2 container can be up and the instance entirely broken, with
  Tomcat serving pages while every `/api/*` request answers 404, so a row whose
  ping goes unanswered reads `starting` rather than `up`. It carries no version,
  because no component row does and DHIS2 gives its own only to a logged-in
  session. See [`varde status`](./status.md).
- **`varde doctor`** adds a `components` line - which components are on, whether
  the OCS config is present and still the example, whether any data source
  credential is set, how many plugin files there are and, while the instance is
  up, what it holds - plus a port line per published component and an image line
  per component image. Those last facts ride on the warning about an unedited
  config as much as on an `ok` line: a deployment that has just enabled `ocs`
  still has the example config, and that is the run in which a missing ERA5-Land
  key is most worth reading. For `dhis2` the same line reports whether
  `dhis2/dhis.conf` is there - a `fail` when it is not, because DHIS2 throws on
  startup without it - and which seed this deployment is set to, including the
  case where the setting is `default` and varde knows no dump for the pinned
  minor line, which is the one to read before a first `varde up` brings up an
  empty DHIS2. A deployment with `dhis2` enabled also gets a `memory` line among
  the machine checks, measured as what docker says it can give a container
  rather than as the host's memory. The `deployment files` line names the component
  directories it does not count, since these facts are what covers them.
- **`varde auth show`** adds an `OCS data sources` block, set or unset, never the values. The two DHIS2
  secrets are not in it: they are this deployment's own, in `.env`, and
  `DHIS2_ENCRYPTION_PASSWORD` is one the database was created with rather than
  something to paste anywhere.
- **`varde update`** reports each component's image. `ocs` and `s3` follow moving
  tags, so there is no pin to move, only a pull: `docker compose pull` takes
  whatever the tag points at today. An active `OCS_IMAGE_TAG` or `S3_IMAGE_TAG`
  line in `.env` is your own pin, and is reported as such. `dhis2` is pinned to a
  minor line, which means the same tag is a newer patch release tomorrow: a pull
  moves it without anything on the screen changing, and a newer DHIS2 migrates
  `dhis2_db` irreversibly on the next `varde up`. A run that would re-pull it
  while that volume is already on the machine warns and names `varde backup
  create` first; see
  [Changing the DHIS2 version](./dhis2.md#changing-the-dhis2-version). When the
  pull brings an image the machine did not have, the closing line names the
  component and `varde restart` is what puts it in service.
- **`varde backup`** archives each component volume that holds state: `ocs_data`,
  `s3_data` and `dhis2_home`. The DHIS2 database is a `pg_dump` without the
  analytics tables, not a copy of `dhis2_db`; see
  [Backing it up](./dhis2.md#backing-it-up). `dhis2_dump` is left
  out on purpose - it is a download cache the one-shot refills - so no archive
  carries the seed dump. See [Backup and restore](./backup.md).
- **`varde ui`** has a components page beside the models one: `Tab` moves
  between them, the rows are the ones `varde components list` prints, and
  `space`, `p` and `P` do there what they do for a model. `o` opens the row's web
  interface, the same page `varde open` opens, and says in the footer when there
  is none. One `s` saves both pages. The settings it deliberately does not edit are named in each
  component's `i` overlay rather than hidden: OCS's `--base-url` and
  `--read-only`, because they write to `ocs/climate-service.yaml`, which is
  yours; and DHIS2's seed, which `varde components enable dhis2 --seed SPEC`
  changes. `v` on the `dhis2` row
  picks the DHIS2 version. See
  [The components page](./models.md#the-components-page).
