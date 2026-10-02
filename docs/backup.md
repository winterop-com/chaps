# Backup and restore

A deployment is four things: the files in the project directory, the chap-core
database in PostgreSQL, one data volume per model service, and one data volume
per volume an enabled component keeps state in (`ocs`, `s3`, and `dhis2` twice).
`chaps backup create` puts all four into one `tar.gz`, and
`chaps backup restore` puts them back.

## The archive

A plain gzipped tar with a flat layout, so `tar -tzf` reads it and an admin can
restore it by hand:

```text
chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz
  manifest.yaml              what this archive is: the chaps version that wrote
                             it, the UTC timestamp, the project directory name,
                             the chap-core image tag, the PostgreSQL server
                             version, every enabled model with its service id,
                             version, image tag, host port (null when it
                             publishes none), data dir and volume, and what was
                             included (or why it was not)
  files/                     .env, .chaps/**, ocs/**, dhis2/** and every
                             compose*.yml at the project root
  db/chap_core.dump          pg_dump in the custom format (-Fc)
  models/<service_id>.tar    one model's data directory, as tar saw it
  components/<name>.tar      one component volume, likewise: ocs, s3, dhis2-home
                             and dhis2-db
```

Every member but the manifest is optional, and the manifest records which ones
are there: what was included, what was skipped and why, and how long each
service was held still while its volume was read.

`ocs/climate-service.yaml` is in `files/` because it is the operator's own
file rather than a rendered artifact: `chaps sync` only ever creates a missing
one, so nothing can rebuild the edits made to it. `dhis2/dhis.conf` is the same
kind of file and is in `files/` on the same grounds - DHIS2 does not start
without it, and all `chaps sync` can do is scaffold a fresh one.

What is collected is the whole of each such directory rather than those two
files, because what an operator puts beside them - `ocs/plugins/` is the case in
point - is theirs for the same reason; anything nested under `dhis2/` is in the
archive too. A directory is collected whether or not its component is enabled at
the time, because `chaps components disable` leaves it alone and says it is
yours, so a backup taken while a component is off still protects its files. See
[DHIS2](./dhis2.md#dhis2dhisconf).

The archive is built and read with the system `tar` binary rather than a Rust
tar crate, so what an operator sees with `tar -tzf` is exactly what
`chaps backup restore` sees.

The `logs`, `runs`, `renv`, `uv` and `pytensor` volumes are deliberately left
out: they are caches that rebuild themselves, and they are far larger than
everything above put together.

## Taking a backup

```sh
chaps backup create [--out PATH] [--no-db] [--no-models] [--no-components]
```

The archive is named `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` (UTC)
and written to the current directory, unless `--out` names a file or an
existing directory to put it in.

PostgreSQL has to be running for the database part, because the dump goes
through `docker compose exec`; when it is not, the command stops and says to
start Chap or pass `--no-db`.

Model data is read through the overlay's one-shot `<service_id>-init`
container, which mounts the same named volume at the same path as the model
itself, so it works whether the model is running, stopped, or was brought down
entirely. Every model overlay has one, root included. A model that has never
started has no volume yet; it is skipped with a warning and recorded as such
in the manifest. A read that fails - the busybox image cannot be pulled, tar
errors out - is different: the archive is still written, for what it does
hold, and the volume is recorded as failed in the manifest, but `backup
create` exits non-zero and names what it could not read, so a cron job or a
script does not take a partial backup for a whole one.

Component data is read through a throwaway `busybox` container instead: no
component one-shot - `s3-init`, `dhis2-dump`, `dhis2-prep` - mounts a volume
that ends up in the archive. `--no-components` leaves all of it out.

`dhis2` contributes two members rather than one, `dhis2-home` and `dhis2-db`,
because it keeps `/opt/dhis2` - the installed apps and the file store - apart
from its database. Its third volume, `dhis2_dump`, is deliberately left out: it
is a download cache the `dhis2-dump` one-shot refills on its own, so archiving it
would add the whole seed dump to every backup of the deployment for nothing.
`chaps components disable dhis2 --purge` still takes it, and `chaps doctor` does
not call it a leftover; the archive is the one place it costs something.

A service that is running is paused for the seconds its volume takes to read:

```text
models    chapkit-ewars-model  /app/data  (40.0 KB, paused for 1.4 s)
```

`docker compose pause` freezes the processes in the container without touching
its storage, so nothing writes into a half-read tar - a chapkit model keeps a
live SQLite database in its data directory, and a tar of a file that is being
written to is a tar of a torn database. The service is always let go again,
including when the read fails - except when the backup itself is interrupted
(Ctrl-C) while it holds one: then the service stays paused, and the next
`chaps up` resumes it and says so (`resuming chapkit-ewars-model, left
paused`). Where `pause` is unsupported the service is
stopped and started instead (`stopped for 6.0 s`), and where neither works the
read goes ahead with a warning. The database needs none of this: `pg_dump`
reads one transactional snapshot, however busy chap-core is while it runs.

Two things to know about the pause. Docker cannot run a health check on a
frozen container, so a paused service is reported unhealthy until its next
probe succeeds - up to one health check interval (30 seconds for the services
here) after the backup, `chaps status` and `docker compose ps` may still say
so about a service that is fine. And a `chaps backup create` that is killed
outright cannot unpause what it paused; `chaps docker run -- unpause SERVICE`
lets it go.

Everything is staged under `.chaps/tmp/` (the same filesystem as the project,
so a multi-gigabyte model volume never lands in a small `/tmp`) and packed in
one `tar -czf` to `.<name>.tmp` beside the destination, which is renamed into
place only once tar has finished. The staging directory is removed afterwards,
success or not; a failure halfway leaves no half-written archive, and any
archive already at that path - last night's, say - exactly as it was.

## Restoring

```sh
chaps backup restore ARCHIVE [--yes] [--files-only] [--db-only] [--no-models]
                             [--no-components] [--adopt-identity] [--no-start]
```

It first prints what the archive holds, what it will overwrite and what it will
stop, and asks. `--yes` answers in advance; without a terminal to ask at and
without `--yes` it refuses rather than guessing. Nothing is touched before the
plan has been printed and confirmed.

Then, in order:

1. `docker compose stop chap worker <models> <components>`, only the services
   that are actually running, so nothing is woken up just to be stopped,
2. the files go back over the project directory, and `chaps sync` re-renders
   the compose files from the `.chaps/` that just arrived. A
   `.chaps/models.yaml`, `.chaps/models-manual.yaml` or `.chaps/components.yaml`
   the archive does not carry is removed here rather than kept: the deployment
   the backup came from had none, and keeping this one's would mix the two. A
   path in the archive's manifest that leads outside the project directory,
   or a compose file name in the restored `.chaps/` that does, stops the
   restore before anything is written. A `.env` that differs
   from the one in the archive is kept as `.env.before-restore`. The database
   credentials in it stay this deployment's (`POSTGRES_USER`,
   `POSTGRES_PASSWORD`, `POSTGRES_DB`, `CHAP_DATABASE_URL`, and
   `DHIS2_DB_PASSWORD` and `DHIS2_ENCRYPTION_PASSWORD` unless the archive's
   `dhis2_db` volume is restored too). A role and its password live in the
   database volume, the restore keeps this deployment's volume, and
   `pg_restore` restores rows, not roles, so the archive's password would not
   open it. Under `--adopt-identity` the volumes are the archive's, and so are
   the credentials. Every step
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
   holds,
6. `docker compose up -d`, unless `--no-start`.

The order is the whole design: nothing writes to the database or a data volume
while its storage is being swapped underneath it.

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
identity  chapy-ab12cd is kept; the archive's own (chapx-9f01bc) is not
          adopted, so this deployment keeps its containers and volumes
```

Everything else in `.chaps/project.yaml` does come from the archive, the API
port included - the `.env` beside it sets that too, and the two have to agree.

`--adopt-identity` is the takeover, for the one case that wants it: this
deployment *is* the one in the archive, moved to another directory or another
machine, and should answer to its name again.

Scopes:

- `--files-only` does step 2 and nothing else, and needs no Docker at all,
  which is handy for rebuilding a project directory on a new machine before
  starting anything.
- `--db-only` does step 3 alone.
- `--no-models` leaves the model data volumes as they are.
- `--no-components` leaves the component data volumes as they are.
- `--no-start` skips the last step.

`--files-only` cannot be combined with `--db-only`, `--no-models`,
`--no-components` or `--no-start`, and `--db-only` cannot be combined with
`--no-models` or `--no-components`.

## The same thing without chaps

Every step is an ordinary Docker command. They all need the project's `-f` list,
which `chaps docker run -- ...` supplies (or write out

```sh
docker compose -f compose.yml -f compose.chaps.yml -f compose.marketplace.yml ...
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
| write it back | `docker compose run --rm --no-deps -T <service>-init sh -c 'rm -rf <data_dir>/* && tar xf - -C <data_dir>' < <service>.tar` |
| hand it to the model | `docker compose run --rm --no-deps -T <service>-init chown -R 1000:1000 <data_dir>` (`0:0` for a root model) |
| check the connection | `docker compose exec -T postgres psql -U $PGU -d $PGDB -tAc 'select 1'` |
| hold a service still | `docker compose pause <service>`, and `unpause` afterwards |
| read a component volume | `docker run --rm -v <project>_ocs_data:/v busybox:1.37 tar -C /v -cf - . > ocs.tar` |
| write it back | `docker run --rm -i -v <project>_ocs_data:/v busybox:1.37 sh -c 'rm -rf /v/* /v/.[!.]* /v/..?* 2>/dev/null; tar -C /v -xf -' < ocs.tar` |

Stop `chap`, `worker`, the model services and the component services - `ocs`,
`s3`, `dhis2` and `dhis2-db` - before loading a database or a data volume, and
start them again afterwards. A DHIS2 whose `dhis2_db` volume is replaced under it
is the one case where it matters most: it read the schema version at startup. `<data_dir>` and
the uid:gid are in the manifest, one entry per model; `<project>` is the
`compose_project` in `.chaps/project.yaml`, which is also the prefix
`docker volume ls` shows.
