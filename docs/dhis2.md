# DHIS2

The `dhis2` component runs a DHIS2 instance and a PostgreSQL of its own beside
CHAP, for a deployment that wants one.

```sh
chaps init mychap --with dhis2
chaps components enable dhis2      # or afterwards
chaps dhis2 connect                # once DHIS2 answers, so the Modeling App can
                                   # reach CHAP
```

## What this component is for

A demo or a development DHIS2: something to log into while trying CHAP out,
developing against it, or showing it. It is not a way to run DHIS2 in
production, and that is upstream's own judgement of the images rather than a
limitation of `chaps` - `docker/DOCKERHUB.md` in `dhis2/dhis2-core` says *"We
cannot recommend the images for use in production"*. A production DHIS2 is
deployed by other means, and a CHAP deployment can be pointed at one without
this component: see [Connecting the Modeling App to CHAP](#connecting-the-modeling-app-to-chap)
for what the connection is made of, all of which is requests to DHIS2 that you
can make against an instance `chaps` did not deploy.

CHAP is often deployed **with** DHIS2 and not always. A deployment that already
has a DHIS2, or does not want one, leaves this component off and loses nothing:
chap-core, the models and OCS do not depend on it, and there is no `depends_on`
between DHIS2 and anything else here. This component is one way to get a DHIS2,
for the deployments that want one alongside.

## Four services and three volumes

`chaps sync` renders `compose.dhis2.yml`:

| Service | What it is |
| --- | --- |
| `dhis2` | DHIS2 itself, from `dhis2/core`, published on 8780 by default. Multi-arch since 2.39.1-rc, so nothing pins a platform for it. |
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

### Applying an edit

```sh
chaps restart dhis2
```

DHIS2 reads this file once, at startup, and `chaps` mounts it as a **bind
mount**, which compose does not compare: `docker compose up -d` alone finds
nothing to do after an edit. `chaps restart` checks the file itself - when it
was written after the `dhis2` container was created, that container is
recreated, and the line says so:

```text
recreating dhis2 to apply its edited config file
```

If the file's modification time does not show the edit (a copy that kept an
older one), `chaps restart --all dhis2` recreates the container regardless.

DHIS2 migrates before it serves a request again, so it is not back the moment
compose returns; the next `chaps dhis2` command waits for it and says what it
found. This is the same trap, for the same reason, as
[`ocs/climate-service.yaml`](./components.md#read-only-instances).

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
route.remote_servers_allowed = http://*,https://*
```

DHIS2 version 42 and later default it to `https://*` and then **refuse** an
`http://` target. The target the Modeling App needs is usually exactly that: the
app does not reach chap-core directly, it reaches it through a DHIS2 Route, and
chap-core is `http://chap:8000` inside the deployment,
`http://host.docker.internal:8000` when it runs on this machine
(`--chap-core-url`), and plain http on most lab servers. So the default value
would refuse the one route this component exists to make possible.

The scaffold allows every http and https target, so any chap-core works without
touching the file. A DHIS2 chaps deploys is a development or evaluation
instance; DHIS2 logs a warning about the wildcard on every start, and on a DHIS2
that is not a test instance the line belongs narrowed to the origins in use. One
origin per entry, comma separated, and none of them may carry a path - DHIS2
throws on startup if one does.

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

The 2.41 line used to have a dump as well; it is no longer published, so a 2.41
deployment starts empty unless `seed:` names one. DHIS2's Sierra Leone demo is
published for 2.41, 2.42 and 2.43
(`https://databases.dhis2.org/sierra-leone/<version>/dhis2-db-sierra-leone.sql.gz`),
without the climate data the Modeling App uses.

Published paths have differed in patch version and basename from one line to
the next, so `chaps` keeps a table keyed by the minor line (`2.42`, `2.42.1` and `2.42.1.1`
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
  port: 8780
  image_tag: '2.41'
  seed: default
```

at creation with `chaps init --with dhis2 --dhis2-tag 2.41`, later with
`chaps components enable dhis2 --tag 2.41` (and `--image` for another
repository), or in the browser:
`chaps ui`, `Tab` to the components page, `v` on the `dhis2`
row. It offers the minor lines chaps has a demo database for (and the version in
force, if that is another), says the forward-only rule as soon as you pick one
that differs, and warns again on save when `dhis2_db` is already there. Or with
an active `DHIS2_IMAGE_TAG` line in `.env`, which Compose reads last and
which therefore wins without a sync. The rendered service is
`dhis2/core:${DHIS2_IMAGE_TAG:-2.42}`, so the recorded tag is only the default.

An unreleased DHIS2 comes from another image repository: DHIS2 publishes its
development builds as `dhis2/core-dev`, with the next version under `master`.
`chaps init --with dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master` runs
it, and `image:` in the `dhis2` block of `.chaps/components.yaml` is the setting
afterwards. There is no demo database for a development build, so it starts
empty.

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

## Connecting the Modeling App to CHAP

A DHIS2 and a CHAP started side by side cannot talk, and the reason is worth
understanding before the commands make sense: **chap-core never calls DHIS2 and
DHIS2 never calls chap-core.** The only thing that talks to both is the Modeling
App running in a browser, and it reaches chap-core through **DHIS2's Route API** -
a reverse proxy configured as a `Route` row inside DHIS2, with `code: "chap"`,
that DHIS2 then serves under `/api/routes/chap/run/`.

Three things follow from that, and they are the three the commands do:

| Without it | What the app does |
| --- | --- |
| the `chap` route | redirects to `/get-started`; it cannot see CHAP at all |
| the `analytics_*` tables | has no data to send, and nothing on screen says why |
| the apps themselves | there is no CHAP user interface in DHIS2 |

```sh
chaps dhis2 show        # what this instance has, changing nothing
chaps dhis2 connect     # the route, the apps, then analytics
```

| Command | What it does |
| --- | --- |
| `chaps dhis2 show` | Asks and reports: where the `chap` route points and whether chap-core answers through it, what DHIS2's analytics timestamp is worth, which of the two apps are installed, and each piece that is missing. A route that is right in every field but that nothing answers through counts as missing. Writes nothing. |
| `chaps dhis2 route` | Creates the `chap` route, or **repoints** one that is there, then proxies a request through it to prove the whole path. |
| `chaps dhis2 analytics` | Generates the analytics tables and waits for them; `--no-wait` starts the run and leaves it going. |
| `chaps dhis2 apps` | Installs the Modeling App and the Climate App from the App Hub, at the newest version this DHIS2 can run. |
| `chaps dhis2 connect` | All three, in that order. |
| `chaps dhis2 use` | Points all of the above at a DHIS2 that runs elsewhere. See [A DHIS2 that runs elsewhere](#a-dhis2-that-runs-elsewhere). |

Every one of them is idempotent and meant to be re-run: a second `chaps dhis2
connect` repoints nothing, reinstalls nothing and says so. A step that found
nothing to do still reports it.

### `chaps up` does none of this, on purpose

`chaps up` is a thin wrapper around `docker compose up`, and three things make
this the wrong work to hang off it:

- it needs **DHIS2 credentials**, which are not chaps' to invent;
- it reaches the **network** - the App Hub, twice - and `chaps up` never does;
- DHIS2's API is **not ready when `up` returns**. Tomcat serves pages while every
  `/api/*` request 404s, which is the shape `chaps status`'s `dhis2` row already
  knows about, and analytics then takes tens of seconds on demo data and much
  longer on real data. An `up` that waited for that would be an `up` that takes
  an hour.

So `chaps init --with dhis2` and `chaps components enable dhis2` each end with the
line that names the command instead:

```text
note: the Modeling App reaches chap-core through a DHIS2 route, and this deployment has none yet; once DHIS2 answers, `chaps dhis2 connect` adds it, generates analytics and installs the apps
```

### `up` and `status` keep asking until a connect is recorded

That note is printed at the one moment DHIS2 does not exist yet. What follows it
is `chaps up`, then several minutes of restoring a dump and migrating a schema,
and by the time anything can be connected the note is far up the scrollback. A
deployment can therefore sit for good with every row `up`, `chaps doctor`
reporting no problem, and the Modeling App unable to reach CHAP at all.

So the two commands that report on a running deployment say it again, and go on
saying it until a connect has been recorded. `chaps up` closes with it, under
the line it always ends on:

```text
unchanged: chap, dhis2
run `chaps status` to check that everything answers
chaps has not connected this DHIS2 to CHAP; run `chaps dhis2 connect` once DHIS2 answers
```

and `chaps status` puts it under its verdict, with the model hints:

```text
chap-core   up   http://localhost:8700   2.42.6   auth: off
dhis2       up   http://localhost:8780

no models enabled; run `chaps models enable ID` to add one
  chaps has not connected this DHIS2 to CHAP; run `chaps dhis2 connect`
```

Both lines come off `.chaps/components.yaml` and nothing else. Neither asks
DHIS2 anything, which is why each says what **chaps** has recorded rather than
what DHIS2 is - and naming the command is safe either way, because every `chaps
dhis2` verb is idempotent and a second run changes nothing.

`chaps status` says it only while the `dhis2` row reads `up`, since a DHIS2 that
is not answering cannot be connected to anything and the advice could not be
taken. `chaps up` has asked nothing at all and DHIS2 is minutes from its first
request, so it says *once DHIS2 answers*. Neither says it on a deployment
[without chap-core](./components.md), where `chaps dhis2 connect` refuses: the
route would point at a service that is not there.

### `connected_at`, and what it is not

The line stops once `chaps dhis2 connect` has got through, and that is recorded
in the `dhis2` block of `.chaps/components.yaml`:

```yaml
dhis2:
  enabled: true
  port: 8780
  image_tag: '2.42'
  seed: default
  connected_at: 2026-09-27T12:09:53Z
```

**It is a note that the command ran, and it is not evidence.** Nothing decides
anything by it except whether those two lines are printed. The route can be
deleted, repointed at another server or disabled in DHIS2's own Route
administration a minute later, and this timestamp will not move, because nothing
reads DHIS2 to check it.

**[`chaps dhis2 show`](#connecting-the-modeling-app-to-chap) is the command that
asks DHIS2.** It reads the route out of the instance, proxies a request through
it to prove chap-core answers, lists which of the two apps are installed and
says what the analytics timestamp is worth - and it writes nothing, so it is
safe to run on anything. If the question is *can the Modeling App reach CHAP*,
that is the command; `connected_at` cannot answer it and does not try.

`connect` says as much where it writes the record:

```text
recorded in `.chaps/components.yaml`, so `chaps up` and `chaps status` stop asking
  a note that this ran, not proof the route is still right; `chaps dhis2 show` asks DHIS2
```

#### What sets it

A run of `chaps dhis2 connect` that ended with a **verified route and both apps
in place**. Two of the three steps, and deliberately so: the hint exists because
a DHIS2 beside a CHAP cannot be used at all without them - the app redirects to
`/get-started` with no route, and there is no CHAP user interface in DHIS2 with
no apps. Analytics is not part of it. The Modeling App reaches chap-core and
works without the `analytics_*` tables; empty tables are a deployment with no
data rather than one that cannot talk to itself, `chaps dhis2 analytics
--no-wait` is a supported shape in which no run has finished, and `chaps dhis2
show` reports the analytics question separately and with the evidence it is
worth.

The route is judged by the **proxied request**, not by the row: `connect` asks
chap-core for its health through `/api/routes/chap/run/`, the way the app does.

When chap-core has an API token (see [Authentication](./auth.md)), the route
carries it: `connect` writes it into the route's `auth` as an `Authorization:
Bearer` header, which DHIS2 adds to every request it proxies and never lists
back. `/health` needs no token, so with one on the check also asks for
`/v2/services`, which does; a route that reaches `/health` and gets a 401 past
it is rewritten, and one DHIS2 lists with no header auth at all is rewritten
before anything is asked. The Modeling App itself never needs the token.

The single-step verbs record nothing. `chaps dhis2 route` alone leaves a
deployment with a route and no apps, which is still one nobody can use CHAP
from.

`chaps dhis2 connect --offline` is the third answer, and it is neither of the
other two. It skips the app install and says so, so the run has looked at
nothing that could tell it whether the apps are in DHIS2 - it **judges nothing,
and leaves the record exactly as it found it**, in either direction. A
deployment nothing has connected still records no connect. A deployment that
was connected keeps the timestamp it has: the apps are still installed, this
run simply did not ask, and a flag that turned off an unrelated step is not
evidence that anything broke. Clearing on it would make `chaps up` say chaps
has not connected a DHIS2 that chaps connected.

#### What clears it

| When | What happens |
| --- | --- |
| `chaps components disable dhis2`, with or without `--purge` | Forgotten with the component, and the disable says so. The record is about a DHIS2 instance this deployment no longer has. |
| `chaps down --volumes`, when `dhis2_db` was actually removed | Forgotten with the database, and the line says why. |
| A `chaps dhis2 connect` that found something wrong | Cleared, and the report says `cleared`. A route nothing answered through, or an app missing or failed: the hint comes back, which is the answer that errs the safe way. |
| A `chaps dhis2 connect` that could not look | Nothing. `--offline` skips the apps, so the run has nothing to say about them and the timestamp stays where it was. |

The `chaps down --volumes` row is the one worth understanding. `dhis2_db` going
means the next `chaps up` restores [the seed dump](#the-seed) into a database
being created **and that dump ships a `chap` route of its own, pointed at an
external server**. A record that survived would suppress the one line asking the
operator to repoint it, on a deployment whose Modeling App is quietly talking to
somebody else's CHAP. Disabling the component is the same hazard by another
route, which is why it clears the record whether or not `--purge` was given: a
`dhis2_db` kept on disk may be re-created from the dump later anyway.

```text
note: the record of `chaps dhis2 connect` is forgotten with the component; a DHIS2 enabled here again is asked to connect afresh
```

#### What does not clear it

Nothing that happens inside DHIS2. Deleting the route, repointing it, disabling
it, dropping the `F_CHAP_MODELING_APP` authority, uninstalling either app, or
restoring a different database into the same volume by hand all leave the
timestamp exactly where it is, and the two hint lines stay quiet. So does a
restored [backup](./backup.md), which brings `.chaps/` and the volumes back
together and is therefore consistent, but says nothing about what has happened
to that DHIS2 since.

None of that is a gap to be closed by reading more state: a record can only ever
say what happened at one moment, and the live answer needs a request. `chaps
dhis2 show` is that request.

### The credentials, and where they come from

`chaps` holds no DHIS2 credentials, and every request above needs one: a
username and password (HTTP Basic), or a DHIS2 **personal access token**. The
rule is that **a password belongs to the user it was set with**, so the places
chaps reads are pairs, and the first that has a value wins:

| Order | Source | Notes |
| --- | --- | --- |
| 1 | `DHIS2_API_TOKEN` in `.env` | A personal access token. Preferred to the password beside it: it can be scoped and revoked. |
| 2 | `DHIS2_ADMIN_PASSWORD` in `.env`, for `DHIS2_ADMIN_USERNAME` | The deployment's own answer, and where its other secrets already live. The user is `admin` when the file names none. |
| 3 | `CHAPS_DHIS2_TOKEN` in the environment | A token kept off disk: `export` it for one shell. |
| 4 | `CHAPS_DHIS2_PASSWORD` in the environment, for `CHAPS_DHIS2_USERNAME` | A password kept off disk. Without `CHAPS_DHIS2_USERNAME` it is the password of the user `.env` names. |
| 5 | `admin` / `district` | The DHIS2 default, and only on a DHIS2 chaps deployed. |

The default is not a guess. A seeded demo dump ships that user, and a
Flyway-bootstrapped empty database gets it from `DefaultAdminUserPopulator`, which
has both words compiled in. It is never sent to an
[external DHIS2](#a-dhis2-that-runs-elsewhere): chaps did not create that
instance, so `district` is nobody's password there, and trying it would only be a
failed login in its audit log. An external DHIS2 with none of the four set is an
error before any request is made. So is a `.env` that names a user and sets no
password for them: the default belongs to `admin`, not to whoever the file names.

`.env` carries the variable names as commented placeholders. On a DHIS2 chaps
deployed they hold exactly the values already used, so uncommenting one changes
nothing until it is edited:

```ini
# DHIS2 login `chaps dhis2` uses. Only chaps reads these - no container is given
# them - and commented out means the DHIS2 default below. A personal access token,
# when set, is used instead of the username and password.
# DHIS2_API_TOKEN=
# DHIS2_ADMIN_USERNAME=admin
# DHIS2_ADMIN_PASSWORD=district
```

For an external DHIS2 the password line is empty, since there is no default to
write down. `chaps sync` adds the section when a DHIS2 is first used.
Only `chaps` reads these variables. None of them reaches a container, so none
appears in a generated compose file.

**`--user NAME`** asks for one user by name for one run. That always means a
password, even when a token is on offer, and only a password that belongs to
`NAME` is sent:

- `DHIS2_ADMIN_PASSWORD`, when `.env` names `NAME`;
- `CHAPS_DHIS2_PASSWORD`, unless `CHAPS_DHIS2_USERNAME` gives it to someone else;
- the default, for `admin` on a DHIS2 chaps deployed.

Anything else is refused before a request is made, rather than sending one user's
password in another's name. So a one-off run as someone else is:

```sh
CHAPS_DHIS2_PASSWORD=theirs chaps dhis2 show --user alice
```

There is deliberately **no `--password` or `--token` flag**, because a secret on
a command line ends up in the shell history and in `ps`. Nothing ever prints the
secret. A report names the user and says what the credential is and where it was
found (`password from .env`, `API token from CHAPS_DHIS2_TOKEN`, `the DHIS2
default password`). For a token, the user is whoever DHIS2 says owns it
(`GET /api/me`). `-v` traces the header as `Authorization: Basic <admin and its
password>` or `Authorization: ApiToken <the token>`.

A token is created in DHIS2 under **Profile > Personal access tokens**. Leave its
allowed HTTP methods unrestricted, or allow at least `GET`, `POST` and `PUT`:
`route` writes with `POST` and `PUT`, and `analytics` and `apps` start work with
`POST`. The token's user needs the same authority a password's would, and a
superuser has it.

A credential DHIS2 does not accept gets an error that says so, rather than
"request failed":

```text
error: DHIS2 at http://localhost:8780 did not accept the password for `admin` (the DHIS2 default password); set `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` in `.env`, or export `CHAPS_DHIS2_PASSWORD`
error: DHIS2 at https://dhis2.example.org did not accept the API token (API token from `.env`): it is expired, revoked, or not allowed from this address; set a current one as `DHIS2_API_TOKEN` in `.env`, or export `CHAPS_DHIS2_TOKEN`
```

### A DHIS2 that runs elsewhere

Most real deployments put CHAP beside a DHIS2 that already runs on a server of
its own. `chaps dhis2 use` records one, and from then on every `chaps dhis2`
verb talks to it instead of the `dhis2` component:

```sh
chaps dhis2 use https://dhis2.example.org --chap-url https://chap.example.org
```

It takes two URLs because the two directions are different:

| URL | Whose view | What it is for |
| --- | --- | --- |
| the DHIS2 URL | this machine's | every request chaps makes: the origin, plus the context path of a DHIS2 served under one (`https://example.org/dhis`) |
| `--chap-url` | DHIS2's | where the `chap` route points. chap-core as that DHIS2 server reaches it, including any `CHAP_ROOT_PATH` |

The route is the only reason for `--chap-url`, and chaps cannot work it out. The
compose alias `http://chap:8000` resolves only inside this deployment, and
`localhost` on the DHIS2 server is that server. So the route becomes
`<chap-url>/**` (`https://chap.example.org/**`), and a `--chap-url` that names
`localhost` or `127.x` is recorded with a note that it will not work.

The record lives in `.chaps/components.yaml`, next to the components:

```yaml
dhis2-external:
  url: https://dhis2.example.org
  chap_url: https://chap.example.org
  connected_at: null
```

`use` records the URLs and then asks the DHIS2 two things: whether `/api/ping`
answers, and whether it accepts the credential `chaps dhis2` would send. It
reports both, and names the next step: the credential to set, or `chaps dhis2
connect`. A URL that did not answer is still recorded, since the DHIS2 may just
be down for maintenance. The report says so.

```text
recorded the external DHIS2 at https://dhis2.example.org in `.chaps/components.yaml`
dhis2     https://dhis2.example.org (answers /api/ping)
chap-url  https://chap.example.org
route     https://chap.example.org/**
login     API token from `.env` (accepted)
connected never recorded
run `chaps dhis2 connect` to point its route at this CHAP
```

| Form | What it does |
| --- | --- |
| `chaps dhis2 use URL --chap-url URL` | Record one, or replace the one recorded. |
| `chaps dhis2 use URL` / `chaps dhis2 use --chap-url URL` | Move one of the two URLs of the one already recorded. |
| `chaps dhis2 use` | Say which DHIS2 `chaps dhis2` talks to, and ask it. Writes nothing. |
| `chaps dhis2 use --clear` | Forget it; `chaps dhis2` talks to the `dhis2` component again. |

What changes against an external DHIS2:

- **Credentials have no default.** See
  [the credentials](#the-credentials-and-where-they-come-from). A token is the
  natural choice for a server you do not own the admin password of.
- **Nothing asks docker.** There is no container. The wait for `/api/ping` is
  the whole check, and a DHIS2 that does not answer points you back at the
  recorded URL rather than at `chaps logs dhis2`.
- **The analytics timestamp is trusted.** No seed dump of chaps' was restored
  into it, so `lastAnalyticsTableSuccess` is that instance's own.
- **The allowlist is that server's.** A refused route names
  `route.remote_servers_allowed` in the `dhis.conf` on the DHIS2 server, which is
  its operator's file, not one in this directory.
- **`chaps open dhis2` opens it**, and `chaps up` names `chaps dhis2 connect`
  until a connect is recorded against it, just as for the component.
  `connected_at` is its own record. Moving either URL forgets it, because a
  connect was a fact about one DHIS2 and one route target.

It is one DHIS2 or the other, never both. `use` refuses while the `dhis2`
component is on, and `chaps components enable dhis2` refuses while an external
DHIS2 is recorded. Each names the command that clears the way.

### The route is repointed, not created

**A seeded instance already has a `chap` route, and it points at a stranger's
CHAP.** The climate demo dumps ship one - right code, right authority, not
disabled - aimed at an external server, so it looks fully configured. A
"create if absent" implementation would skip it and the deployment would quietly
send its data somewhere else.

`chaps dhis2 route` therefore compares and rewrites. It leaves the route alone
only when all three are already true, and names whichever is not:

- the URL is this deployment's chap-core;
- it is not disabled;
- it carries the `F_CHAP_MODELING_APP` authority.

```text
repointed the `chap` route at http://chap:8000/**
  it pointed at http://158.39.75.126/stable/**
  verified chap-core answered through it: healthy
```

The target is `http://chap:8000/**` - the compose service alias, never
`localhost`, because the request is made by DHIS2 from inside the deployment, and
`localhost` there is DHIS2's own container. The `/**` suffix is what makes DHIS2
proxy the paths *under* the route; without it exactly one path is proxied and
everything the app asks for after `/health` answers 404. A deployment that sets
`CHAP_ROOT_PATH` gets it in the middle (`http://chap:8000/master/**`), because
that prefix moves every chap-core route with it.

Then the route is **proved rather than assumed**: `GET
/api/routes/chap/run/health` goes through DHIS2 to chap-core and back, which is
the same address the Modeling App uses. A row in DHIS2's database says nothing
about whether DHIS2 can resolve the hostname, is allowed to reach it, or gets an
answer. If nothing answers, the route is still correct and the command still
succeeds - a chap-core that is down is `chaps up`'s problem - so the line is a
warning and `--json` carries `"verified": false`:

```text
warning: the `chap` route is in place but nothing answered through it: HTTP 502 Bad Gateway; run `chaps status` to see whether chap-core is up
```

`route.remote_servers_allowed` in
[`dhis2/dhis.conf`](#routeremote_servers_allowed) already permits every http and
https target, so the allowlist is not normally in the way. On an instance whose file was narrowed
or replaced it is, and DHIS2's refusal is turned into the line that says so:

```text
error: DHIS2 refused the route: version 42 and later only allow the targets `route.remote_servers_allowed` lists, and http://chap:8000/** has to be one of them; check that line in `dhis2/dhis.conf` and run `chaps restart dhis2`, which recreates it to read the file again
```

DHIS2's own answer to that write is `409 Conflict` with the message `Route URL
is not permitted`, which names neither the setting nor the file - and its
`errorCode`, `E1004`, is DHIS2's general-purpose conflict code rather than this
one's, so the message above is built from the message alone. `--all` is not a
flourish: see [Applying an edit](#applying-an-edit).

### Analytics, and the parameter that populates nothing

The Modeling and Climate apps read their figures out of DHIS2's `analytics_*`
tables, which exist only once analytics has been generated.

```sh
chaps dhis2 analytics
```

is `POST /api/resourceTables/analytics?skipTrackedEntities=true`, followed by
polling `GET /api/system/tasks/ANALYTICS_TABLE/{id}` until DHIS2 says the run
completed.

**No `lastYears`, ever.** It reads as an obvious optimisation and it is a trap:
measured on the climate demo, `&lastYears=8` produced **zero rows in every
analytics table** while reporting success in 16.9 seconds, where the same run
without it wrote 146,129 rows into `analytics_2024` in 15.7. A silent success
that populates nothing leaves the app with no data and nothing explaining why, so
there is no flag for it here. An instance so large that the years have to be
limited is a `curl` against DHIS2 rather than a footgun in this command.

DHIS2 runs **one** analytics job at a time, and a second `POST` queues behind the
first with an empty notifier - so a wait on it would report nothing for as long
as the first one takes. `chaps dhis2 analytics` therefore adopts a run that is
already going instead of asking for another:

```text
the analytics run that was already going finished in 4 minutes (job jFxL1tE0pAy)
```

A job left `RUNNING` by a hard stop blocks every future run for good;
[`dhis2-prep`](#four-services-and-three-volumes) resets those on every start,
which is why that is not a state you have to get out of by hand.

The run is not silent: a step is printed to stderr every half minute, and `-v`
prints every one DHIS2 announces. `--timeout SECONDS` is how long to wait, an
hour by default, and running out of it is not a cancellation - DHIS2 carries on,
and the same command watches the same job again.

### What `show` can say about analytics, and what it cannot

DHIS2 reports a `lastAnalyticsTableSuccess` in `/api/system/info`, and on a
[seeded](#the-seed) deployment **that timestamp is not about this deployment.**
It is a settings row, so it is restored with everything else: measured on a
2.42.6 with the Laos climate demo, a freshly seeded instance reported a last
success of `2026-06-16T07:51:00.093` while `analytics_2024` did not exist at all -
the table was absent, not empty. It is a fact about the database the dump was
taken from.

`chaps` reaches DHIS2 over HTTP and cannot look at the tables, so `show` reports
what it checked and labels the timestamp with what it is worth:

| Row | What chaps checked |
| --- | --- |
| `analytics  never run` | DHIS2 records no successful run at all. |
| `analytics  2026-09-27T10:10:40.043 (a run finished on this deployment)` | An analytics run has finished on this DHIS2 since it started, which `GET /api/system/tasks/ANALYTICS_TABLE` says. That notifier lives in the running process, so nothing a dump carries can put an entry in it. |
| `analytics  2026-06-16T07:51:00.093 (unconfirmed on a seeded database)` | DHIS2 records a success, this deployment was seeded, and no run has finished here since DHIS2 started. The timestamp proves nothing either way. |
| `analytics  2026-09-27T10:10:40.043` | DHIS2 records a success and this deployment has no seed, so its database was migrated from empty and the record was made against it. |

The third of those is a question and not a verdict, and it is put as one:

```text
missing: analytics may never have run on this deployment: a seeded database can carry the dump's timestamp, and no run has finished since DHIS2 started; run `chaps dhis2 analytics` to settle it
```

Restarting DHIS2 empties the notifier, so a seeded deployment whose run was made
before the last restart is reported `unconfirmed` again. That is the cautious
answer rather than a wrong one: running `chaps dhis2 analytics` a second time is
idempotent and cheap, and reading `show` as "analytics is done" when the tables
are absent is the failure this whole command exists to catch.

**A run that finished is not the same as tables with rows in them.** Nothing
here can see a row count, so nothing here claims one - which is the same
distinction the `lastYears` trap above turns on.

### The apps come from the App Hub, server-side

```sh
chaps dhis2 apps
```

resolves each app's newest version that this DHIS2 can run from
`https://apps.dhis2.org/api/v1/apps/{id}`, then `POST /api/appHub/{versionId}` -
and **DHIS2 downloads the app itself.** Nothing is fetched here and no multipart
upload is built.

| App | What it is for |
| --- | --- |
| Modeling App | the CHAP user interface inside DHIS2 |
| DHIS2 Climate App | imports climate data into DHIS2 through Google Earth Engine, which is where the Modeling App's covariates come from - chap-core does not fetch them |

A version whose `minDhisVersion` is newer than the instance is skipped: DHIS2
installs it happily and the app then fails in the browser. An app already
installed at that version is left alone, and one at another version is moved to
it.

**A blank bound is no bound.** The App Hub sends a bound it has not set as an
empty string rather than as an absent field - every one of the Modeling App's
published versions carries `"maxDhisVersion": ""` - and chaps treats missing,
empty, whitespace and unreadable the same on both sides. A bound it cannot
compare against is one it cannot honour, and refusing on it would refuse
everything.

Three names for the same app, and they all differ: the App Hub publishes the
Modeling App as `Modeling`, an instance lists it under `Modeling` with the key
`dhis2-chapmodeling-app`, and `chaps` calls it the **Modeling App** everywhere it
prints. Matching takes the punctuation and the case out and accepts one name
containing the other, so all three spellings find each other.

`chaps dhis2 show` reports those two apps and no others. A 2.42.6 ships 29
bundled apps of its own, identical on every DHIS2, and they are not what this
command answers for:

```text
apps       Modeling App 7.1.0, DHIS2 Climate App not installed
missing: the Climate App is not installed
```

Installed apps live in `/opt/dhis2`, which is the `dhis2_home` volume, so they
survive a recreate and [a backup](#backing-it-up) carries them.

This is the one step that cannot work `--offline`, and it needs the network twice:
here for the version id, and from DHIS2 for the app. Under `--offline` it is
refused with what it would have needed, and `chaps dhis2 connect --offline` does
the other two steps and reports the skip rather than failing:

```text
skipped: installing an app needs the DHIS2 App Hub, twice: chaps resolves the version there and DHIS2 downloads the app itself; run the command without `--offline`, or install both apps from DHIS2's own App Management page
```

### The waits are long, and they are refusals first

Before a single request, three things are refused with what is true instead: a
deployment without the component, one whose DHIS2 publishes no host port and so
cannot be reached from this machine at all, and one whose `dhis2` container docker
says is not running. A docker that cannot be asked is not a refusal - that would
refuse on every machine docker is absent from - so the container check degrades
to a trace line and DHIS2 is asked directly.

Then `/api/ping` is polled until it answers, for twenty minutes by default
(`--wait SECONDS`), because that is what a first start can take. A DHIS2 that is
already answering says nothing at all; one that is not says so once:

```text
DHIS2 at http://localhost:8780 is not answering /api/ping yet; waiting up to 20 minutes, and `chaps logs dhis2` is where the migration shows
```

`/api/ping` is the only route DHIS2 answers without credentials, so it is the
honest probe for "the API layer is up"; the authenticated `/api/system/info` right
after it is what settles whether the password is right, and its answer is the
version every report opens with.

## The files

```text
mychap/
  .chaps/
    components.yaml      intent: the dhis2 block - enabled, port, image_tag, seed
  compose.dhis2.yml      artifact: the four services and the three volumes
  dhis2/
    dhis.conf            the DHIS2 instance configuration - yours, written once
  .env                   the two generated secrets, three commented pins and the login
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

# DHIS2 login `chaps dhis2` uses. Only chaps reads these - no container is given
# them - and commented out means the DHIS2 default below. A personal access token,
# when set, is used instead of the username and password.
# DHIS2_API_TOKEN=
# DHIS2_ADMIN_USERNAME=admin
# DHIS2_ADMIN_PASSWORD=district
```

The commented lines each carry the value the command that reads them already
falls back to, so uncommenting one changes nothing until it is edited. The login
block is [a section of its own](#the-credentials-and-where-they-come-from) so that
a deployment which enabled DHIS2 before `chaps dhis2` existed gets it appended on
its next `chaps sync`; the two variables are the whole answer to where the
credentials come from, and a name nobody can find is a name nobody sets. For a seed
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
