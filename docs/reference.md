# Command reference

Generated from the `--help` texts by `make docs-reference`; edit
`src/cli.rs` and run that target rather than editing this file.

Every command accepts the global options listed under [chaps](#chaps),
`--json` included. Commands that need a deployment directory are hidden from
`chaps --help` outside one, but they are all listed here.

## chaps

chaps deploys and manages CHAP, the DHIS2 Climate Health Analytics Platform.

It runs chap-core, its worker and database with Docker Compose, and adds
forecasting models from the CHAP model marketplace as pinned overlays.
Start with `chaps init`. Docs: https://chap.dhis2.org

```text
Usage: chaps [OPTIONS] <COMMAND>
```

| Global option | Description |
| --- | --- |
| `--json` | Emit machine-readable JSON instead of human output. |
| `-C, --project-dir <DIR>` | Project directory (or any directory inside one); found like git finds .git. Default: `.`. |
| `--registry-url <URL>` | URL of the marketplace registry index. Default: `https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml`. |
| `--offline` | Never touch the network; use the cache or the embedded snapshot. |
| `--cache-dir <DIR>` | Directory for the cached registry snapshot. |

Subcommands: [`chaps init`](#chaps-init), [`chaps models`](#chaps-models), [`chaps ui`](#chaps-ui), [`chaps registry`](#chaps-registry), [`chaps sync`](#chaps-sync), [`chaps update`](#chaps-update), [`chaps up`](#chaps-up), [`chaps down`](#chaps-down), [`chaps logs`](#chaps-logs), [`chaps docker`](#chaps-docker), [`chaps backup`](#chaps-backup), [`chaps status`](#chaps-status), [`chaps auth`](#chaps-auth)

## chaps init

Create a deployment directory: compose files, .env and .chaps/.

```text
Usage: chaps init [OPTIONS] [DIR]
```

| Argument | Description |
| --- | --- |
| `<DIR>` | Directory to create, resolved against the current working directory. Default: `.`. |
| `--models <SPEC>` | Models to enable: `none`, `default`, or a comma-separated list of ids. Default: `default`. |
| `--interactive` | Pick the models in the browser instead of taking them from --models. |
| `--chap-tag <TAG>` | Image tag for the chap-core services: `latest` (the default, resolved to the newest release's `vX.Y.Z` so the deployment is reproducible), `master`, `dev`, or an exact `vX.Y.Z`. `chaps` also writes the `compose.ghcr.yml` that tag publishes; `--offline` keeps the tag as given and uses the compose file built into this binary. Default: `latest`. |
| `--force` | Overwrite an existing project in the target directory. |
| `--api-port <PORT>` | Host port to publish chap-core's API on. It is the only port the deployment publishes: model services are reached through it. Default: `8000`. |
| `--port-base <PORT>` | Lowest host port `chaps models expose` may publish a model on. Default: `5001`. |
| `--api-token <TOKEN>` | Protect the API with a token. Without a value one is generated; with one it is used verbatim. chap-core then rejects every request that carries no `Authorization: Bearer <token>`, apart from its health and `/system/info` endpoints, and `chaps` writes a second secret so the model services can still register. Both land in `.env`, which is the only place they are kept. Unset means no authentication at all: anyone who can reach the port can use the API. |
| `--no-env` | Do not write a .env file. |
| `--fresh-env` | Regenerate .env even when the directory already has one, rotating the database password. An existing database volume keeps the old one. |
| `--source <PATH>` | Build chap-core from a local checkout instead of the published images. Not supported yet; passing it is an error. |

## chaps models

Browse and manage marketplace models.

```text
Usage: chaps models [OPTIONS] <COMMAND>
```

Subcommands: [`chaps models list`](#chaps-models-list), [`chaps models search`](#chaps-models-search), [`chaps models info`](#chaps-models-info), [`chaps models enable`](#chaps-models-enable), [`chaps models disable`](#chaps-models-disable), [`chaps models expose`](#chaps-models-expose), [`chaps models unexpose`](#chaps-models-unexpose)

## chaps models list

List marketplace models.

```text
Usage: chaps models list [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--all` | Include every entry, templates as well as models. |
| `--templates` | List only templates. |
| `--enabled` | List only the models enabled in this project. |

## chaps models search

Search the marketplace by id, name or summary.

```text
Usage: chaps models search [OPTIONS] <QUERY>
```

| Argument | Description |
| --- | --- |
| `<QUERY>` | Text to look for; matching is case-insensitive. |

## chaps models info

Show everything known about one model.

```text
Usage: chaps models info [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

## chaps models enable

Enable a model: record it in .chaps/models.yaml and write its overlay.

```text
Usage: chaps models enable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--channel <CHANNEL>` | Follow a release channel instead of pinning a version. Values: `stable`, `latest`. |
| `--version <VERSION>` | Pin an exact version from the model's `versions` list. |
| `--port <PORT\|auto>` | Publish the service on this host port, or on the lowest free one with `auto`. Without it the model publishes nothing. |
| `--data-dir <PATH>` | Data directory inside the container, for images that differ from the default. |
| `--user <USER>` | User the container runs as, as `user:group`. |
| `--allow-template` | Enable a template even though it is scaffolding, not a model. |

## chaps models disable

Disable a model: drop it from .chaps/models.yaml and remove its overlay.

```text
Usage: chaps models disable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

## chaps models expose

Publish a host port for an enabled model.

```text
Usage: chaps models expose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--port <PORT\|auto>` | Host port to publish on; `auto` (the default) takes the lowest free one. |

## chaps models unexpose

Take an enabled model's host port away again.

```text
Usage: chaps models unexpose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

## chaps ui

Open the model browser.

```text
Usage: chaps ui [OPTIONS]
```

## chaps registry

Inspect and refresh the marketplace registry.

```text
Usage: chaps registry [OPTIONS] <COMMAND>
```

Subcommands: [`chaps registry update`](#chaps-registry-update), [`chaps registry show`](#chaps-registry-show)

## chaps registry update

Fetch the registry from the network and refresh the cache.

```text
Usage: chaps registry update [OPTIONS]
```

## chaps registry show

Show where the registry was loaded from and what it contains.

```text
Usage: chaps registry show [OPTIONS]
```

## chaps sync

Render the compose files from .chaps/ (what `up` does first).

```text
Usage: chaps sync [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Write nothing; exit non-zero if anything would change. |

## chaps update

Move the model and chap-core pins to what upstream publishes now, then pull and restart.

```text
Usage: chaps update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--dry-run` | Show what would change and write nothing. |
| `--no-restart` | Update the files and pull the images but do not run `up -d`. |
| `--pin-chap-core` | Turn a moving chap-core tag (`latest`, `master`, `dev`) into a pin on the newest release, the way `chaps init` does by default. |

## chaps up

Sync, then start the stack (docker compose up).

```text
Usage: chaps up [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `-a, --attach, --foreground` | Run in the foreground and stream all logs (Ctrl-C stops the stack). |
| `--pull` | Pull every image first, including a moving chap-core tag such as `latest` (docker compose up --pull always). |
| `--no-preflight` | Do not check the host ports first; let Docker report a conflict. |
| `<EXTRA>...` | Extra arguments passed through to docker compose up. |

## chaps down

Stop the stack (docker compose down).

```text
Usage: chaps down [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose down. |

## chaps logs

Show container logs (docker compose logs).

```text
Usage: chaps logs [OPTIONS] [SERVICE]...
```

| Argument | Description |
| --- | --- |
| `-f, --follow` | Keep streaming new output. |
| `<SERVICE>...` | Services to show logs for; all of them when omitted. |

## chaps docker

Talk to Docker directly: containers, images, raw compose commands.

```text
Usage: chaps docker [OPTIONS] <COMMAND>
```

Subcommands: [`chaps docker ps`](#chaps-docker-ps), [`chaps docker pull`](#chaps-docker-pull), [`chaps docker exec`](#chaps-docker-exec), [`chaps docker run`](#chaps-docker-run), [`chaps docker config`](#chaps-docker-config)

## chaps docker ps

List the containers this project is running (docker compose ps).

```text
Usage: chaps docker ps [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose ps. |

## chaps docker pull

Download the pinned images into the local Docker daemon (no files change).

```text
Usage: chaps docker pull [OPTIONS]
```

## chaps docker exec

Run a command inside one of the running containers.

```text
Usage: chaps docker exec [OPTIONS] <SERVICE> [CMD]...
```

| Argument | Description |
| --- | --- |
| `<SERVICE>` | Service to run the command in, as named in the compose files. |
| `<CMD>...` | Command and arguments to run; a shell (`sh`) when omitted. |

## chaps docker run

Run any docker compose command against this project's files.

```text
Usage: chaps docker run [OPTIONS] [ARGS]...
```

| Argument | Description |
| --- | --- |
| `<ARGS>...` | Arguments passed to docker compose after the project's -f list. |

## chaps docker config

Print the finished stack: every compose file merged into one document.

```text
Usage: chaps docker config [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose config. |

## chaps backup

Create or restore a backup of the database, model data and project files.

```text
Usage: chaps backup [OPTIONS] <COMMAND>
```

Subcommands: [`chaps backup create`](#chaps-backup-create), [`chaps backup restore`](#chaps-backup-restore)

## chaps backup create

Write a tar.gz of the database, the model data and the project files.

```text
Usage: chaps backup create [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--out <PATH>` | Where to write the archive: a file, or a directory to name it in. Default: `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` in the current directory. |
| `--no-db` | Leave the chap-core database out of the archive. |
| `--no-models` | Leave the model data volumes out of the archive. |

## chaps backup restore

Put a deployment back from an archive.

```text
Usage: chaps backup restore [OPTIONS] <ARCHIVE>
```

| Argument | Description |
| --- | --- |
| `<ARCHIVE>` | The `tar.gz` written by `chaps backup create`. |
| `--yes` | Skip the confirmation. Required when there is no terminal to ask at. |
| `--files-only` | Restore only `.env`, `.chaps/` and the compose files. Needs no Docker. |
| `--db-only` | Restore only the chap-core database. |
| `--no-models` | Leave the model data volumes as they are. |
| `--no-start` | Do not run `docker compose up -d` at the end. |

## chaps status

Check chap-core health and which model services have registered.

```text
Usage: chaps status [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--url <URL>` | Base URL of the chap-core API. Default: `http://localhost:<api_port>`, the port `.chaps/project.yaml` records. |
| `--timeout <SECONDS>` | Request timeout in seconds. Default: `5`. |

## chaps auth

Turn API authentication on or off, and show the token to paste into DHIS2.

```text
Usage: chaps auth [OPTIONS] <COMMAND>
```

Subcommands: [`chaps auth show`](#chaps-auth-show), [`chaps auth enable`](#chaps-auth-enable), [`chaps auth disable`](#chaps-auth-disable), [`chaps auth rotate`](#chaps-auth-rotate)

## chaps auth show

Say whether the API is protected, and by which token.

```text
Usage: chaps auth show [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--reveal` | Print the API token in full, to paste into the DHIS2 Modeling App. |

## chaps auth enable

Turn authentication on: generate the secrets, write them to .env and sync.

```text
Usage: chaps auth enable [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--token <TOKEN>` | Use this API token instead of generating one. |

## chaps auth disable

Turn authentication off again, keeping both values as comments.

```text
Usage: chaps auth disable [OPTIONS]
```

## chaps auth rotate

Replace both secrets with freshly generated ones.

```text
Usage: chaps auth rotate [OPTIONS]
```
