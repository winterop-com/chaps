# Command reference

Generated from the `--help` texts by `make docs-reference`; edit
`src/cli.rs` and run that target rather than editing this file.

Every command accepts the options listed under [varde](#varde), `--json`
included. The registry options (`--registry-url`, `--offline`, `--cache-dir`)
show only in the help of the commands that use them, and each such command
names them below. Commands that need a deployment directory are hidden from
`varde --help` outside one, but they are all listed here.

## varde

deploy Chap (climate-informed disease forecasting) and its services.

```text
Usage: varde [OPTIONS] <COMMAND>
```

| Global option | Description |
| --- | --- |
| `--json` | Print machine-readable JSON instead of human output. |
| `--no-color` | Never colour the output (NO_COLOR does the same). |
| `-v, --verbose` | Show the hints; -vv also shows the commands and requests that run. |
| `-d, --debug` | -vv plus the raw responses. |
| `-C, --project-dir <DIR>` | Deployment directory, or any directory inside one. Default: `.`. |
| `--registry-url <URL>` | URL of the marketplace registry index. Default: `https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml`. |
| `--offline` | Never touch the network; use what is cached or on this machine. |
| `--cache-dir <DIR>` | Cache directory: the registry snapshot and the self-update downloads. |

Subcommands: [`varde init`](#varde-init), [`varde run`](#varde-run), [`varde ps`](#varde-ps), [`varde stop`](#varde-stop), [`varde top`](#varde-top), [`varde models`](#varde-models), [`varde components`](#varde-components), [`varde ui`](#varde-ui), [`varde registry`](#varde-registry), [`varde sync`](#varde-sync), [`varde update`](#varde-update), [`varde up`](#varde-up), [`varde down`](#varde-down), [`varde logs`](#varde-logs), [`varde restart`](#varde-restart), [`varde docker`](#varde-docker), [`varde backup`](#varde-backup), [`varde status`](#varde-status), [`varde open`](#varde-open), [`varde jobs`](#varde-jobs), [`varde api`](#varde-api), [`varde chap`](#varde-chap), [`varde doctor`](#varde-doctor), [`varde cleanup`](#varde-cleanup), [`varde auth`](#varde-auth), [`varde dhis2`](#varde-dhis2), [`varde self`](#varde-self), [`varde completions`](#varde-completions)

## varde init

Create a deployment directory: compose files, .env and .varde/.

```text
Usage: varde init [OPTIONS] [DIR]
```

| Argument | Description |
| --- | --- |
| `<DIR>` | Directory to create. Default: `.`. |
| `--models <SPEC>` | Models to enable: none, default, all, or a list of ids. Default: `default`. |
| `--interactive` | Select the models in the browser instead of with --models. |
| `--chap-tag <TAG>` | chap-core image tag: latest, master, dev or vX.Y.Z. Default: `latest`. |
| `--force` | Overwrite an existing deployment in the target directory. |
| `--api-port <PORT>` | Host port to publish chap-core's API on. Default: `8700`. |
| `--port-base <PORT>` | Lowest host port a model may be published on. Default: `5001`. |
| `--api-token <TOKEN>` | Protect the API with a token (generated when no value is given). |
| `--no-env` | Do not write a .env file. |
| `--fresh-env` | Regenerate .env with a new database password. |
| `--source <PATH>` | Build chap-core from the chap-core checkout at PATH. |
| `--with <LIST>` | Components to add: ocs, s3, dhis2. |
| `--without <LIST>` | Components to omit: chap-core, ocs, s3, dhis2. |
| `--only <LIST>` | Exactly these components and nothing else; `none` for models alone. |
| `--chap-core-url <URL>` | Register the models with the chap-core at URL instead of running one. |
| `--ocs-base-url <URL>` | Public URL OCS is reached at, for a proxied instance. |
| `--ocs-port <PORT\|none>` | Host port to publish OCS on, or none to keep it internal. |
| `--s3-port <PORT\|none>` | Host port to publish the object store on, or none. |
| `--dhis2-port <PORT\|none>` | Host port to publish DHIS2 on, or none to keep it internal. |
| `--dhis2-seed <SPEC>` | Dump to seed the DHIS2 database from: default, none, URL or path. |
| `--dhis2-seed-password <PW>` | Give every DHIS2 user this password (default: district). |
| `--dhis2-tag <TAG>` | DHIS2 version to run: an image tag such as 2.41, 2.42 or 2.43.1. |
| `--dhis2-image <REPO>` | DHIS2 image repository, e.g. dhis2/core-dev for unreleased versions. |
| `--ocs-read-only` | Refuse ingestion over HTTP on the new OCS instance. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. Malawi. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent. |
| `--ocs-bbox <BBOX>` | OCS extent as xmin,ymin,xmax,ymax in degrees. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde run

Start one model and print where it answers.

```text
Usage: varde run [OPTIONS] <MODEL>
```

| Argument | Description |
| --- | --- |
| `<MODEL>` | Marketplace id, GitHub repository URL, ghcr image, or local image:tag. |
| `--port <PORT\|auto>` | Host port to publish the model on, or auto for a free one. Default: `auto`. |
| `--bind <ADDR>` | Host address to publish the port on, e.g. 0.0.0.0 for the network. |
| `--id <ID\|auto>` | Identifier to record an added model under, or auto for a free one. |
| `--group <NAME>` | Group to start it in, outside a deployment; ps and stop take it too. |
| `--allow-template` | Start a marketplace template, which is not a deployable model. |
| `--no-wait` | Return once the container started, without waiting for it to answer. |
| `-a, --attach, --foreground` | Run in the foreground and stream its log (Ctrl-C stops the model). |
| `--rm` | With --attach, Ctrl-C also removes the model's data volume. |
| `--chap-core <URL>` | A chap-core elsewhere for the group's models to register with. |
| `--models-host <HOST>` | Host that chap-core calls the models back at; detected if omitted. |
| `--timeout <SECONDS>` | How long to wait for the model to answer, in seconds. Default: `300`. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde ps

List the models that run, and where each one answers.

```text
Usage: varde ps [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--group <NAME>` | List this group only; every group when omitted. |

## varde stop

Stop a model and remove its compose overlay; its data stays.

```text
Usage: varde stop [OPTIONS] [ID]
```

| Argument | Description |
| --- | --- |
| `<ID>` | Id or service id of the model. |
| `--group <NAME>` | Group to stop in; every model in it when no id is given. |
| `--all` | Stop every model in the group, or in every group. |
| `--purge` | Delete the data volumes too, and a group left empty with them. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde top

Watch every varde deployment on this machine as a live tree.

```text
Usage: varde top [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--interval <SECONDS>` | Seconds between two looks at docker. Default: `2`. |

## varde models

Browse and manage marketplace models.

```text
Usage: varde models [OPTIONS] <COMMAND>
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

Subcommands: [`varde models list`](#varde-models-list), [`varde models search`](#varde-models-search), [`varde models info`](#varde-models-info), [`varde models test`](#varde-models-test), [`varde models configs`](#varde-models-configs), [`varde models add`](#varde-models-add), [`varde models remove`](#varde-models-remove), [`varde models enable`](#varde-models-enable), [`varde models disable`](#varde-models-disable), [`varde models expose`](#varde-models-expose), [`varde models unexpose`](#varde-models-unexpose)

## varde models list

List marketplace models.

```text
Usage: varde models list [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--all` | Include every entry, templates as well as models. |
| `--templates` | List only templates. |
| `--enabled` | List only the models enabled in this deployment. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models search

Search the marketplace by id, name or summary.

```text
Usage: varde models search [OPTIONS] <QUERY>
```

| Argument | Description |
| --- | --- |
| `<QUERY>` | Text to look for; matching is case-insensitive. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models info

Show everything known about one model.

```text
Usage: varde models info [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models test

Make a model train and predict, and say whether it could.

```text
Usage: varde models test [OPTIONS] [ID]...
```

| Argument | Description |
| --- | --- |
| `<ID>...` | Marketplace ids or service ids of the models to test. |
| `--all` | Test every model this deployment has enabled. |
| `--backtest` | Run a backtest through chap-core instead of the model's own test. |
| `--seed <N>` | Seed for the generated data, so a run can be repeated. |
| `--timeout <SECONDS>` | Time limit for the test of one model, in seconds. |
| `--keep` | Keep what the test created instead of deleting it. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models configs

List, add, archive and sync the configured models of each model.

```text
Usage: varde models configs [OPTIONS] [ID]
       varde models configs <COMMAND>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id; all enabled models when none is given. |
| `--all` | Show the archived configured models too. |

Subcommands: [`varde models configs list`](#varde-models-configs-list), [`varde models configs add`](#varde-models-configs-add), [`varde models configs archive`](#varde-models-configs-archive), [`varde models configs sync`](#varde-models-configs-sync)

## varde models configs list

List the configured models chap-core has of each enabled model.

```text
Usage: varde models configs list [OPTIONS] [ID]
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id; all enabled models when none is given. |
| `--all` | Show the archived configured models too. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models configs add

Add a configured model of a model to chap-core.

```text
Usage: varde models configs add [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--name <NAME>` | Variant name, which the Modeling App shows in brackets. |
| `--set <KEY=VALUE>` | Value of one user option; repeat for more than one. |
| `--covariates <A,B>` | Additional covariates, separated by commas. |
| `-i, --interactive` | Ask for the name, each option and the covariates. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models configs archive

Archive a configured model of a model.

```text
Usage: varde models configs archive [OPTIONS] <ID> <NAME>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `<NAME>` | Variant name of the configured model. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models configs sync

Create the configured models that chap-core needs to run each model.

```text
Usage: varde models configs sync [OPTIONS] [ID]...
```

| Argument | Description |
| --- | --- |
| `<ID>...` | Marketplace ids or service ids; all enabled models when none is given. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models add

Add a model the marketplace does not list.

```text
Usage: varde models add [OPTIONS] <SOURCE>
```

| Argument | Description |
| --- | --- |
| `<SOURCE>` | GitHub repository URL, ghcr image reference, or local image name:tag. |
| `--id <ID\|auto>` | Identifier to record the model under, or auto for a free one. |
| `--service-id <ID>` | Compose service name, which must match the service's own id. |
| `--name <NAME>` | Name to show in the listings. |
| `--port <PORT\|auto>` | Host port to publish the model on, or auto for a free one. |
| `--bind <ADDR>` | Host address to publish the port on, e.g. 127.0.0.1 for this machine. |
| `--data-dir <PATH>` | Data directory inside the container. |
| `--user <USER>` | User the container runs as, as user:group. |
| `--runtime-amd64` | Record the image as published for amd64 only. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models remove

Remove a model that was added with models add.

```text
Usage: varde models remove [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Id of a model added with models add. |
| `--purge` | Delete the model's data volume as well. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models enable

Enable a model and write its compose overlay.

```text
Usage: varde models enable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--channel <CHANNEL>` | Follow a release channel instead of pinning a version. Values: `stable`, `latest`. |
| `--version <VERSION>` | Pin an exact version from the model's versions list. |
| `--port <PORT\|auto>` | Host port to publish the model on, or auto for a free one. |
| `--bind <ADDR>` | Host address to publish the port on, e.g. 127.0.0.1 for this machine. |
| `--data-dir <PATH>` | Data directory inside the container. |
| `--user <USER>` | User the container runs as, as user:group. |
| `--allow-template` | Enable a template even though it is not a model. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models disable

Disable a model and remove its compose overlay.

```text
Usage: varde models disable [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--purge` | Delete the model's data volume as well. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models expose

Publish a host port for an enabled model.

```text
Usage: varde models expose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--port <PORT\|auto>` | Host port to publish on, or auto for the lowest free one. |
| `--bind <ADDR>` | Host address to publish the port on, e.g. 127.0.0.1 for this machine. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde models unexpose

Remove the host port of an enabled model.

```text
Usage: varde models unexpose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde components

Show and change what this deployment is made of.

```text
Usage: varde components [OPTIONS] <COMMAND>
```

Subcommands: [`varde components list`](#varde-components-list), [`varde components enable`](#varde-components-enable), [`varde components disable`](#varde-components-disable)

## varde components list

List every component and whether this deployment has it.

```text
Usage: varde components list [OPTIONS]
```

## varde components enable

Enable a component, or change the settings of an enabled one.

```text
Usage: varde components enable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: ocs, s3, dhis2 or chap-core. |
| `--port <PORT\|none>` | Host port to publish on, or none to keep it internal. |
| `--base-url <URL>` | Public URL OCS is reached at, for a proxied instance. |
| `--read-only` | Refuse ingestion over HTTP on this OCS instance. |
| `--read-write` | Allow ingestion over HTTP again. |
| `--url <URL>` | chap-core only: use the chap-core at URL instead of running one. |
| `--tag <TAG>` | dhis2 only: the DHIS2 version to run, an image tag such as 2.43. |
| `--image <REPO>` | dhis2 only: the image repository, e.g. dhis2/core-dev. |
| `--seed <SPEC>` | dhis2 only: dump for a new database: default, none, URL or path. |
| `--models-host <HOST>` | With --url: the host that chap-core calls the models back at. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. Malawi. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent. |
| `--ocs-bbox <BBOX>` | OCS extent as xmin,ymin,xmax,ymax in degrees. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde components disable

Disable a component and remove its compose file.

```text
Usage: varde components disable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: ocs, s3, dhis2 or chap-core. |
| `--purge` | Delete the component's data volume as well. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde ui

Open the browser: marketplace models and components.

```text
Usage: varde ui [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde registry

Inspect and refresh the marketplace registry.

```text
Usage: varde registry [OPTIONS] <COMMAND>
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

Subcommands: [`varde registry update`](#varde-registry-update), [`varde registry show`](#varde-registry-show)

## varde registry update

Fetch the registry from the network and refresh the cache.

```text
Usage: varde registry update [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde registry show

Show where the registry was loaded from and what it holds.

```text
Usage: varde registry show [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde sync

Render the compose files from .varde/.

```text
Usage: varde sync [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Write nothing; exit non-zero if anything would change. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde update

Move the pins to what upstream publishes now, and pull.

```text
Usage: varde update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--dry-run` | Show what would change: no pull, and nothing written. |
| `--pin-chap-core` | Pin a moving chap-core tag to the newest release. |
| `--chap-tag <TAG>` | Move chap-core to this tag: vX.Y.Z, latest, master or dev. |
| `--list-tags` | List the chap-core tags you can move to, newest first. |
| `--yes` | Answer yes to the confirmation a backwards move asks for. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde up

Sync, then start Chap (docker compose up).

```text
Usage: varde up [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `-a, --attach, --foreground` | Run in the foreground and stream all logs (Ctrl-C stops Chap). |
| `--pull` | Pull every image first (docker compose up --pull always). |
| `--no-preflight` | Do not check the host ports first. |
| `--replace` | First stop the other varde deployments that hold these ports. |
| `--wait` | Return only once chap-core, the models, OCS and DHIS2 answer. |
| `--timeout <SECONDS>` | How long --wait waits before it fails, in seconds. Default: `300`. |
| `<EXTRA>...` | Extra arguments passed through to docker compose up. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde down

Stop Chap (docker compose down).

```text
Usage: varde down [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `--volumes` | Also remove this deployment's volumes, destroying its data. |
| `-y, --yes` | Skip the confirmation. |
| `<EXTRA>...` | Extra arguments passed through to docker compose down. |

## varde logs

Show container logs (docker compose logs).

```text
Usage: varde logs [OPTIONS] [SERVICE]...
```

| Argument | Description |
| --- | --- |
| `-f, --follow` | Show new output when it arrives. |
| `--tail <N>` | Show only the last N lines of each service's log. |
| `<SERVICE>...` | Services to show logs for; all of them when omitted. |

## varde restart

Recreate the services whose image or configuration changed.

```text
Usage: varde restart [OPTIONS] [SERVICE]...
```

| Argument | Description |
| --- | --- |
| `--all` | Recreate the named services even when nothing changed. |
| `<SERVICE>...` | Services to restart; the whole deployment when omitted. |

## varde docker

Use Docker directly: containers, images, compose.

```text
Usage: varde docker [OPTIONS] <COMMAND>
```

Subcommands: [`varde docker ps`](#varde-docker-ps), [`varde docker pull`](#varde-docker-pull), [`varde docker exec`](#varde-docker-exec), [`varde docker run`](#varde-docker-run), [`varde docker config`](#varde-docker-config)

## varde docker ps

List the containers this deployment is running.

```text
Usage: varde docker ps [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose ps. |

## varde docker pull

Download the pinned images into the local Docker daemon.

```text
Usage: varde docker pull [OPTIONS]
```

## varde docker exec

Run a command inside one of the running containers.

```text
Usage: varde docker exec [OPTIONS] <SERVICE> [CMD]...
```

| Argument | Description |
| --- | --- |
| `<SERVICE>` | Service to run the command in. |
| `<CMD>...` | Command to run; a shell (sh) when omitted. |

## varde docker run

Run any docker compose command against this deployment.

```text
Usage: varde docker run [OPTIONS] [ARGS]...
```

| Argument | Description |
| --- | --- |
| `<ARGS>...` | Arguments passed to docker compose after the -f list. |

## varde docker config

Print every compose file merged into one document.

```text
Usage: varde docker config [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose config. |

## varde backup

Create or restore a backup of this deployment.

```text
Usage: varde backup [OPTIONS] <COMMAND>
```

Subcommands: [`varde backup create`](#varde-backup-create), [`varde backup restore`](#varde-backup-restore)

## varde backup create

Write a tar.gz of the database, the data and the files.

```text
Usage: varde backup create [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--out <PATH>` | Where to write the archive: a file, or a directory. |
| `--no-db` | Do not put the chap-core database in the archive. |
| `--no-models` | Do not put the model data volumes in the archive. |
| `--no-components` | Do not put the component data volumes in the archive. |

## varde backup restore

Put a deployment back from a backup archive.

```text
Usage: varde backup restore [OPTIONS] <ARCHIVE>
```

| Argument | Description |
| --- | --- |
| `<ARCHIVE>` | The tar.gz written by varde backup create. |
| `--yes` | Skip the confirmation. |
| `--files-only` | Restore only the deployment files; no Docker needed. |
| `--db-only` | Restore only the chap-core database. |
| `--no-models` | Leave the model data volumes as they are. |
| `--no-components` | Leave the component data volumes as they are. |
| `--adopt-identity` | Use the compose project name from the archive. |
| `--no-start` | Do not start the deployment at the end. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde status

Show chap-core health and which models registered.

```text
Usage: varde status [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--url <URL>` | Base URL of the chap-core API. |
| `--timeout <SECONDS>` | Request timeout in seconds. Default: `5`. |

## varde open

Open a component's web interface in a browser.

```text
Usage: varde open [OPTIONS] [NAME]
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component to open (chap-core, ocs, dhis2), or an enabled model's id. |
| `--no-browser` | Print the address instead of opening a browser. |

## varde jobs

List the backtests and predictions chap-core has run.

```text
Usage: varde jobs [OPTIONS]
       varde jobs <COMMAND>
```

| Argument | Description |
| --- | --- |
| `--status <STATUS>` | Only jobs in this status; repeat for more than one. |
| `--type <TYPE>` | Only jobs of this type, such as create_backtest. |
| `--limit <N>` | Show at most this many jobs. |

Subcommands: [`varde jobs list`](#varde-jobs-list), [`varde jobs show`](#varde-jobs-show), [`varde jobs logs`](#varde-jobs-logs), [`varde jobs cancel`](#varde-jobs-cancel), [`varde jobs delete`](#varde-jobs-delete)

## varde jobs list

List the jobs chap-core knows about, newest first.

```text
Usage: varde jobs list [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--status <STATUS>` | Only jobs in this status; repeat for more than one. |
| `--type <TYPE>` | Only jobs of this type, such as create_backtest. |
| `--limit <N>` | Show at most this many jobs. |

## varde jobs show

Show everything chap-core records about one job.

```text
Usage: varde jobs show [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## varde jobs logs

Print one job's log, which is where a failure says why.

```text
Usage: varde jobs logs [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |
| `--tail <N>` | Print only the last N lines of the log. |

## varde jobs cancel

Ask chap-core to stop a job that is still running.

```text
Usage: varde jobs cancel [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## varde jobs delete

Remove a finished job from chap-core's list.

```text
Usage: varde jobs delete [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Job id, or enough of its start to name one job. |

## varde api

Send one authenticated request to chap-core's API.

```text
Usage: varde api [OPTIONS] <METHOD> <PATH>
```

| Argument | Description |
| --- | --- |
| `<METHOD>` | GET, POST, PUT, PATCH or DELETE; case does not matter. |
| `<PATH>` | Request path starting with /, query string and all. |
| `--data <JSON\|@FILE\|->` | JSON body: inline, @file, or - to read stdin. |
| `--url <URL>` | Base URL of the chap-core API. |
| `--raw` | Print the body exactly as it arrived. |
| `--timeout <SECONDS>` | Request timeout in seconds. Default: `30`. |

## varde chap

Run the chap CLI in a container, with no Python or uv installed.

```text
Usage: varde chap [OPTIONS] [CHAP_ARGS]...
```

| Argument | Description |
| --- | --- |
| `--tag <TAG>` | chap-core image tag; the deployment's tag or the newest release if omitted. |
| `--image <IMAGE>` | Image to run in; chosen from --model-name when omitted. Values: `core`, `worker`. |
| `--group <NAME>` | `varde run` group whose network the container joins. |
| `--docker` | Give the container the docker socket, for docker_env models. |
| `--stop` | Stop the model this run started, once chap is done. |
| `--timeout <SECONDS>` | How long a model this run starts may take to answer, in seconds. Default: `300`. |
| `<CHAP_ARGS>...` | Arguments for chap, such as `eval --model-name URL ...`. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde doctor

Run a checklist over this machine and this deployment.

```text
Usage: varde doctor [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde cleanup

Delete what deployments whose directory is gone left in docker.

```text
Usage: varde cleanup [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `-n, --dry-run` | List what would be deleted and change nothing. |
| `--yes` | Delete without asking first. |

## varde auth

Enable or disable API authentication, and show the token.

```text
Usage: varde auth [OPTIONS] <COMMAND>
```

Subcommands: [`varde auth show`](#varde-auth-show), [`varde auth token`](#varde-auth-token), [`varde auth enable`](#varde-auth-enable), [`varde auth disable`](#varde-auth-disable), [`varde auth rotate`](#varde-auth-rotate)

## varde auth show

Say whether the API is protected, and by which token.

```text
Usage: varde auth show [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--reveal` | Print the API token. |

## varde auth token

Print the API token alone, for a script to capture.

```text
Usage: varde auth token [OPTIONS]
```

## varde auth enable

Enable authentication: write both secrets and sync.

```text
Usage: varde auth enable [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--token <TOKEN>` | Use this API token instead of generating one. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde auth disable

Disable authentication; keep both values as comments.

```text
Usage: varde auth disable [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde auth rotate

Replace both secrets with new generated ones.

```text
Usage: varde auth rotate [OPTIONS]
```

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde dhis2

Let the DHIS2 Modeling App reach this deployment's Chap.

```text
Usage: varde dhis2 [OPTIONS] <COMMAND>
```

Subcommands: [`varde dhis2 show`](#varde-dhis2-show), [`varde dhis2 route`](#varde-dhis2-route), [`varde dhis2 analytics`](#varde-dhis2-analytics), [`varde dhis2 apps`](#varde-dhis2-apps), [`varde dhis2 connect`](#varde-dhis2-connect), [`varde dhis2 use`](#varde-dhis2-use)

## varde dhis2 show

Say what DHIS2 has: the chap route, analytics and the apps.

```text
Usage: varde dhis2 show [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--user <NAME>` | DHIS2 user to authenticate as, with a password rather than a token. |
| `--wait <SECONDS>` | Seconds to wait for DHIS2's API; 1200 local, 60 external by default. |

## varde dhis2 route

Point DHIS2's chap route at this deployment's chap-core.

```text
Usage: varde dhis2 route [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--user <NAME>` | DHIS2 user to authenticate as, with a password rather than a token. |
| `--wait <SECONDS>` | Seconds to wait for DHIS2's API; 1200 local, 60 external by default. |

## varde dhis2 analytics

Generate DHIS2's analytics tables and wait for them.

```text
Usage: varde dhis2 analytics [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--user <NAME>` | DHIS2 user to authenticate as, with a password rather than a token. |
| `--wait <SECONDS>` | Seconds to wait for DHIS2's API; 1200 local, 60 external by default. |
| `--timeout <SECONDS>` | Seconds to wait for the analytics run to finish. Default: `3600`. |
| `--no-wait` | Start the run and return without a wait for it to finish. |

## varde dhis2 apps

Install the Modeling and Climate apps from the App Hub.

```text
Usage: varde dhis2 apps [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--user <NAME>` | DHIS2 user to authenticate as, with a password rather than a token. |
| `--wait <SECONDS>` | Seconds to wait for DHIS2's API; 1200 local, 60 external by default. |

Registry options: `--offline` (see [varde](#varde)).

## varde dhis2 connect

The route, the apps, then analytics; the route only on an external DHIS2.

```text
Usage: varde dhis2 connect [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--user <NAME>` | DHIS2 user to authenticate as, with a password rather than a token. |
| `--wait <SECONDS>` | Seconds to wait for DHIS2's API; 1200 local, 60 external by default. |
| `--timeout <SECONDS>` | Seconds to wait for the analytics run to finish. Default: `3600`. |
| `--no-wait` | Start the run and return without a wait for it to finish. |

Registry options: `--registry-url <URL>`, `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde dhis2 use

Use a DHIS2 that runs elsewhere, or show which DHIS2 is used.

```text
Usage: varde dhis2 use [OPTIONS] [URL]
```

| Argument | Description |
| --- | --- |
| `<URL>` | DHIS2's URL, as this machine reaches it. |
| `--chap-url <URL>` | chap-core's URL, as that DHIS2 reaches it; the route points here. |
| `--clear` | Forget the external DHIS2 and use the dhis2 component again. |

## varde self

Update varde itself, and report what this build is.

```text
Usage: varde self [OPTIONS] <COMMAND>
```

Subcommands: [`varde self update`](#varde-self-update), [`varde self version`](#varde-self-version)

## varde self update

Replace this binary with the newest release.

```text
Usage: varde self update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Report what an update would do and change nothing. |
| `--version <TAG>` | Install this release tag instead of the newest one. |
| `-y, --yes` | Do not ask before replacing the binary. |

Registry options: `--offline`, `--cache-dir <DIR>` (see [varde](#varde)).

## varde self version

Show what this build is: version, revision, target and path.

```text
Usage: varde self version [OPTIONS]
```

## varde completions

Print a shell completion script for varde.

```text
Usage: varde completions [OPTIONS] <SHELL>
```

| Argument | Description |
| --- | --- |
| `<SHELL>` | Shell to generate for. Values: `bash`, `elvish`, `fish`, `powershell`, `zsh`. |
