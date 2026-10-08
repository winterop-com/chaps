# Backup and restore

A deployment is four things: the files in the project directory, the chap-core
database in PostgreSQL, one data volume per model service, and the state of
each enabled component: the `ocs` and `s3` volumes, the `dhis2_home` volume and
a `pg_dump` of the DHIS2 database.
`varde backup create` puts all four into one `tar.gz`, and
`varde backup restore` puts them back.

## The archive

A plain gzipped tar with a flat layout, so `tar -tzf` reads it and an admin can
restore it by hand:

```text
varde-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz
  manifest.yaml              what this archive is: the varde version that wrote
                             it, the UTC timestamp, the project directory name,
                             the chap-core image tag, the PostgreSQL server
                             version, every enabled model with its service id,
                             version, image tag, host port (null when it
                             publishes none), data dir, uid:gid and volume, and
                             what was included (or why it was not)
  files/                     .env, .varde/**, ocs/**, dhis2/** and every
                             compose*.yml at the project root
  db/chap_core.dump          pg_dump in the custom format (-Fc)
  models/<service_id>.tar    one model's data directory, as tar saw it
  components/<name>.tar      one component volume, likewise: ocs, s3 and
                             dhis2-home
  components/dhis2-db.dump   the DHIS2 database: pg_dump -Fc without
                             analytics_*, aggregated_*, completeness_*, _* and
                             the rows of audit
```

Every member but the manifest is optional, and the manifest records which ones
are there: what was included, what was skipped and why, and how long each
service was held still while its volume was read.

`ocs/climate-service.yaml` is in `files/` because it is the operator's own
file rather than a rendered artifact: `varde sync` only ever creates a missing
one, so nothing can rebuild the edits made to it. `dhis2/dhis.conf` is the same
kind of file and is in `files/` on the same grounds - DHIS2 does not start
without it, and all `varde sync` can do is scaffold a fresh one.

What is collected is the whole of each such directory rather than those two
files, because what an operator puts beside them - `ocs/plugins/` is the case in
point - is theirs for the same reason; anything nested under `dhis2/` is in the
archive too. A directory is collected whether or not its component is enabled at
the time, because `varde components disable` leaves it alone and says it is
yours, so a backup taken while a component is off still protects its files. See
[DHIS2](./dhis2.md#dhis2dhisconf).

The archive is built and read with the system `tar` binary rather than a Rust
tar crate, so what an operator sees with `tar -tzf` is exactly what
`varde backup restore` sees.

The `logs`, `runs`, `renv`, `uv` and `pytensor` volumes are deliberately left
out: they are caches that rebuild themselves, and they are far larger than
everything above put together.

## Taking a backup

```sh
varde backup create [--out PATH] [--no-db] [--no-models] [--no-components]
```

The archive is named `varde-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` (UTC)
and written to the current directory, unless `--out` names a file or a
directory to put it in. A path that ends in `/` is a directory. varde makes the
directories that do not exist yet.

PostgreSQL has to be running for the database part, because the dump goes
through `docker compose exec`. When it is not, the command writes nothing and
stops:

```text
error: the postgres container is not running, so the database cannot be dumped; start it with `varde up`, or pass --no-db
```

Model data is read through the overlay's one-shot `<service_id>-init`
container, which mounts the same named volume at the same path as the model
itself, so it works whether the model is running, stopped, or removed by
`varde down`. Every model overlay has one, root included. A model that has never
started has no volume yet; it is skipped with a warning and recorded as such
in the manifest. A read that fails - the busybox image cannot be pulled, tar
errors out - is different: the archive is still written, for what it does
hold, and the volume is recorded as failed in the manifest, but `backup
create` exits non-zero and names what it could not read, so a cron job or a
script does not take a partial backup for a whole one.

Component data is read through a throwaway `busybox` container instead: no
component one-shot - `s3-init`, `dhis2-dump`, `dhis2-prep` - mounts a volume
that ends up in the archive. The DHIS2 database is the exception: it is a
`pg_dump` run in `dhis2-db`, so `dhis2-db` must be running. If it is not, the
backup marks it as failed and exits non-zero; start it with `varde up`, or pass
`--no-components`. `--no-components` leaves all of it out.

`dhis2` contributes two members rather than one, `dhis2-home` and `dhis2-db`,
because it keeps `/opt/dhis2` - the installed apps and the file store - apart
from its database. Its third volume, `dhis2_dump`, is deliberately left out: it
is a download cache the `dhis2-dump` one-shot refills on its own, so archiving it
would add the whole seed dump to every backup of the deployment for nothing.
`varde components disable dhis2 --purge` still takes it, and `varde doctor` does
not call it a leftover; the archive is the one place it costs something.

`varde backup create` prints one line: where the archive is, and its size.
With `-v`, a hint for each part gives its size. For chap-core, one model and
DHIS2 on the Laos demo database, the backup took 5 seconds:

```text
$ varde -v backup create
wrote /srv/chapx/varde-backup-chapx-20261008-002753.tar.gz (21.7 MB gzipped, 57.2 MB of data)
hint: files: .env, .varde/components.yaml, .varde/compose.chap-core.v2.4.0.yml, .varde/models.yaml, .varde/project.yaml, dhis2/dhis.conf, compose.chapkit-ewars-model.yml, compose.dhis2.yml, compose.marketplace.yml, compose.varde.yml, compose.yml
hint: database: chap_core as chap (54.0 KB), PostgreSQL 17.11 (Debian 17.11-1.pgdg13+2)
hint: model chapkit-ewars-model: /app/data (54.0 KB, paused for 0.4 s)
hint: component dhis2: /opt/dhis2 (48.8 MB, paused for 1.4 s)
hint: component dhis2: pg_dump without analytics_*, aggregated_*, completeness_*, _* and the data of audit (8.3 MB)
hint: `varde backup restore /srv/chapx/varde-backup-chapx-20261008-002753.tar.gz` restores it
```

A part left out by `--no-db`, `--no-models` or `--no-components` is a hint. A
part left out for another reason, such as a model that never started, is a
warning:

```text
warning: chapkit-ewars-model: no ck_chapkit_ewars_model_data volume yet, so there is no data to back up; the service has never started
```

A service that is running is paused for the seconds its volume takes to read,
as the `paused for 1.4 s` above shows.

`docker compose pause` freezes the processes in the container without touching
its storage, so nothing writes into a half-read tar - a chapkit model keeps a
live SQLite database in its data directory, and a tar of a file that is being
written to is a tar of a torn database. The service is always let go again,
including when the read fails - except when the backup itself is interrupted
(Ctrl-C) while it holds one: then the service stays paused, and the next
`varde up` resumes it and says so:

```text
resuming dhis2, left paused (an interrupted `varde backup create`?)
```

Until then, `varde status` shows the paused service as `not running`, and it
does not answer. Where `pause` is unsupported the service is
stopped and started instead (`stopped for 6.0 s`), and where neither works the
read goes ahead with a warning. The database needs none of this: `pg_dump`
reads one transactional snapshot, however busy chap-core is while it runs.

**A large volume means a long pause.** A paused service does not answer.
Before varde pauses a running component service for a volume of 1 GB or more,
it warns. A model service gets no such warning: a model volume of 1.1 GB held
the model still for 24.8 seconds, without a line about it.
The DHIS2 database is not paused at all: it is a `pg_dump` without the
analytics tables, see [DHIS2](./dhis2.md#backing-it-up).

```text
warning: ocs is paused while varde copies 3.1 GB of `mychap_ocs_data`, and it does not answer until the copy is done; this can take minutes, so run the backup when nobody uses it, or use `--no-components` to leave out the volumes of every component
```

Two things to know about the pause. Docker cannot run a health check on a
frozen container, so a paused service is reported unhealthy until its next
probe succeeds - up to one health check interval (30 seconds for the services
here) after the backup, `varde status` and `docker compose ps` may still say
so about a service that is fine. And a `varde backup create` that is killed
outright cannot unpause what it paused; `varde docker run -- unpause SERVICE`
lets it go.

Everything is staged under `.varde/tmp/` (the same filesystem as the project,
so a multi-gigabyte model volume never lands in a small `/tmp`) and packed in
one `tar -czf` to `.<name>.tmp` beside the destination, which is renamed into
place only once tar has finished. The staging directory is removed afterwards,
also when the backup fails. A failure halfway leaves no half-written archive,
and any archive already at that path - last night's, say - exactly as it was.

Ctrl-C ends the command at once, so its staging directory stays in
`.varde/tmp/` as `backup-<pid>`, and no later command removes it. It can be as
large as the archive. If no backup runs, remove it with
`rm -rf .varde/tmp/backup-*`.

## Restoring

```sh
varde backup restore ARCHIVE [--yes] [--files-only] [--db-only] [--no-models]
                             [--no-components] [--adopt-identity] [--no-start]
```

The restore goes into a deployment that exists. In a directory without
`.varde/project.yaml`, it stops with ``run `varde init` first``. To restore on
a new machine or in a new directory, make the deployment first, then restore
into it:

```sh
varde init chapy --models none
cd chapy
varde backup restore /srv/backups/varde-backup-chapx-20261008-002753.tar.gz
```

The models and components that `init` sets do not matter: the restore replaces
`.varde/` with the one from the archive.

It first prints what the archive holds, what it will overwrite and what it will
stop, and asks. `--yes` answers in advance; without a terminal to ask at and
without `--yes` it refuses rather than guessing. Nothing is touched before the
plan has been printed and confirmed.

Then, in order:

1. `docker compose stop chap worker <models> <components>`, only the services
   that are actually running, so nothing is woken up just to be stopped,
2. the files go back over the project directory, and `varde sync` re-renders
   the compose files from the `.varde/` that just arrived. A
   `.varde/models.yaml`, `.varde/models-manual.yaml` or `.varde/components.yaml`
   the archive does not carry is removed here rather than kept: the deployment
   the backup came from had none, and keeping this one's would mix the two. A
   path in the archive's manifest that leads outside the project directory,
   or a compose file name in the restored `.varde/` that does, stops the
   restore before anything is written. A `.env` that differs
   from the one in the archive is kept as `.env.before-restore`. The database
   credentials in it stay this deployment's: `POSTGRES_USER`,
   `POSTGRES_PASSWORD`, `POSTGRES_DB`, `CHAP_DATABASE_URL` and
   `DHIS2_DB_PASSWORD`. A role and its password live in the database volume,
   the restore keeps this deployment's volume, and `pg_restore` restores rows,
   not roles, so the archive's password would not open it. The DHIS2 database
   is a `pg_dump` too, and it loads into this deployment's role.
   `DHIS2_ENCRYPTION_PASSWORD` comes from the archive when the restore loads
   the DHIS2 database, because DHIS2 encrypted some of those rows with it. When
   the restore leaves the DHIS2 database out, it stays this deployment's. Under
   `--adopt-identity` the volumes are the archive's, and so are the
   credentials. Every step
   below works from the deployment as it now is, so a `compose.ocs.yml` that
   arrived with the archive is in the `-f` list the rest of the restore uses,
3. postgres is started on its own (`docker compose up -d postgres`) if it is not
   up; once `docker compose ps` reports it healthy, `psql -tAc 'select 1'` has
   to answer before anything is dropped, and then the dump goes in through
   `pg_restore --clean --if-exists --no-owner`,
4. each model's data directory is emptied, hidden files included, and refilled
   through its init
   container, then chowned back to the model's numeric uid:gid (busybox resolves
   no account names, which is why the numbers matter),
5. each component data volume is emptied and refilled the same way, through a
   `busybox` container, since no component one-shot mounts a volume the archive
   holds. The DHIS2 database is the exception: varde starts `dhis2-db` alone,
   makes the database new (`dropdb`, `createdb`), loads the dump with
   `pg_restore -j 4`, and sets the mark of a complete restore,
6. `docker compose up -d`, unless `--no-start`.

If the archive holds the DHIS2 database, run `varde dhis2 analytics` after the
restore, when the `dhis2` line of `varde status` says `up`. The dump has no
analytics tables, so DHIS2 has no data for its dashboards and the Modeling App
until then. The restore does not remind you of this step. On the Laos demo
database, DHIS2 said `up` about 40 seconds after the restore, and
`varde dhis2 analytics` took 36 seconds.

The order is the whole design: nothing writes to the database or a data volume
while its storage is being swapped underneath it.

When it is done, the restore prints what it restored in one line, and the next
step. For the chap-core, model and DHIS2 archive above, the restore took 17
seconds:

```text
restored 11 files, the chap_core database, the data of chapkit-ewars-model, dhis2, dhis2-db
the deployment is starting
```

The services it stopped, the files, the `.env` copy, the kept credentials and
the identity are hints, shown with `-v`. For a restore into a new deployment
that had not started yet:

```text
restored 11 files, the chap_core database, the data of chapkit-ewars-model, dhis2, dhis2-db
hint: files: .env, .varde/components.yaml, ...
hint: the previous .env is in .env.before-restore
hint: kept this deployment's POSTGRES_PASSWORD, DHIS2_DB_PASSWORD in .env: its database volumes stayed, and they open only with these values
hint: chapv-60913c is kept; the archive's own (chapx-31ab52) is not adopted, so this deployment keeps its containers and volumes
the deployment is starting
hint: `varde status` shows when it answers
```

The `kept` line names only the values that differ from the archive's.

In a deployment that had not started yet, the restore makes the `dhis2_home`
volume itself, through `busybox`. Docker Compose did not make it, so each
later `docker compose up` prints a warning about it:

```text
level=warning msg="volume \"chapv-60913c_dhis2_home\" already exists but was not created by Docker Compose. Use `external: true` to use an existing volume"
```

The volume works, and `varde down --volumes` removes it. Do not add
`external: true`.

Under `--no-start` or `--files-only`, the last line is ``run `varde up` to
apply``.

### When the database restore fails

`pg_restore` exits 1 both when it finished and only complained about objects
`--clean --if-exists` could not avoid complaining about, and when it restored
nothing at all - a refused connection, a role that does not exist and a server
that is not listening are all exit 1 as well. Exit 1 therefore counts as
success only when

- `pg_restore` printed its closing `errors ignored on restore: N`, which it
  reaches only after finishing, and
- every `pg_restore: error:` line it printed is one of `already exists`,
  `does not exist` or `must be owner of`, the three that
  `--clean --if-exists --no-owner` cannot help producing, and none of them
  came from loading rows. A `COPY` into a table that `does not exist` reads
  like a harmless drop, but its rows were never imported:

  ```text
  pg_restore: error: could not execute query: ERROR:  relation "public.jobs" does not exist
  Command was: COPY public.jobs (id, name) FROM stdin;
  ```

Anything else, and any exit above 1, fails the restore: the last lines of
`pg_restore`'s stderr are printed, the exit code is non-zero, nothing claims
the database was restored, and neither the model volumes nor
`docker compose up -d` are reached. The `select 1` in step 3 exists so that the
common case - the wrong credentials, a postgres that is not up yet - fails
before anything has been dropped, with the error PostgreSQL actually gave.

### Whose deployment it becomes

A restore never takes the archive's compose project name. That name is what
every container and named volume of a deployment is prefixed with, so adopting
it would point this deployment at the volumes of the one the backup came from
and abandon its own. Restoring production's archive into a staging copy is a
normal thing to do, and the plan says which name is kept:

```text
  identity  chapy-8df379 is kept; the archive's own (chapx-31ab52) is not adopted, so this deployment keeps its containers and volumes
```

Everything else in `.varde/project.yaml` does come from the archive, the API
port included - the `.env` beside it sets that too, and the two have to agree.
The DHIS2 port in `.varde/components.yaml` also comes from the archive.

### A second deployment on the same machine

A staging copy on the machine of the deployment it copies gets that
deployment's ports from the archive (8700 and 8780 by default). The restore
ends with `docker compose up -d`, without the port check of `varde up`. If the
other deployment is up, the restore puts the data back and then stops with:

```text
Error response from daemon: failed to set up container networking: driver failed programming external connectivity on endpoint chapy-8df379-chap-1 (...): Bind for 0.0.0.0:8700 failed: port is already allocated
error: docker compose exited with status 1
```

The data is restored at that point. To prevent this, restore without the start
and move the ports before `varde up`:

```sh
cd chapy
varde backup restore ../chapx/varde-backup-chapx-20261008-002753.tar.gz --no-start
```

1. Set `CHAP_API_PORT=8701` in `.env`.
2. Run `varde components enable dhis2 --port 8781`.
3. Run `varde up`.

If you skip the steps, `varde up` names the ports in use and the commands that
move them. A `--files-only` restore also writes the ports of the archive, so do
the same steps after it.

### Taking over the identity

`--adopt-identity` is the takeover, for the one case that wants it: this
deployment *is* the one in the archive, moved to another directory or another
machine, and should answer to its name again. The plan says so:

```text
  identity  compose project chapx-31ab52, taken over from the archive (--adopt-identity)
```

On the same machine, the old directory then has the same compose project name.
varde in that directory shows the containers of the new one as its own, and
says nothing about the second directory. `varde down --volumes` in the old
directory removes the volumes the new one uses. Remove the old directory, and
do not run varde in it again.

After a takeover, `varde doctor` warns that the database volume "predates this
deployment". This is expected: the volume is the one from the old directory.

### Scopes

- `--files-only` does step 2 and nothing else, and needs no Docker at all,
  which is handy for rebuilding a project directory on a new machine before
  starting anything. It does not ask Docker what runs, so its plan always
  says `nothing is running, so nothing is stopped first`, also when the
  deployment is up.
- `--db-only` restores the chap-core database alone: it stops `chap` and
  `worker` (step 1), does step 3, and then starts the deployment (step 6)
  unless `--no-start`. A running model loses chap-core for that time, and
  registers again within about a minute.
- `--no-models` leaves the model data volumes as they are.
- `--no-components` leaves the component data volumes as they are.
- `--no-start` skips the last step.

`--files-only` cannot be combined with `--db-only`, `--no-models`,
`--no-components` or `--no-start`, `--db-only` cannot be combined with
`--no-models` or `--no-components`, and `--adopt-identity` cannot be combined
with `--db-only`.

## The same thing without varde

Every step is an ordinary Docker command. They all need the project's `-f` list,
which `varde docker run -- ...` supplies (or write out

```sh
docker compose -f compose.yml -f compose.varde.yml -f compose.marketplace.yml ...
```

yourself). `$PGU` and `$PGDB` are `POSTGRES_USER` and `POSTGRES_DB` from `.env`,
defaulting to `chap` and `chap_core`.

| step | command |
| --- | --- |
| list an archive | `tar -tzf <archive>` |
| read its manifest | `tar -xzf <archive> -O manifest.yaml` |
| unpack the files | `tar -xzf <archive> -C <project> --strip-components=1 files` |
| dump the database | `docker compose exec -T postgres pg_dump -U $PGU -Fc $PGDB > chap_core.dump` |
| load the database | `docker compose exec -T postgres pg_restore -U $PGU -d $PGDB --clean --if-exists --no-owner < chap_core.dump` |
| read a model's data | `docker compose run --rm --no-deps -T <service>-init tar cf - -C <data_dir> . > <service>.tar` |
| write it back | `docker compose run --rm --no-deps -T <service>-init sh -c 'rm -rf <data_dir>/* <data_dir>/.[!.]* <data_dir>/..?* 2>/dev/null; tar xf - -C <data_dir>' < <service>.tar` |
| hand it to the model | `docker compose run --rm --no-deps -T <service>-init chown -R 1000:1000 <data_dir>` (`0:0` for a root model) |
| check the connection | `docker compose exec -T postgres psql -U $PGU -d $PGDB -tAc 'select 1'` |
| hold a service still | `docker compose pause <service>`, and `unpause` afterwards |
| read a component volume | `docker run --rm -v <project>_ocs_data:/v busybox:1.38 tar -C /v -cf - . > ocs.tar` |
| write it back | `docker run --rm -i -v <project>_ocs_data:/v busybox:1.38 sh -c 'rm -rf /v/* /v/.[!.]* /v/..?* 2>/dev/null; tar -C /v -xf -' < ocs.tar` |
| dump the DHIS2 database | `docker compose exec -T dhis2-db sh -c 'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Fc -Z 1 -T "analytics_*" -T "aggregated_*" -T "completeness_*" -T "_*" --exclude-table-data=audit' > dhis2-db.dump` |
| load it | `docker compose cp dhis2-db.dump dhis2-db:/tmp/d.dump`, then in `dhis2-db`: `dropdb --force`, `createdb` and `pg_restore -j 4 --no-owner --no-privileges /tmp/d.dump` |
| mark it complete | in `dhis2-db`: `psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "COMMENT ON DATABASE \"$POSTGRES_DB\" IS 'varde: seed restored'"` |

On a seeded deployment, the health check of `dhis2-db` asks for the mark. The
new database from `createdb` does not have it. Without the last step, the
health check fails, Docker marks `dhis2-db` unhealthy after 30 checks (about 5
minutes), and `dhis2` waits for a healthy `dhis2-db` before it starts. See
[DHIS2](./dhis2.md#a-restore-that-stops).

Stop `chap`, `worker`, the model services, `ocs`, `s3` and `dhis2` before you
load a database or a data volume. Keep `dhis2-db` running for its own dump and
restore. Start the services again afterwards. `<data_dir>` and
the uid:gid are in the manifest, one entry per model; `<project>` is the
`compose_project` in `.varde/project.yaml`, which is also the prefix
`docker volume ls` shows.
