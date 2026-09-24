# Command reference

Generated from the `--help` texts by `make docs-reference`; edit
`src/cli.rs` and run that target rather than editing this file.

Every command accepts the global options listed under [chaps](#chaps),
`--json` included. Commands that need a deployment directory are hidden from
`chaps --help` outside one, but they are all listed here.

## chaps

chaps deploys and manages CHAP, the Climate Health Analytics Platform.

It runs chap-core, its worker and database with Docker Compose, and adds
forecasting models from the CHAP model marketplace as pinned overlays.

chap or chaps? `chap` is chap-core's own developer CLI, the one that serves the
API and runs evaluations from a checkout; `chaps` is this tool, which deploys
and manages CHAP on a machine.
Start with `chaps init`. Docs: https://chap.dhis2.org

```text
Usage: chaps [OPTIONS] <COMMAND>
```

| Global option | Description |
| --- | --- |
| `--json` | Emit machine-readable JSON instead of human output. |
| `--no-color` | Never colour the output (NO_COLOR in the environment does the same). |
| `-v, --verbose` | Narrate on stderr what runs: external commands, HTTP requests, the registry source, the files sync compared. |
| `-d, --debug` | Everything --verbose says, plus response bodies, the raw compose ps output and the resolved project paths. Implies --verbose. |
| `-C, --project-dir <DIR>` | Project directory (or any directory inside one); found like git finds .git. Default: `.`. |
| `--registry-url <URL>` | URL of the marketplace registry index. Default: `https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml`. |
| `--offline` | Never touch the network; use the cache or the embedded snapshot. |
| `--cache-dir <DIR>` | Directory for the cached registry snapshot. |

Subcommands: [`chaps init`](#chaps-init), [`chaps models`](#chaps-models), [`chaps components`](#chaps-components), [`chaps ui`](#chaps-ui), [`chaps registry`](#chaps-registry), [`chaps sync`](#chaps-sync), [`chaps update`](#chaps-update), [`chaps up`](#chaps-up), [`chaps down`](#chaps-down), [`chaps logs`](#chaps-logs), [`chaps restart`](#chaps-restart), [`chaps docker`](#chaps-docker), [`chaps backup`](#chaps-backup), [`chaps status`](#chaps-status), [`chaps doctor`](#chaps-doctor), [`chaps auth`](#chaps-auth), [`chaps self`](#chaps-self), [`chaps completions`](#chaps-completions)

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
| `--with <LIST>` | Optional components to add, as a comma-separated list: `ocs` for Open Climate Service beside chap-core, `s3` for the object store OCS will keep its objects in. `chaps components` changes them afterwards. |
| `--without <LIST>` | Components to leave out, as a comma-separated list. Only `chap-core` is on by default, so `--without chap-core` is the standalone case: a deployment of the other components alone. Models need chap-core. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. `Malawi`. It names the extent and, with " Climate Service" after it, the instance. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent, e.g. `MWI`. |
| `--ocs-bbox <BBOX>` | Bounding box of the OCS extent as `xmin,ymin,xmax,ymax` in degrees. |

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

The model gets no host port: chap-core reaches it over the compose network and a human through `/v2/services/<id>/run/`. `--port` opts in to one.

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

The model keeps the version it is pinned to: this only rewrites its overlay's `ports:`, so it is safe on a deployment that is running a build you do not want moved. Run `chaps up` afterwards to apply it.

```text
Usage: chaps models expose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |
| `--port <PORT\|auto>` | Host port to publish on; `auto` (the default) takes the lowest free one. |

## chaps models unexpose

Take an enabled model's host port away again.

It stays registered with chap-core and reachable at `/v2/services/<id>/run/`; only the host mapping goes. Run `chaps up` afterwards to apply it.

```text
Usage: chaps models unexpose [OPTIONS] <ID>
```

| Argument | Description |
| --- | --- |
| `<ID>` | Marketplace id or service id. |

## chaps components

Show and change what this deployment is made of.

A component is a service (or a small group of them) that `chaps sync` renders a compose file for: `chap-core`, which is CHAP itself and is on unless it was turned off, `ocs`, which is Open Climate Service beside it, and `s3`, the object store OCS will keep its objects in. The set lives in `.chaps/components.yaml`; enabling or disabling one syncs the compose files straight away, and `chaps up` applies them.

Enabling `ocs` also scaffolds `ocs/climate-service.yaml`, the OCS instance configuration: which country or region this instance covers, and where it keeps its data. That file is yours from the moment it exists - `chaps` never rewrites it - and `chaps doctor` warns while it still holds OCS's example values.

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

Turn a component on: record it in .chaps/components.yaml and sync.

Enabling one that is already on is how its settings are changed: `--port` moves the host port it publishes. `ocs` is published on 9000 by default because it serves a web interface; `s3` publishes nothing, since OCS reaches it over the compose network.

```text
Usage: chaps components enable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: `ocs`, `s3` or `chap-core`. |
| `--port <PORT>` | Host port to publish this component on. |
| `--ocs-name <NAME>` | Country or region the OCS instance covers, e.g. `Malawi`. It names the extent and, with " Climate Service" after it, the instance. |
| `--ocs-country <CODE>` | ISO 3166-1 alpha-3 country code for the OCS extent, e.g. `MWI`. |
| `--ocs-bbox <BBOX>` | Bounding box of the OCS extent as `xmin,ymin,xmax,ymax` in degrees. |

## chaps components disable

Turn a component off: drop it from .chaps/components.yaml, remove its compose file and sync.

Turning `chap-core` off is refused while any model is enabled: model services register with chap-core and are reached through it.

```text
Usage: chaps components disable [OPTIONS] <NAME>
```

| Argument | Description |
| --- | --- |
| `<NAME>` | Component name: `ocs`, `s3` or `chap-core`. |

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

Writes every enabled model's overlay and compose.marketplace.yml, removes overlays of models that are no longer enabled, and appends missing image pin comments to .env. Files that already match are left alone.

```text
Usage: chaps sync [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Write nothing; exit non-zero if anything would change. |

## chaps update

Move the model and chap-core pins to what upstream publishes now, pull the images and say what needs restarting.

After updating .chaps/models.yaml and the .env pin comments it runs `chaps sync` and `docker compose pull`. A chap-core pin on a release tag moves to the newest release, compose file included; a moving tag (`latest`, `master`, `dev`) is only refreshed by the pull.

Always fetches the registry from the network (no cache, no fallback). Models pinned to an exact version are listed but not moved. The run reads in that order: the plan, the pull, and one line saying what moved.

It never touches a container. `chaps restart` applies what it fetched to the services that are running, and `chaps up` starts a deployment that is not.

```text
Usage: chaps update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--dry-run` | Show what would change: no pull, and nothing written. |
| `--pin-chap-core` | Turn a moving chap-core tag (`latest`, `master`, `dev`) into a pin on the newest release, the way `chaps init` does by default. |

## chaps up

Sync, then start CHAP (docker compose up).

Before invoking Docker it checks that the host ports the stack publishes are free, and says what to do about any that are not; `--no-preflight` skips that.

```text
Usage: chaps up [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `-a, --attach, --foreground` | Run in the foreground and stream all logs (Ctrl-C stops CHAP). |
| `--pull` | Pull every image first, including a moving chap-core tag such as `latest` (docker compose up --pull always). |
| `--no-preflight` | Do not check the host ports first; let Docker report a conflict. |
| `<EXTRA>...` | Extra arguments passed through to docker compose up. |

## chaps down

Stop CHAP (docker compose down).

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

## chaps restart

Recreate the running services whose image or configuration changed.

This is the second half of `chaps update`: the update moves the pins and pulls the images, and this applies them to what is running. It is `docker compose up -d`, which recreates only the containers that no longer match the files, so a service nothing changed for is left alone and its uptime with it. `--all` recreates the named services anyway, which is what a model that is running but never registered needs.

It changes no file, no pin and nothing in `.chaps/`, and it never syncs: it starts what is already on disk. A deployment that is not running at all is `chaps up`'s to start.

```text
Usage: chaps restart [OPTIONS] [SERVICE]...
```

| Argument | Description |
| --- | --- |
| `--all` | Recreate the named services even when nothing about them changed (docker compose up --force-recreate). This is what a model that is running but never registered needs. |
| `<SERVICE>...` | Services to restart; the whole project when omitted. Named services are recreated on their own, without their dependencies. |

## chaps docker

Talk to Docker directly: containers, images, raw compose commands.

The everyday verbs (`up`, `down`, `logs`) are at the top level; this group holds the plumbing you only reach for when you already know what Docker is doing.

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

The container has to be up already; start the stack with `chaps up` first. Without a CMD you get a shell. When this command's own input is not a terminal - in a script, or behind a pipe - `-T` is passed to compose so it does not try to allocate one.

```text
Usage: chaps docker exec [OPTIONS] <SERVICE> [CMD]...
```

| Argument | Description |
| --- | --- |
| `<SERVICE>` | Service to run the command in, as named in the compose files. |
| `<CMD>...` | Command and arguments to run; a shell (`sh`) when omitted. |

## chaps docker run

Run any docker compose command against this project's files.

Everything after `run` is handed to `docker compose` unchanged, behind the project's `-f` list: `chaps docker run -- restart chap` becomes `docker compose -f ... -f ... restart chap`.

```text
Usage: chaps docker run [OPTIONS] [ARGS]...
```

| Argument | Description |
| --- | --- |
| `<ARGS>...` | Arguments passed to docker compose after the project's -f list. |

## chaps docker config

Print the finished configuration: every compose file merged into one document.

This is what Docker actually reads after the `-f` list, the `include:` and the `.env` substitutions have been applied - useful when a setting is not taking effect. With `--json` the document is printed as JSON.

```text
Usage: chaps docker config [OPTIONS] [EXTRA]...
```

| Argument | Description |
| --- | --- |
| `<EXTRA>...` | Extra arguments passed through to docker compose config. |

## chaps backup

Create or restore a backup of the database, model data and project files.

One archive holds everything a deployment is: `.env`, `.chaps/`, the compose files, a `pg_dump` of the chap-core database and one tar per model data volume. It is a plain `tar.gz` - `tar -tzf` lists it, and the docs say which raw `pg_restore` and `tar` commands put it back without `chaps`.

```text
Usage: chaps backup [OPTIONS] <COMMAND>
```

Subcommands: [`chaps backup create`](#chaps-backup-create), [`chaps backup restore`](#chaps-backup-restore)

## chaps backup create

Write a tar.gz of the database, the model data, the component data and the project files.

The database part needs a running postgres (`chaps up`); each model's data is read straight from its volume by the overlay's one-shot init container, so it works whether or not the model itself is running. A model that has never started has no volume yet and is skipped with a warning. A service that is running is paused for the seconds its volume takes to read, so nothing writes into a half-read tar.

```text
Usage: chaps backup create [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--out <PATH>` | Where to write the archive: a file, or a directory to name it in. Default: `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` in the current directory. |
| `--no-db` | Leave the chap-core database out of the archive. |
| `--no-models` | Leave the model data volumes out of the archive. |
| `--no-components` | Leave the component data volumes (ocs, s3) out of the archive. |

## chaps backup restore

Put a deployment back from an archive `chaps backup create` wrote.

Prints what it is about to overwrite and asks before touching anything. Then: stop `chap`, `worker` and the model services, write the files back and sync, `pg_restore --clean` the database, refill each model and component data volume, and start the stack again.

The deployment keeps its own compose project name unless `--adopt-identity` says to take the archive's over, so restoring into a second deployment is a copy and not a takeover.

```text
Usage: chaps backup restore [OPTIONS] <ARCHIVE>
```

| Argument | Description |
| --- | --- |
| `<ARCHIVE>` | The `tar.gz` written by `chaps backup create`. |
| `--yes` | Skip the confirmation. Required when there is no terminal to ask at. |
| `--files-only` | Restore only `.env`, `.chaps/`, `ocs/` and the compose files. Needs no Docker. |
| `--db-only` | Restore only the chap-core database. |
| `--no-models` | Leave the model data volumes as they are. |
| `--no-components` | Leave the component data volumes (ocs, s3) as they are. |
| `--adopt-identity` | Take over the compose project name the archive was taken under. Without this the deployment keeps its own name, so restoring an archive into a second deployment refills that deployment's containers and volumes rather than the ones the backup came from. Pass it when this deployment *is* the one in the archive, moved to another directory or another machine, and should answer to its name again. |
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

## chaps doctor

Run a checklist over this machine and this deployment.

One line per check: Docker and Compose, the CPU architecture, free disk, whether the hosts CHAP pulls from answer, and which release of `chaps` this is. Inside a deployment directory it also checks the project files, whether the compose files are in sync, `.env`, the host ports, the chap-core pin, each enabled model's image and the running stack.

Every check is bounded, so this always finishes; it exits non-zero only when a check failed, never on a warning. `--json` prints the same checklist as a document, and `--offline` skips everything that would need the network.

```text
Usage: chaps doctor [OPTIONS]
```

## chaps auth

Turn API authentication on or off, and show the token to paste into DHIS2.

Two secrets, both living in `.env` and nowhere else: `CHAP_API_TOKEN`, which every client has to send as `Authorization: Bearer <token>`, and `SERVICEKIT_REGISTRATION_KEY`, which each model service sends when it registers with chap-core. `.chaps/project.yaml` records only whether they are in use.

```text
Usage: chaps auth [OPTIONS] <COMMAND>
```

Subcommands: [`chaps auth show`](#chaps-auth-show), [`chaps auth enable`](#chaps-auth-enable), [`chaps auth disable`](#chaps-auth-disable), [`chaps auth rotate`](#chaps-auth-rotate)

## chaps auth show

Say whether the API is protected, and by which token.

Reads `.chaps/project.yaml` for the intent and `.env` for what is actually set, and says so when the two disagree. The token is abbreviated unless `--reveal` asks for it in full.

```text
Usage: chaps auth show [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--reveal` | Print the API token in full, to paste into the DHIS2 Modeling App. |

## chaps auth enable

Turn authentication on: generate the secrets, write them to .env and sync.

Writes an active `CHAP_API_TOKEN` and `SERVICEKIT_REGISTRATION_KEY` into `.env` - only those two lines change - records both in `.chaps/project.yaml` and re-renders the model overlays so each service sends the registration key. Run `chaps up` afterwards: chap-core and the models only read `.env` when their containers are created.

```text
Usage: chaps auth enable [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--token <TOKEN>` | Use this API token instead of generating one. |

## chaps auth disable

Turn authentication off again, keeping both values as comments.

Comments the two `.env` lines out rather than deleting them, so the token your clients are configured with can be recovered, and re-renders the overlays without the registration key. Run `chaps up` afterwards.

```text
Usage: chaps auth disable [OPTIONS]
```

## chaps auth rotate

Replace both secrets with freshly generated ones.

Rewrites only those two `.env` lines and leaves the rest of the file alone. Every client keeps sending the old token until it is updated, so rotating is two steps: `chaps auth rotate`, then `chaps up` and the new token wherever the old one was configured.

```text
Usage: chaps auth rotate [OPTIONS]
```

## chaps self

Update chaps itself, and report what this build is.

These are the only commands that are about the CLI rather than about a deployment, so they work anywhere, project or not.

```text
Usage: chaps self [OPTIONS] <COMMAND>
```

Subcommands: [`chaps self update`](#chaps-self-update), [`chaps self version`](#chaps-self-version)

## chaps self update

Replace this binary with the newest release.

Looks up the release, downloads the archive built for this target (on macOS the universal one, which carries both slices), checks it against the release's `SHA256SUMS`, and renames the new binary over the running one. The file is never written through: a failed download leaves the installed `chaps` exactly as it was.

A binary installed by `cargo install` is replaced the same way, but `cargo install chaps-cli` is the more honest way to move that one on.

```text
Usage: chaps self update [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--check` | Report what an update would do and change nothing. |
| `--version <TAG>` | Install this release tag instead of the newest one. Going back to an older release is allowed; going nowhere is not. |
| `-y, --yes` | Do not ask before replacing the binary. |

## chaps self version

Show what this build is: version, revision, target and path.

`chaps --version` prints the version alone; this prints everything that identifies one build, which is what a bug report needs.

```text
Usage: chaps self version [OPTIONS]
```

## chaps completions

Print a shell completion script for chaps.

The script is written to stdout, so it can be sourced directly or saved into the shell's completion directory. The release archives ship the same four scripts under `completions/`, and the install script puts them in place, so this command is for a checkout or a shell the archives do not cover.

```text
Usage: chaps completions [OPTIONS] <SHELL>
```

| Argument | Description |
| --- | --- |
| `<SHELL>` | Shell to generate for. Values: `bash`, `elvish`, `fish`, `powershell`, `zsh`. |
