# Backup and restore

A deployment is three things: the files in the project directory, the chap-core
database in PostgreSQL, and one data volume per model service.
`chaps backup create` puts all three into one `tar.gz`, and
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
  files/                     .env, .chaps/** and every compose*.yml at the
                             project root
  db/chap_core.dump          pg_dump in the custom format (-Fc)
  models/<service_id>.tar    one model's data directory, as tar saw it
```

Every member but the manifest is optional, and the manifest records which ones
are there. The archive is built and read with the system `tar` binary rather
than a Rust tar crate, so what an operator sees with `tar -tzf` is exactly what
`chaps backup restore` sees.

The `logs`, `runs`, `renv`, `uv` and `pytensor` volumes are deliberately left
out: they are caches that rebuild themselves, and they are far larger than
everything above put together.

## Taking a backup

```sh
chaps backup create [--out PATH] [--no-db] [--no-models]
```

The archive is named `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` (UTC)
and written to the current directory, unless `--out` names a file or an
existing directory to put it in.

PostgreSQL has to be running for the database part, because the dump goes
through `docker compose exec`; when it is not, the command stops and says to
start CHAP or pass `--no-db`.

Model data is read through each overlay's one-shot `<service_id>-init`
container, which mounts the same named volume at the same path as the model
itself, so it works whether the model is running, stopped, or was brought down
entirely. A model that has never started has no volume yet; it is skipped with
a warning and recorded as such in the manifest.

Everything is staged under `.chaps/tmp/` (the same filesystem as the project,
so a multi-gigabyte model volume never lands in a small `/tmp`) and packed in
one `tar -czf`. The staging directory is removed afterwards, success or not, and
a failure halfway leaves no half-written archive at the destination.

## Restoring

```sh
chaps backup restore ARCHIVE [--yes] [--files-only] [--db-only] [--no-models] [--no-start]
```

It first prints what the archive holds, what it will overwrite and what it will
stop, and asks. `--yes` answers in advance; without a terminal to ask at and
without `--yes` it refuses rather than guessing. Nothing is touched before the
plan has been printed and confirmed.

Then, in order:

1. `docker compose stop chap worker <models>`, only the services that are
   actually running, so nothing is woken up just to be stopped,
2. the files go back over the project directory, and `chaps sync` re-renders
   the compose files from the `.chaps/` that just arrived. A `.env` that differs
   from the one in the archive is kept as `.env.before-restore`,
3. postgres is started on its own (`docker compose up -d postgres`) if it is not
   up, and once `docker compose ps` reports it healthy the dump goes in through
   `pg_restore --clean --if-exists --no-owner`. `pg_restore` exits 1 when it
   finished but complained, most of all about dropping objects a fresh database
   never had, so exit 1 counts as success and the complaints are printed as
   warnings; anything above 1 fails the restore,
4. each model's data directory is emptied and refilled through its init
   container, then chowned back to the model's numeric uid:gid (busybox resolves
   no account names, which is why the numbers matter),
5. `docker compose up -d`, unless `--no-start`.

The order is the whole design: nothing writes to the database or a data volume
while its storage is being swapped underneath it.

Scopes:

- `--files-only` does step 2 and nothing else, and needs no Docker at all,
  which is handy for rebuilding a project directory on a new machine before
  starting anything.
- `--db-only` does step 3 alone.
- `--no-models` leaves the data volumes as they are.
- `--no-start` skips step 5.

`--files-only` cannot be combined with `--db-only`, `--no-models` or
`--no-start`, and `--db-only` cannot be combined with `--no-models`.

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
| hand it to the model | `docker compose run --rm --no-deps -T <service>-init chown -R 1000:1000 <data_dir>` |

Stop `chap`, `worker` and the model services before loading a database or a data
volume, and start them again afterwards. `<data_dir>` and the uid:gid are in the
manifest, one entry per model.
