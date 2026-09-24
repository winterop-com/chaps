# Commands

```text
chaps [--json] [-C DIR] [--registry-url URL] [--offline] [--cache-dir DIR] <command>
```

The groups below are the shape of the tool. The
[Command reference](./reference.md) is generated from the `--help` texts and
lists every command, every flag and every default.

## The everyday verbs

| Command | What it does |
| --- | --- |
| `chaps init [DIR]` | Create a deployment directory: compose files, `.env` and `.chaps/`. `--api-token` protects the API from the start; `--with ocs[,s3]` adds optional components. |
| `chaps sync [--check]` | Render the compose files from `.chaps/`; `--check` writes nothing and exits non-zero if anything would change. |
| `chaps up [-a] [--pull] [--no-preflight] [EXTRA..]` | Sync, check that the host ports are free, then `docker compose up -d --remove-orphans`, and end with what started or was recreated. The orphans are the containers of models and components that were disabled; see [Orphans](./concepts.md#orphans) for what that means for a compose file of your own. `-a` (also `--attach`, `--foreground`) runs in the foreground and streams the logs instead. |
| `chaps down [--volumes] [-y] [EXTRA..]` | `docker compose down`, then say what it stopped and that the volumes are still there, naming the compose project they are prefixed with. `--volumes` removes them too, and the data in them: it names the volumes first and asks, unless `-y`/`--yes` says it was meant. |
| `chaps logs [-f] [SERVICE..]` | `docker compose logs`; says so instead of printing nothing when the project has no containers, and lists the services when `SERVICE` is not one of them. |
| `chaps restart [SERVICE..] [--all]` | Recreate the running services whose image or configuration changed (`docker compose up -d --remove-orphans`), and say which ones that was. Changes no file and no pin; `--all` recreates the named services anyway. |
| `chaps status [--url URL] [--timeout SECONDS]` | `GET /health` and `/v2/services`, check that the answers are chap-core's, and diff the registered services against the ones this project enabled. Sends the API token from `.env` when there is one, and says `auth: on` or `auth: off`. |
| `chaps jobs [list] [--status S].. [--type T] [--limit N]` | The backtests, predictions and datasets chap-core has run, newest first, with what each one cost and which of them failed. `show`, `logs`, `cancel` and `delete` take one job id, or enough of its start to name one. |
| `chaps api METHOD PATH [--data JSON\|@FILE\|-] [--url URL] [--raw]` | One authenticated request to chap-core's API, with this deployment's base URL and token filled in. JSON comes back pretty-printed; the exit code is 0 for a 2xx, 1 for a 4xx/5xx and 2 when chap-core is not answering. |
| `chaps update [--dry-run] [--pin-chap-core]` | Move the pins to what upstream publishes now, pull the images, and end with one line saying what moved and what needs restarting. Never touches a container. |
| `chaps doctor` | Run a checklist over this machine and this deployment: Docker, Compose, architecture, disk, the hosts CHAP pulls from, and - inside a project - the files, the ports, the pins, the images and whether CHAP is up. Works anywhere. |

`chaps up` starts what is on disk, `chaps update` fetches newer versions, and
`chaps restart` applies them to the services that are running. That is the
whole distinction, and it is why `up` and `update` are both safe to run at any
time. See [Updating](./updating.md).

`chaps jobs` and `chaps api` are the two that talk to the API rather than to
Docker. A job is one unit of slow work chap-core handed to its worker, and a
failed one says why in `chaps jobs logs <id>` and nowhere else; `chaps api` is
what starts a job from a terminal in the first place. See
[Jobs and the API](./jobs.md).

`chaps doctor` is the one to run first on a machine you have not deployed on
before, and first again when something is wrong: it asks in one pass what the
other commands assume. It exits non-zero only when a check failed. See
[Doctor](./doctor.md).

`chaps down --volumes` is the reset that takes the data with it
(`docker compose down -v --remove-orphans`): it lists the volumes docker holds
under this deployment's name - with how much the OCS data volume holds next to
it, when that can be measured - asks before removing them and ends with the
ones that are gone, by name. `-y` (`--yes`) is how a script says it meant it, and is
required where there is nothing to ask at - a stdin that is not a terminal, or
`--json` - rather than the question being skipped. Compose removes the volumes
its files still declare, so a volume left over from a disabled model survives;
`chaps doctor` finds those, and `chaps models disable <id> --purge` removes
them. There is no `-v` for this: `-v` is the global `--verbose` flag, so a `-v`
that reaches the passthrough (`chaps down -- -v`) is refused and told to use
`--volumes`, rather than being passed on to compose or quietly dropped.

## Models

| Command | What it does |
| --- | --- |
| `chaps models list` | List marketplace models (`--all`, `--templates`, `--enabled`). |
| `chaps models search QUERY` | Search id, name and summary. |
| `chaps models info ID` | Everything known about one model. |
| `chaps models add SOURCE` | Add a model the marketplace does not list, from a GitHub repository URL or a ghcr image reference, and enable it (`--id`, `--service-id`, `--name`, `--port`, `--data-dir`, `--user`, `--runtime-amd64`). |
| `chaps models remove ID [--purge]` | Disable such a model if it is on, then drop its definition from `.chaps/models-manual.yaml`; `--purge` removes its data volume too. |
| `chaps models enable ID` | Record the model in `.chaps/models.yaml` and write its overlay (`--channel`, `--version`, `--port`, `--data-dir`, `--user`, `--allow-template`). |
| `chaps models disable ID [--purge]` | Drop the model from `.chaps/models.yaml` and remove its overlay, keeping its data volume and naming it; `--purge` removes that volume too. |
| `chaps models expose ID [--port N\|auto]` | Publish a host port for an enabled model, without touching the version it is pinned to. |
| `chaps models unexpose ID` | Take that host port away again. |
| `chaps ui` | The model browser. |

`list`, `search` and `info` read the catalogue and work outside a project. The
rest write to a project. `add` is the only one that needs the network for
something other than the catalogue: it reads GitHub and ghcr to resolve what it
was given. See [Models and the marketplace](./models.md).

## Components

| Command | What it does |
| --- | --- |
| `chaps components list` | Every component, whether this deployment has it and where it is reached. |
| `chaps components enable NAME [--port N]` | Turn a component on, or change the settings of one that already is, then sync. `ocs` also takes `--ocs-name`, `--ocs-country` and `--ocs-bbox` for the instance config it scaffolds. |
| `chaps components disable NAME [--purge]` | Turn it off, remove its compose file and sync, keeping its data volume and naming it; `--purge` removes that volume too, and is refused for chap-core. |

A component is a service (or a small group) that `chaps sync` renders one
compose file for: `chap-core`, which is CHAP itself and is on unless you turn it
off, `ocs` (Open Climate Service) and `s3` (the object store OCS will use). The
set lives in `.chaps/components.yaml`. `chaps init --with ocs,s3` and
`--without chap-core` set it at creation time. See [Components](./components.md).

## Authentication

| Command | What it does |
| --- | --- |
| `chaps auth show [--reveal]` | Whether the API token and the registration key are set, and the token itself, abbreviated unless `--reveal`. |
| `chaps auth token` | The token alone on stdout, for `TOKEN=$(chaps auth token)`; nothing on stdout and exit 1 when authentication is off. |
| `chaps auth enable [--token VALUE]` | Write both secrets to `.env`, record them in `.chaps/project.yaml` and re-render the overlays. |
| `chaps auth disable` | Comment both `.env` lines out, keeping their values, and re-render. |
| `chaps auth rotate` | Replace both secrets with freshly generated ones. |

All four touch only those two `.env` lines, and none of them restarts anything:
chap-core and the models read `.env` when Compose creates them, so `chaps up`
has to follow. `chaps init --api-token` does the same thing at creation time.
See [Authentication](./auth.md).

## Registry

| Command | What it does |
| --- | --- |
| `chaps registry update` | Fetch the catalogue now and refresh the cache. |
| `chaps registry show` | Where the catalogue came from and what it holds. |

## Docker

The Docker plumbing you only reach for when you already know what Docker is
doing lives under `chaps docker`, so the top-level list stays readable if you
have never used Compose.

| Command | What it does |
| --- | --- |
| `chaps docker ps [EXTRA..]` | List this project's containers (`docker compose ps`). |
| `chaps docker pull` | Download the pinned images into the local Docker daemon; no files change. |
| `chaps docker exec SERVICE [CMD..]` | Run a command in a running container, which has to be up already (`chaps up`). `CMD` defaults to a shell, and `-T` is passed for you when there is no terminal, so it works in scripts. |
| `chaps docker run -- ARGS..` | Any `docker compose` command, behind the project's `-f` list. |
| `chaps docker config [-- EXTRA..]` | The finished configuration: every compose file merged into one document (`--json` prints it as JSON). |

`chaps docker config` is what Docker actually reads after the `-f` list, the
`include:` and the `.env` substitutions have been applied, which is the fastest
way to find out why a setting is not taking effect.

## Backup

| Command | What it does |
| --- | --- |
| `chaps backup create [--out PATH] [--no-db] [--no-models]` | Write the database, the model data and the project files to one `tar.gz`. |
| `chaps backup restore ARCHIVE [--yes] [--files-only] [--db-only] [--no-models] [--no-start]` | Put a deployment back from such an archive, after printing what it overwrites. |

See [Backup and restore](./backup.md).

## chaps itself

The two commands that are about the CLI rather than about a deployment. Both
work anywhere, inside a project or not.

| Command | What it does |
| --- | --- |
| `chaps self update [--check] [--version TAG] [--yes]` | Replace this binary with the newest release: download the archive for this target, check it against the release's `SHA256SUMS`, rename it over the running one. `--check` only reports. |
| `chaps self version` | Version, git revision, target triple, binary path and whether it came from a release archive or `cargo install`. |
| `chaps completions SHELL` | Print a completion script for bash, zsh, fish, PowerShell or elvish. |

`chaps update` moves a deployment's pins; `chaps self update` replaces the
`chaps` binary. Nothing in the first touches the second. See
[Install](./install.md) for both of these and for the once-a-day notice that a
newer release exists.

## Global options

They are accepted before or after the subcommand, and the
[reference](./reference.md#chaps) lists them with their defaults.

| Option | What it does |
| --- | --- |
| `--json` | Machine-readable output: exactly one JSON document on stdout. |
| `--no-color` | Never colour the output; `NO_COLOR` in the environment does the same. |
| `-v, --verbose` | Narrate on stderr what runs: commands, HTTP requests, the registry source, the files `sync` compared. |
| `-d, --debug` | Everything `-v` says, plus response bodies and resolved paths. |
| `-C, --project-dir DIR` | Where to look for the project; found like git finds `.git`. |
| `--registry-url URL` | A different marketplace index, for a fork or a mirror. |
| `--offline` | Never touch the network; use the cache or the embedded snapshot. |
| `--cache-dir DIR` | Override the registry cache directory for one invocation. |
