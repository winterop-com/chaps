# Roadmap

What is planned for chaps, and what is not. The order is not a promise.

## Planned

### Resource limits for each service

Today the compose files set no memory or CPU limits, except the heap of
DHIS2 (`DHIS2_JAVA_TOOL_OPTIONS` in `.env`). A large DHIS2 database, such as a
national dump, needs more memory for PostgreSQL and for DHIS2 than the demo
dumps do. The plan:

- Settings in `.chaps/components.yaml` and `.chaps/models.yaml` for the
  memory and the CPUs of each service, rendered as `mem_limit`, `cpus` and,
  for PostgreSQL, `shared_buffers` and `work_mem`.
- A preset for a large DHIS2 database.
- A `chaps doctor` check that compares the limits with the memory that docker
  can give.

### A longer wait for a large seed

The health check of `dhis2-db` waits at most about 15 minutes. A large
dump takes longer, so `chaps up` stops with "unhealthy" while the restore
continues. The wait must grow with the size of the dump.

### Levels for the output of every command

`chaps models` and `chaps update` print their lines with a level (info,
warning, hint), as [Status and output](./status.md#levels) describes. The
other commands still print text with no level. They change to the levels one
group at a time.

### Registry options only where they apply

`--registry-url`, `--offline` and `--cache-dir` are global options today. They
move to the commands that read the marketplace registry, under a "Registry
options" heading.

### Templates for the compose files

The large compose files are templates in `src/compose/templates/`, but the
optional parts are built in Rust: the seed one-shot, the port lines, the
mounts. The plan is to use minijinja (Jinja2 for Rust) so that the conditions
and the loops are in the template files, and the Rust code only gives the
values. The golden files in `tests/fixtures/` make sure that the output does
not change.

## Not planned

### Native Windows

chaps runs on Windows in WSL 2, with the Linux binary. The native Windows
builds are published, but they are not supported, and there is no PowerShell
installer. These parts are ready:

- the cache and data directories under `%LOCALAPPDATA%\chaps`;
- `chaps open`, Ctrl-C and `chaps self update`.

Native support would also need a Windows job in CI, a test double for docker
that is not a shell script, a path map for the mounts of `chaps chap`, and a
signed binary.
