# DHIS2

The `dhis2` component runs a DHIS2 instance and a PostgreSQL of its own beside
CHAP, for a deployment that wants one.

```sh
chaps init mychap --with dhis2
chaps components enable dhis2      # or afterwards
```

## What this component is for

A demo or a development DHIS2: something to log into while trying CHAP out,
developing against it, or showing it. It is not a way to run DHIS2 in
production, and that is upstream's own judgement of the images rather than a
limitation of `chaps` - `docker/DOCKERHUB.md` in `dhis2/dhis2-core` says *"We
cannot recommend the images for use in production"*. A production DHIS2 is
deployed by other means, and a CHAP deployment can be pointed at one without
this component: see [What is not here yet](#what-is-not-here-yet) for what
connecting the two still takes.

CHAP is often deployed **with** DHIS2 and not always. A deployment that already
has a DHIS2, or does not want one, leaves this component off and loses nothing:
chap-core, the models and OCS do not depend on it, and there is no `depends_on`
between DHIS2 and anything else here. This component is one way to get a DHIS2,
for the deployments that want one alongside.

## Four services and three volumes

`chaps sync` renders `compose.dhis2.yml`:

| Service | What it is |
| --- | --- |
| `dhis2` | DHIS2 itself, from `dhis2/core`, published on 8080 by default. Multi-arch since 2.39.1-rc, so nothing pins a platform for it. |
| `dhis2-db` | Its PostgreSQL, from `ghcr.io/baosystems/postgis:16-3.5`. `expose`d on 5432 and never published. |
| `dhis2-dump` | A one-shot that prepares the [seed dump](#the-seed) and exits. Not rendered at all for a deployment that starts empty. |
| `dhis2-prep` | A one-shot that runs on every start, after the database is healthy and before DHIS2 boots. |

| Volume | What it holds |
| --- | --- |
| `dhis2_home` | `/opt/dhis2`: the installed apps and the file store, about 429 MB on a real instance. Without it both go with every recreate. |
| `dhis2_db` | The database's data directory. |
| `dhis2_dump` | The prepared dump - a **download cache**, not data. The one-shot fetches the dump again when the volume is empty. |

`dhis2-prep` does two things nothing else does, neither of them visible from
outside:

- `ANALYZE`. A freshly restored dump carries no planner statistics, and the
  analytics aggregations then get terrible query plans - measured here, a first
  analytics run of about 20 minutes instead of about 20 seconds. Autovacuum gets
  there eventually, but not before that first run.
- Reset any job a hard stop left flagged `RUNNING`. DHIS2 will not start a new
  analytics run while one is `RUNNING`, so a single stale row blocks analytics
  for good.

It runs under `ON_ERROR_STOP=0`, so a database DHIS2 has not migrated yet - one
with no `jobconfiguration` table - is a no-op rather than a failure.

## Its own database, and why it is not chap-core's

Three reasons, and the first is enough on its own.

1. **DHIS2 needs PostGIS.** Its own startup check throws without the `postgis`,
   `pg_trgm` and `btree_gin` extensions, so the image is a PostGIS one rather
   than plain PostgreSQL. `ghcr.io/baosystems/postgis:16-3.5` is the tag
   `dhis2-core`'s own compose file uses.
2. **chap-core deploys `postgres:17`**, which has none of the three.
3. **A component could not reach it anyway.** Upstream's `compose.yml` puts the
   database on a second network, off the compose default one, which is exactly
   what keeps a model service away from it. A component's services join the
   default network like a model's do.

The two databases are therefore separate containers with separate volumes and
separate passwords, and `DHIS2_DB_PASSWORD` in `.env` has nothing to do with the
CHAP one.

The health check on `dhis2-db` probes over TCP (`pg_isready -h 127.0.0.1`) on
purpose: while the entrypoint runs its init scripts the server listens on the
unix socket alone, so a socket probe would report a half-restored database as
ready and DHIS2 would start against half a database.

## `dhis2/dhis.conf`

DHIS2 loads this file at startup and **will not start without it**. There is no
environment-only mode: a missing file is a startup failure, not a fallback to
defaults. So `chaps` scaffolds it when the component is first enabled, mounts it
read-only at `/opt/dhis2/dhis.conf`, and never rewrites it - it is yours from
that moment on, exactly like
[`ocs/climate-service.yaml`](./components.md#ocsclimate-serviceyaml). `chaps
sync` re-creates it only if it has gone missing, which is worth doing precisely
because nothing starts without it.

What DHIS2 does with `${...}` in that file is substitute the environment
variable of that name, so the values live in the compose file and the file
itself carries placeholders. The `dhis2` service passes five:

| Variable | Default in the compose file |
| --- | --- |
| `DHIS2_DB_HOST` | `dhis2-db` |
| `DHIS2_DB_NAME` | `dhis` |
| `DHIS2_DB_USER` | `dhis` |
| `DHIS2_DB_PASSWORD` | `dhis`, overridden by the generated value in `.env` |
| `DHIS2_ENCRYPTION_PASSWORD` | empty, overridden by the generated value in `.env` |

The last is passed even when nothing sets it, so the placeholder in the file
becomes the empty string rather than standing there as a literal `${...}`.

### `encryption.password`

**24 characters or more.** DHIS2 encrypts stored credentials with it, and it
judges the length itself:

- shorter than 24, and it reports `ENCRYPTION_PASSWORD_TOO_SHORT` and stops;
- empty, and it reports `MISSING_ENCRYPTION_PASSWORD` and encrypts with a
  password compiled into DHIS2's own source - which every other DHIS2 in the
  world also has.

`chaps` generates 32 hex characters into `.env` as `DHIS2_ENCRYPTION_PASSWORD`,
comfortably past the floor. Like the database password it is written **once** and
never rewritten: the volume was created with it, and changing it later leaves the
credentials already in the database undecryptable.

### `route.remote_servers_allowed`

This is the one value `chaps` fills in rather than leaving as a placeholder:

```ini
route.remote_servers_allowed = http://chap:8000
```

DHIS2 version 42 and later default it to `https://*` and then **refuse** an
`http://` target. The target the Modeling App needs is exactly that: the app does
not reach chap-core directly, it reaches it through a DHIS2 Route, and chap-core
inside the deployment is `http://chap:8000`. So the default value would refuse
the one route this component exists to make possible.

One origin per entry, comma separated, and none of them may carry a path - DHIS2
throws on startup if one does. `http://*` is what a DHIS2 beside CHAP usually
settles for; the scaffold is narrower on purpose, because a wildcard is an open
server-side request forgery hole that DHIS2 itself warns about on every start.

### Behind a TLS-terminating proxy

Two lines are in the file, commented:

```ini
# server.https = on
# server.base.url = https://dhis2.example.org
```

The first marks the session cookie `Secure`, the second is the origin DHIS2
builds absolute links from. Uncomment **both** behind a proxy that terminates
TLS, and leave both commented while DHIS2 is reached over plain http - a `Secure`
cookie on an http origin makes the login loop.

Pair that with `--dhis2-port none` (or `chaps components enable dhis2 --port
none`), so the proxy is the only way in. `chaps` records no second copy of
`server.base.url`: that file is yours, so `chaps status` and `chaps components
list` say `internal` for such an instance rather than naming a proxy.

### Restoring an older dump

Two more commented lines cover the case where the seed dump comes from an older
DHIS2 than the image runs, which can stop on Flyway's migration order:

```ini
# flyway.repair_before_migration = on
# flyway.migrate_out_of_order = on
```

## The seed

`seed:` in `.chaps/components.yaml` decides what the database is restored from
the first time it is created. `--dhis2-seed` takes the same four answers at
`init`:

| Value | What happens |
| --- | --- |
| `default` | The demo dump published for the pinned minor line, if `chaps` knows of one. |
| `none` | Nothing. The database starts empty and DHIS2 migrates a new schema into it. |
| a URL | That dump, downloaded by the one-shot. An `http://` or `https://` prefix is the whole of the rule. |
| a path | That file, bind-mounted into the one-shot and copied rather than downloaded. |

```sh
chaps init mychap --with dhis2                              # the version-matched demo
chaps init mychap --with dhis2 --dhis2-seed none            # empty
chaps init mychap --with dhis2 --dhis2-seed dumps/laos.sql.gz
chaps init mychap --with dhis2 --dhis2-seed https://example.org/mine.sql.gz
```

There is no flag for it after `init`. Change `seed:` in
`.chaps/components.yaml` and run `chaps sync`.

A path has to be inside the deployment directory, because it becomes a bind mount
and compose refuses to start a service whose bind source does not exist. `chaps
sync` warns when the file is not there rather than leaving that for `chaps up`.
A URL seed under `--offline` is refused outright: the two ask for opposite
things.

### It applies once, to a database being created

The restore is the stock PostgreSQL entrypoint's own. `dhis2-dump` writes the
prepared dump into `dhis2_dump`, and `dhis2-db` mounts that volume at
`/docker-entrypoint-initdb.d/`, which the entrypoint reads **only** while it
initialises a data directory it has just created. So:

- the first `chaps up` restores the dump;
- every later `chaps up` does not, whatever `seed:` says;
- changing the seed on a deployment that has already started means removing
  `dhis2_db` first - `chaps components disable dhis2 --purge` is the honest way
  to ask for that, and it takes all three volumes.

Docker copies the image's own PostGIS init script into that volume as the
container is created, so the extensions are still set up, ahead of the dump.

### The default dump comes from a table, not from the tag

The published filename does not follow from the version:

| Minor line | Dump |
| --- | --- |
| `2.42` | `https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz` |
| `2.41` | `https://databases.dhis2.org/climate/laos/2.41.7/demo.sql.gz` |

A patch version in one path and not the other, and a different basename. So
`chaps` keeps a table keyed by the minor line (`2.42`, `2.42.1` and `2.42.1.1`
are all `2.42`) rather than building a URL from the tag, because a constructed
one would download a 404 on the first start and leave an empty DHIS2 with no
explanation.

A minor line the table does not list starts empty and says so, on every sync:

```text
warning: chaps knows no DHIS2 demo dump for 2.40, so `dhis2_db` starts empty; name one with `seed:` in `.chaps/components.yaml` (a URL or a path) and run `chaps sync`
```

### What the one-shot does to the dump

Two things, both learned from a restore that failed.

- **It verifies before it caches.** The download is checked with `gzip -t`,
  transformed, checked again, and published with one atomic `mv`. A half-finished
  download that got cached is a dump that restores without an error and leaves
  most of the data out.
- **It retrofits `--if-exists`.** Published dumps are `pg_dump --clean`
  *without* `--if-exists`, so they open on `DROP` and `ALTER` statements that
  assume the schema is already there - and the PostgreSQL entrypoint runs init
  scripts under `psql -v ON_ERROR_STOP=1`, where the first missing object aborts
  the whole restore. The rewrite makes that opening block a no-op on an empty
  database. It is idempotent and anchored to the start of each line, so a dump
  that already carries `--if-exists` is normalised rather than doubled and `COPY`
  data rows are left alone. `DROP EXTENSION` and `DROP SCHEMA` go entirely: the
  PostGIS image owns those objects, dropping them fails on the dependency, and
  the dump recreates what it needs with `CREATE EXTENSION IF NOT EXISTS`.

An operator's own dump goes through both, because the mounted file is copied into
the same working directory the download would land in and everything after that
is identical.

## The first start

**Minutes, not seconds.** DHIS2 migrates its whole schema before it serves a
request, and a seeded deployment restores the dump before that. Every other
service in a CHAP deployment answers in seconds, so an operator who does not know
this reads the first `chaps status` as a broken deployment. Both `chaps init
--with dhis2` and `chaps components enable dhis2` say so, and

```sh
chaps logs dhis2
```

is where the migration shows.

The honest budget, measured:

| Shape | Roughly |
| --- | --- |
| Native arm64, empty database | 40 seconds |
| Under emulation (an amd64 image on arm64, or the other way round) | 8 to 15 minutes |
| Seeded | the above, plus the restore, which is most of it on the climate demo |

The health check allows for that: `/api/ping` - the one route that answers
unauthenticated - with a `start_period` of 240 s and 60 retries, and 600 s on
`dhis2-db`, because on a seeded deployment the database's first start *is* the
restore.

## Memory

DHIS2 wants roughly **4 to 5 GB** for the analytics populate phase. Below that
the JVM is `SIGKILL`ed part-way through the populate, and the failure looks like
anything except memory: a container that exits with no Java exception in the log,
an analytics run that never finishes, a DHIS2 that was healthy a minute ago and
is now gone. On Docker Desktop the limit to raise is the VM's, not the container's.

The heap is passed as `JAVA_TOOL_OPTIONS`, not `JAVA_OPTS`: it is the JVM's own
variable and works for both image variants, while the distroless one ignores
`JAVA_OPTS` and then sizes the heap at about a quarter of the machine's memory.
The default is in the compose file and commented into `.env`:

```ini
# DHIS2_JAVA_TOOL_OPTIONS=-Xms2g -Xmx4g -XX:+UseG1GC
```

Uncomment and edit that line to change it; the variable replaces the options
rather than adding to them, so write the whole set.

## Changing the DHIS2 version

DHIS2 migrates its schema **forward only**, and that makes the image tag the one
setting here worth being careful with:

- a **newer** image migrates the database on the way up, irreversibly;
- an **older** image against a migrated database gives an instance that answers
  its own health check while every API request 404s. Up, and wrong - the worst
  shape a deployment can be in.

So: `chaps backup` first. Then move the tag deliberately, either in
`.chaps/components.yaml` followed by `chaps sync`:

```yaml
dhis2:
  enabled: true
  port: 8080
  image_tag: '2.41'
  seed: default
```

or with an active `DHIS2_IMAGE_TAG` line in `.env`, which Compose reads last and
which therefore wins without a sync. The rendered service is
`dhis2/core:${DHIS2_IMAGE_TAG:-2.42}`, so the recorded tag is only the default.

The tag is two parts rather than a full version on purpose, so a patch release
arrives with a `docker pull` rather than an edit.

`chaps` warns when a run moves the tag while `dhis2_db` is already there. It
compares the tag in the **rendered** `compose.dhis2.yml` - what is deployed -
against the one being asked for, not two records against each other:

```text
note: the DHIS2 image moves from 2.42 to 2.41 and `dhis2_db` is already there: DHIS2 migrates a schema forward only, so run `chaps backup` first - an older image on a migrated database answers healthy while every API request 404s
```

Moving the seed does not need this care and moving the tag does, which is why the
two are the settings the browser's components page deliberately does not edit:
its `i` overlay names both and sends you to `.chaps/components.yaml`.

## What is not here yet

**A DHIS2 with CHAP beside it is not yet usable from the Modeling App.** The app
does not talk to chap-core directly: it goes through a DHIS2 Route with
`code: "chap"`, and nothing in `chaps` creates that Route. Three pieces are
Phase 2:

- the `chap` Route itself, pointing at `http://chap:8000/**`;
- analytics generation, which the Modeling and Climate apps read through the
  `analytics_*` tables;
- installing the Modeling and Climate apps.

Worse than absent: **it will look configured.** The climate demo dumps ship a
`chap` route of their own, aimed at an external CHAP server, so a seeded instance
has a route named `chap` that resolves to somebody else's chap-core. It has to be
repointed, not created.

Until `chaps` does it, by hand:

```sh
# the route the demo dump shipped, and where it points
curl -u admin:district http://localhost:8080/api/routes/chap

# repoint it at this deployment's chap-core
curl -u admin:district -X PUT http://localhost:8080/api/routes/<id> \
  -H 'Content-Type: application/json' \
  -d '{"name":"chap","code":"chap","url":"http://chap:8000/**"}'

# and check it proxies through
curl -u admin:district http://localhost:8080/api/routes/chap/run/health
```

`route.remote_servers_allowed` in `dhis2/dhis.conf` is already set to that
target, so the allowlist is not what stands in the way.

## The files

```text
mychap/
  .chaps/
    components.yaml      intent: the dhis2 block - enabled, port, image_tag, seed
  compose.dhis2.yml      artifact: the four services and the three volumes
  dhis2/
    dhis.conf            the DHIS2 instance configuration - yours, written once
  .env                   the two generated secrets and three commented pins
```

The `.env` block is appended once, when the component is first enabled, and never
rewritten:

```ini
# DHIS2 (component). Generated once; the database volume was created with them.
DHIS2_DB_PASSWORD=d82bc75bc49b4c2670235dd1e0ff1fb8
DHIS2_ENCRYPTION_PASSWORD=217ab7126f92244466434ebc5a4c7c16
# DHIS2_IMAGE_TAG=2.42
# DHIS2_DB_DUMP_URL=https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz
# DHIS2_JAVA_TOOL_OPTIONS=-Xms2g -Xmx4g -XX:+UseG1GC
```

The three commented lines each carry the value the rendered compose file already
defaults to, so uncommenting one changes nothing until it is edited. For a seed
that is a file rather than a URL, `DHIS2_DB_DUMP_URL` carries the path *inside*
the container (`/opt/seed.sql.gz`), because the host path would be one nothing in
there can read.

`chaps components disable dhis2` keeps all three volumes and names each one, and
leaves `dhis2/` alone:

```text
disabled dhis2
removed compose.dhis2.yml
note: kept volume mychap-1ab2c3_dhis2_home; remove it with `chaps components disable dhis2 --purge` or `docker volume rm mychap-1ab2c3_dhis2_home`
note: kept volume mychap-1ab2c3_dhis2_db; remove it with `chaps components disable dhis2 --purge` or `docker volume rm mychap-1ab2c3_dhis2_db`
note: kept volume mychap-1ab2c3_dhis2_dump; remove it with `chaps components disable dhis2 --purge` or `docker volume rm mychap-1ab2c3_dhis2_dump`
note: the dhis2/ directory is left alone; it is yours
run `chaps up` to apply
```

## Backing it up

`chaps backup create` archives `dhis2_home` and `dhis2_db`, one member each, and
`chaps backup restore` puts both back. `dhis2_dump` is deliberately left out: it
is a download cache the one-shot refills on its own, so archiving it would add
the whole dump to every backup of the deployment for nothing. See
[Backup and restore](./backup.md).

`dhis2/` is in the archive's `files/`, on the same terms
`ocs/climate-service.yaml` is: it is scaffolded once and never rewritten, so an
edit made to `dhis.conf` exists nowhere else, and `chaps sync` can only write a
fresh one. The whole directory goes in rather than the one file, because whatever
you keep beside it is yours on the same grounds, and anything nested under
`dhis2/` is archived with it.

The directory is collected whether or not `dhis2` is enabled when the backup is
taken. `chaps components disable dhis2` leaves `dhis2/` alone and says it is
yours, so an archive written while the component is off still has your
`dhis.conf` in it, and a restore puts it back where it was.
