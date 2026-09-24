# Command reference

Generated from the `--help` texts by `make docs-reference`; edit
`src/cli.rs` and run that target rather than editing this file.

Every command accepts the global options listed under [chaps](#chaps),
`--json` included. Commands that need a deployment directory are hidden from
`chaps --help` outside one, but they are all listed here.

## chaps

deploy and manage CHAP, the Climate Health Analytics Platform.

```text
Usage: chaps [OPTIONS] <COMMAND>
```

| Global option | Description |
| --- | --- |
| `--json` | Emit machine-readable JSON instead of human output. |
| `--no-color` | Never colour the output (NO_COLOR does the same). |
| `-v, --verbose` | Show the commands and requests as they run. |
| `-d, --debug` | --verbose plus raw responses. |
| `-C, --project-dir <DIR>` | Project directory, or any directory inside one. Default: `.`. |
| `--registry-url <URL>` | URL of the marketplace registry index. Default: `https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml`. |
| `--offline` | Never touch the network; use the cache or the snapshot. |
| `--cache-dir <DIR>` | Directory for the cached registry snapshot. |

Subcommands: [`chaps init`](#chaps-init), [`chaps models`](#chaps-models), [`chaps components`](#chaps-components), [`chaps ui`](#chaps-ui), [`chaps registry`](#chaps-registry), [`chaps sync`](#chaps-sync), [`chaps update`](#chaps-update), [`chaps up`](#chaps-up), [`chaps down`](#chaps-down), [`chaps logs`](#chaps-logs), [`chaps restart`](#chaps-restart), [`chaps docker`](#chaps-docker), [`chaps backup`](#chaps-backup), [`chaps status`](#chaps-status), [`chaps jobs`](#chaps-jobs), [`chaps api`](#chaps-api), [`chaps doctor`](#chaps-doctor), [`chaps auth`](#chaps-auth), [`chaps self`](#chaps-self), [`chaps completions`](#chaps-completions)

## chaps init

Create a deployment directory: compose files, .env and .chaps/.

```text
Usage: chaps init [OPTIONS] [DIR]
```

| Argument | Description |
| --- | --- |
| `<DIR>` | Directory to create. Default: `.`. |
| `--models <SPEC>` | Models to enable: none, default, or a list of ids. Default: `default`. |
| `--interactive` | Pick the models in the browser instead of taking --models. |
| `--chap-tag <TAG>` | chap-core image tag: latest, master, dev or vX.Y.Z. Default: `latest`. |
| `--force` | Overwrite an existing project in the target directory. |
| `--api-port <PORT>` | Host port to publish chap-core's API on. Default: `8000`. |
| `--port-base <PORT>` | Lowest host port a model may be published on. Default: `5001`. |
| `--api-token <TOKEN>` | Protect the API with a token (generated when no value is given). |
| `--no-env` | Do not write a .env file. |
| `--fresh-env` | Regenerate .env, rotating the database password. |
| `--source <PATH>` | Build chap-core from a local checkout (not supported yet). |
| `--with <LIST>` | Components to add: ocs, s3. |
| `--without <LIST>` | Components to leave out: chap-core, ocs, s3. |
| `--ocs-base-url <URL>` | Public URL OCS is reached at, for a proxied instance. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. Malawi. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent. |
| `--ocs-bbox <BBOX>` | OCS extent as xmin,ymin,xmax,ymax in degrees. |

## chaps models

Browse and manage marketplace models.

```text
Usage: chaps models [OPTIONS] <COMMAND>
```

Subcommands: [`chaps models list`](#chaps-models-list), [`chaps models search`](#chaps-models-search), [`chaps models info`](#chaps-models-info), [`chaps models test`](#chaps-models-test), [`chaps models add`](#chaps-models-add), [`chaps models remove`](#chaps-models-remove), [`chaps models enable`](#chaps-models-enable), [`chaps models disable`](#chaps-models-disable), [`chaps models expose`](#chaps-models-expose), [`chaps models unexpose`](#chaps-models-unexpose)

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

## chaps models test

Make a model train and predict, and say whether it could.

```text
Usage: chaps models test [OPTIONS] [ID]...
```

| Argument | Description |
| --- | --- |
| `<ID>...` | Marketplace ids or service ids of the models to test. |
| `--all` | Test every model this project has enabled. |
| `--backtest` | Also run each model through chap-core as a backtest. |
| `--seed <N>` | Seed for the generated data, so a run can be repeated. |
| `--timeout <SECONDS>` | Give up on one model after this many seconds. |
| `--keep` | Keep what the test created instead of deleting it. |

## chaps models add

Add a model the marketplace does not list.

```text
Usage: chaps models add [OPTIONS] <SOURCE>
```

| Argument | Description |
| --- | --- |
| `<SOURCE>` | GitHub repository URL, or a ghcr image reference. |
| `--id <ID>` | Identifier to record the model under. |
| `--service-id <ID>` | Compose service name, which must match the service's own id. |
| `--name <NAME>` | Name to show in the listings. |
| `--port <PORT\|auto>` | Host port to publish the model on, or auto for a free one. |
| `--data-dir <PATH>` | Data directory inside the container. |
| `--user <USER>` | User the container runs as, as user:group. |
| `--runtime-amd64` | Record the image as published for amd64 only. |

## chaps models remove

Remove a model that was added with models add.

```text
Usage: chaps models remove [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Id of a model added with models add. |
| `--purge` | Delete the model's data volume as well. |

## chaps models enable

Enable a model and write its compose overlay.

```text
Usage: chaps models enable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--channel <CHANNEL>` | Follow a release channel instead of pinning a version. Values: `stable`, `latest`. |
| `--version <VERSION>` | Pin an exact version from the model's versions list. |
| `--port <PORT\|auto>` | Host port to publish the model on, or auto for a free one. |
| `--data-dir <PATH>` | Data directory inside the container. |
| `--user <USER>` | User the container runs as, as user:group. |
| `--allow-template` | Enable a template even though it is not a model. |

## chaps models disable

Disable a model and remove its compose overlay.

```text
Usage: chaps models disable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--purge` | Delete the model's data volume as well. |

## chaps models expose

Publish a host port for an enabled model.

```text
Usage: chaps models expose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--port <PORT\|auto>` | Host port to publish on, or auto for the lowest free one. |

## chaps models unexpose

Take an enabled model's host port away again.

```text
Usage: chaps models unexpose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

## chaps components

Show and change what this deployment is made of.

```text
Usage: chaps components [OPTIONS] <COMMAND>
```

Subcommands: [`chaps components list`](#chaps-components-list), [`chaps components enable`](#chaps-components-enable), [`chaps components disable`](#chaps-components-disable)

## chaps components list

List every component and whether this deployment has it.

```text
Usage: chaps components list [OPTIONS]
```

## chaps components enable

Turn a component on, or change the settings of one that is.

```text
Usage: chaps components enable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: ocs, s3 or chap-core. |
| `--port <PORT\|none>` | Host port to publish on, or none to keep it internal. |
| `--base-url <URL>` | Public URL OCS is reached at, for a proxied instance. |
| `--read-only` | Refuse ingestion over HTTP on this OCS instance. |
| `--read-write` | Allow ingestion over HTTP again. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. Malawi. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent. |
| `--ocs-bbox <BBOX>` | OCS extent as xmin,ymin,xmax,ymax in degrees. |

## chaps components disable

Turn a component off and remove its compose file.

```text
Usage: chaps components disable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: ocs, s3 or chap-core. |
| `--purge` | Delete the component's data volume as well. |

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

Show where the registry was loaded from and what it holds.

```text
Usage: chaps registry show [OPTIONS]
```

## chaps sync

Render the compose files from .chaps/.

```text
Usage: chaps sync [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Write nothing; exit non-zero if anything would change. |

## chaps update

Move the pins to what upstream publishes now, and pull.

```text
Usage: chaps update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--dry-run` | Show what would change: no pull, and nothing written. |
| `--pin-chap-core` | Pin a moving chap-core tag to the newest release. |

## chaps up

Sync, then start CHAP (docker compose up).

```text
Usage: chaps up [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `-a, --attach, --foreground` | Run in the foreground and stream all logs (Ctrl-C stops CHAP). |
| `--pull` | Pull every image first (docker compose up --pull always). |
| `--no-preflight` | Do not check the host ports first. |
| `<EXTRA>...` | Extra arguments passed through to docker compose up. |

## chaps down

Stop CHAP (docker compose down).

```text
Usage: chaps down [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `--volumes` | Also remove this deployment's volumes, destroying its data. |
| `-y, --yes` | Skip the confirmation. |
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

## chaps restart

Recreate the services whose image or configuration changed.

```text
Usage: chaps restart [OPTIONS] [SERVICE]...
```

| Argument | Description |
| --- | --- |
| `--all` | Recreate the named services even when nothing changed. |
| `<SERVICE>...` | Services to restart; the whole project when omitted. |

## chaps docker

Talk to Docker directly: containers, images, compose.

```text
Usage: chaps docker [OPTIONS] <COMMAND>
```

Subcommands: [`chaps docker ps`](#chaps-docker-ps), [`chaps docker pull`](#chaps-docker-pull), [`chaps docker exec`](#chaps-docker-exec), [`chaps docker run`](#chaps-docker-run), [`chaps docker config`](#chaps-docker-config)

## chaps docker ps

List the containers this project is running.

```text
Usage: chaps docker ps [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose ps. |

## chaps docker pull

Download the pinned images into the local Docker daemon.

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
| `<SERVICE>` | Service to run the command in. |
| `<CMD>...` | Command to run; a shell (sh) when omitted. |

## chaps docker run

Run any docker compose command against this project.

```text
Usage: chaps docker run [OPTIONS] [ARGS]...
```

| Argument | Description |
| --- | --- |
| `<ARGS>...` | Arguments passed to docker compose after the -f list. |

## chaps docker config

Print every compose file merged into one document.

```text
Usage: chaps docker config [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose config. |

## chaps backup

Create or restore a backup of this deployment.

```text
Usage: chaps backup [OPTIONS] <COMMAND>
```

Subcommands: [`chaps backup create`](#chaps-backup-create), [`chaps backup restore`](#chaps-backup-restore)

## chaps backup create

Write a tar.gz of the database, the data and the files.

```text
Usage: chaps backup create [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--out <PATH>` | Where to write the archive: a file, or a directory. |
| `--no-db` | Leave the chap-core database out of the archive. |
| `--no-models` | Leave the model data volumes out of the archive. |
| `--no-components` | Leave the component data volumes out of the archive. |

## chaps backup restore

Put a deployment back from a backup archive.

```text
Usage: chaps backup restore [OPTIONS] <ARCHIVE>
```

| Argument | Description |
| --- | --- |
| `<ARCHIVE>` | The tar.gz written by chaps backup create. |
| `--yes` | Skip the confirmation. |
| `--files-only` | Restore only the project files; no Docker needed. |
| `--db-only` | Restore only the chap-core database. |
| `--no-models` | Leave the model data volumes as they are. |
| `--no-components` | Leave the component data volumes as they are. |
| `--adopt-identity` | Take over the compose project name from the archive. |
| `--no-start` | Do not start the deployment at the end. |

## chaps status

Show chap-core health and which models registered.

```text
Usage: chaps status [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--url <URL>` | Base URL of the chap-core API. |
| `--timeout <SECONDS>` | Request timeout in seconds. Default: `5`. |

## chaps jobs

List the backtests and predictions chap-core has run.

```text
Usage: chaps jobs [OPTIONS]
       chaps jobs <COMMAND>
```

| Argument | Description |
| --- | --- |
| `--status <STATUS>` | Only jobs in this status; repeat for more than one. |
| `--type <TYPE>` | Only jobs of this type, such as create_backtest. |
| `--limit <N>` | Show at most this many jobs. |

Subcommands: [`chaps jobs list`](#chaps-jobs-list), [`chaps jobs show`](#chaps-jobs-show), [`chaps jobs logs`](#chaps-jobs-logs), [`chaps jobs cancel`](#chaps-jobs-cancel), [`chaps jobs delete`](#chaps-jobs-delete)

## chaps jobs list

List the jobs chap-core knows about, newest first.

```text
Usage: chaps jobs list [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--status <STATUS>` | Only jobs in this status; repeat for more than one. |
| `--type <TYPE>` | Only jobs of this type, such as create_backtest. |
| `--limit <N>` | Show at most this many jobs. |

## chaps jobs show

Show everything chap-core records about one job.

```text
Usage: chaps jobs show [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## chaps jobs logs

Print one job's log, which is where a failure says why.

```text
Usage: chaps jobs logs [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |
| `--tail <N>` | Print only the last N lines of the log. |

## chaps jobs cancel

Ask chap-core to stop a job that is still running.

```text
Usage: chaps jobs cancel [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## chaps jobs delete

Remove a finished job from chap-core's list.

```text
Usage: chaps jobs delete [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## chaps api

Send one authenticated request to chap-core's API.

```text
Usage: chaps api [OPTIONS] <METHOD> <PATH>
```

| Argument | Description |
| --- | --- |
| `<METHOD>` | GET, POST, PUT, PATCH or DELETE; case does not matter. |
| `<PATH>` | Request path starting with /, query string and all. |
| `--data <JSON\|@FILE\|->` | JSON body: inline, @file, or - to read stdin. |
| `--url <URL>` | Base URL of the chap-core API. |
| `--raw` | Print the body exactly as it arrived. |
| `--timeout <SECONDS>` | Request timeout in seconds. Default: `30`. |

## chaps doctor

Run a checklist over this machine and this deployment.

```text
Usage: chaps doctor [OPTIONS]
```

## chaps auth

Turn API authentication on or off, and show the token.

```text
Usage: chaps auth [OPTIONS] <COMMAND>
```

Subcommands: [`chaps auth show`](#chaps-auth-show), [`chaps auth token`](#chaps-auth-token), [`chaps auth enable`](#chaps-auth-enable), [`chaps auth disable`](#chaps-auth-disable), [`chaps auth rotate`](#chaps-auth-rotate)

## chaps auth show

Say whether the API is protected, and by which token.

```text
Usage: chaps auth show [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--reveal` | Print the API token in full. |

## chaps auth token

Print the API token alone, for a script to capture.

```text
Usage: chaps auth token [OPTIONS]
```

## chaps auth enable

Turn authentication on: write both secrets and sync.

```text
Usage: chaps auth enable [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--token <TOKEN>` | Use this API token instead of generating one. |

## chaps auth disable

Turn authentication off, keeping both values as comments.

```text
Usage: chaps auth disable [OPTIONS]
```

## chaps auth rotate

Replace both secrets with freshly generated ones.

```text
Usage: chaps auth rotate [OPTIONS]
```

## chaps self

Update chaps itself, and report what this build is.

```text
Usage: chaps self [OPTIONS] <COMMAND>
```

Subcommands: [`chaps self update`](#chaps-self-update), [`chaps self version`](#chaps-self-version)

## chaps self update

Replace this binary with the newest release.

```text
Usage: chaps self update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Report what an update would do and change nothing. |
| `--version <TAG>` | Install this release tag instead of the newest one. |
| `-y, --yes` | Do not ask before replacing the binary. |

## chaps self version

Show what this build is: version, revision, target and path.

```text
Usage: chaps self version [OPTIONS]
```

## chaps completions

Print a shell completion script for chaps.

```text
Usage: chaps completions [OPTIONS] <SHELL>
```

| Argument | Description |
| --- | --- |
| `<SHELL>` | Shell to generate for. Values: `bash`, `elvish`, `fish`, `powershell`, `zsh`. |
