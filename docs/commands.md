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
| `chaps init [DIR]` | Create a deployment directory: compose files, `.env` and `.chaps/`. `--api-token` protects the API from the start. |
| `chaps sync [--check]` | Render the compose files from `.chaps/`; `--check` writes nothing and exits non-zero if anything would change. |
| `chaps up [-a] [--pull] [--no-preflight] [EXTRA..]` | Sync, check that the host ports are free, then `docker compose up -d`, and end with what started or was recreated. `-a` (also `--attach`, `--foreground`) runs in the foreground and streams the logs instead. |
| `chaps down [EXTRA..]` | `docker compose down`, then say what it stopped and that the volumes are still there. |
| `chaps logs [-f] [SERVICE..]` | `docker compose logs`; says so instead of printing nothing when the project has no containers, and lists the services when `SERVICE` is not one of them. |
| `chaps status [--url URL] [--timeout SECONDS]` | `GET /health` and `/v2/services`, check that the answers are chap-core's, and diff the registered services against the ones this project enabled. Sends the API token from `.env` when there is one, and says `auth: on` or `auth: off`. |
| `chaps update [--dry-run] [--no-restart] [--pin-chap-core]` | Move the pins to what upstream publishes now, then pull and restart. |

`chaps up` starts what is on disk; `chaps update` fetches newer versions. That
is the whole distinction, and it is why `up` is safe to run at any time.

## Models

| Command | What it does |
| --- | --- |
| `chaps models list` | List marketplace models (`--all`, `--templates`, `--enabled`). |
| `chaps models search QUERY` | Search id, name and summary. |
| `chaps models info ID` | Everything known about one model. |
| `chaps models enable ID` | Record the model in `.chaps/models.yaml` and write its overlay (`--channel`, `--version`, `--port`, `--data-dir`, `--user`, `--allow-template`). |
| `chaps models disable ID` | Drop the model from `.chaps/models.yaml` and remove its overlay. |
| `chaps models expose ID [--port N\|auto]` | Publish a host port for an enabled model, without touching the version it is pinned to. |
| `chaps models unexpose ID` | Take that host port away again. |
| `chaps ui` | The model browser. |

`list`, `search` and `info` read the catalogue and work outside a project. The
rest write to a project. See [Models and the marketplace](./models.md).

## Authentication

| Command | What it does |
| --- | --- |
| `chaps auth show [--reveal]` | Whether the API token and the registration key are set, and the token itself, abbreviated unless `--reveal`. |
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
| `chaps docker exec SERVICE [CMD..]` | Run a command in a running container. `CMD` defaults to a shell, and `-T` is passed for you when there is no terminal, so it works in scripts. |
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
| `-v, --verbose` | Narrate on stderr what runs: commands, HTTP requests, the registry source. |
| `-d, --debug` | Everything `-v` says, plus response bodies and resolved paths. |
| `-C, --project-dir DIR` | Where to look for the project; found like git finds `.git`. |
| `--registry-url URL` | A different marketplace index, for a fork or a mirror. |
| `--offline` | Never touch the network; use the cache or the embedded snapshot. |
| `--cache-dir DIR` | Override the registry cache directory for one invocation. |
