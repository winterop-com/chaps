# Commands

```text
varde [--json] [-C DIR] [--registry-url URL] [--offline] [--cache-dir DIR] <command>
```

The groups below are the shape of the tool. The
[Command reference](./reference.md) is generated from the `--help` texts and
lists every command, every flag and every default.

## Running one model

| Command | What it does |
| --- | --- |
| `varde run MODEL [--port N\|auto] [--bind ADDR] [--group NAME] [--allow-template] [--no-wait] [--timeout S] [-a [--rm]] [--chap-core URL]` | Start one model - a marketplace id, a GitHub repository URL, a ghcr image or a local image - and print where it answers, once it does. Outside a deployment it runs in a `varde run` group under the data directory, on `127.0.0.1`; inside one, in that deployment. `-a` keeps it in the foreground with its log, and Ctrl-C then stops the model and keeps its data (`--rm` removes it). `--chap-core URL` makes the group's models register with a chap-core that runs elsewhere. |
| `varde ps [--group NAME]` | Every model in every group (or in the deployment the command is inside), with its state and URL. |
| `varde stop ID\|--all\|--group NAME [--purge]` | Stop a model, in whichever group has it, or every model in a group, and take its overlay away; the data stays unless `--purge`, which also removes a group left empty. |
| `varde top [--interval S]` | Every varde deployment on the machine as a live tree - `init` deployments and `run` groups, every service with its state, CPU, memory and URL - with logs and stop on a key. One snapshot off a terminal. See [varde top](./top.md). |

See [Running one model](./run.md).

## The everyday verbs

| Command | What it does |
| --- | --- |
| `varde init [DIR]` | Create a deployment directory: compose files, `.env` and `.varde/`. `--api-token` protects the API from the start; `--source PATH` builds chap-core from a checkout; `--chap-core-url URL` uses a chap-core elsewhere; `--with ocs[,s3][,dhis2]` adds optional components, `--only LIST` deploys exactly those and nothing else, and `--ocs-port`, `--s3-port`, `--ocs-read-only`, `--dhis2-port`, `--dhis2-seed` and `--dhis2-seed-password` settle how they start. |
| `varde sync [--check]` | Render the compose files from `.varde/`; `--check` writes nothing and exits non-zero if anything would change. |
| `varde up [-a] [--pull] [--no-preflight] [--replace] [--wait [--timeout S]] [EXTRA..]` | Sync, check that the host ports are free (offering to stop another varde deployment that holds them; `--replace` does so without asking), then `docker compose up -d --remove-orphans`, and end with what started or was recreated. The orphans are the containers of models and components that were disabled; see [Orphans](./concepts.md#orphans) for what that means for a compose file of your own. `-a` (also `--attach`, `--foreground`) runs in the foreground and streams the logs instead. A deployment with no components and no models has nothing to start, and `up` says so and names the commands that add one. `--wait` holds the command until chap-core answers and every model is registered (or, without chap-core, answers on its own port), then prints where each one answers; after `--timeout` seconds (300) it fails and names what never did. |
| `varde down [--volumes] [-y] [EXTRA..]` | `docker compose down`, then say what it stopped and that the volumes are still there, naming the compose project they are prefixed with. `--volumes` removes them too, and the data in them: it names the volumes first and asks, unless `-y`/`--yes` says it was meant. Either way it ends on `varde up`, which brings it back. |
| `varde logs [-f] [--tail N] [SERVICE..]` | `docker compose logs`; says so instead of printing nothing when the project has no containers, and lists the services when `SERVICE` is not one of them. `--tail N` keeps the last N lines of each service. When stdout is not a terminal, or under `--no-color` or `NO_COLOR`, the colour codes the containers print themselves are removed too, so a script reads plain text. |
| `varde restart [SERVICE..] [--all]` | Recreate the running services whose image or configuration changed (`docker compose up -d --remove-orphans`), and say which ones that was. Changes no file and no pin; `--all` recreates the named services anyway. |
| `varde status [--url URL] [--timeout SECONDS]` | `GET /health` and `/v2/services`, check that the answers are chap-core's, and diff the registered services against the ones this project enabled. Sends the API token from `.env` when there is one, and says `auth: on` or `auth: off`. |
| `varde open [NAME]` | Open a component's web interface in a browser: chap-core's API documentation at `/docs`, the OCS web interface, DHIS2's own; an enabled model's id opens that model's `/docs`, on its host port or through chap-core. With no name it lists every component and what each one opens. `--no-browser` prints the address instead of opening one, and `--json` never opens one. A component this deployment does not have, one that publishes no host port and the object store - which serves no web interface at all - are each refused with what is true instead and the command that changes it. |
| `varde jobs [list] [--status S].. [--type T] [--limit N]` | The backtests, predictions and datasets chap-core has run, newest first, with what each one cost and which of them failed. `show`, `logs`, `cancel` and `delete` take one job id, or enough of its start to name one. |
| `varde api METHOD PATH [--data JSON\|@FILE\|-] [--url URL] [--raw]` | One authenticated request to chap-core's API, with this deployment's base URL and token filled in. JSON comes back pretty-printed; the exit code is 0 for a 2xx, 1 for a 4xx/5xx and 2 when chap-core is not answering. |
| `varde update [--dry-run] [--pin-chap-core] [--chap-tag TAG] [--list-tags] [--yes]` | Move the pins to what upstream publishes now, pull the images, and end with one line saying what moved and what needs restarting. Never touches a container. `--chap-tag` moves chap-core to another tag (a release, `latest`, `master` or `dev`) instead, asking first when the move goes backwards, which `--yes` answers; `--list-tags` lists where it can go and writes nothing. Outside a deployment it refreshes the marketplace registry instead. |
| `varde doctor` | Run a checklist over this machine and this deployment: Docker, Compose, architecture, disk, the hosts Chap pulls from, and - inside a project - the files, the ports, the pins, the images and whether Chap is up. Works anywhere. |
| `varde cleanup [--dry-run] [--yes]` | Delete the volumes and network that deployments whose directory is gone left in docker, after listing them and asking. `--dry-run` (`-n`) only lists; `--yes` deletes without asking. Only deployments varde recorded are considered, and nothing a container still uses. Works anywhere. See [Cleaning up after removed deployments](./doctor.md#cleaning-up-after-removed-deployments). |
| `varde dhis2 connect` | Let the DHIS2 Modeling App reach this deployment's Chap: the `chap` route, the apps, then analytics. `show`, `route`, `analytics` and `apps` are the same work one step at a time; `varde dhis2 use` points them at a DHIS2 that runs elsewhere. |

`varde up` starts what is on disk, `varde update` fetches newer versions, and
`varde restart` applies them to the services that are running. That is the
whole distinction, and it is why `up` and `update` are both safe to run at any
time. See [Updating](./updating.md).

`varde jobs` and `varde api` are the two that talk to the API rather than to
Docker. A job is one unit of slow work chap-core handed to its worker, and a
failed one says why in `varde jobs logs <id>` and nowhere else; `varde api` is
what starts a job from a terminal in the first place. See
[Jobs and the API](./jobs.md).

`varde doctor` is the one to run first on a machine you have not deployed on
before, and first again when something is wrong: it asks in one pass what the
other commands assume. It exits non-zero only when a check failed. See
[Doctor](./doctor.md).

`varde down --volumes` is the reset that takes the data with it
(`docker compose down -v --remove-orphans`): it lists the volumes docker holds
under this deployment's name - with how much the OCS data volume holds next to
it, when that can be measured - asks before removing them and ends with the
ones that are gone, by name. `-y` (`--yes`) is how a script says it meant it, and is
required where there is nothing to ask at - a stdin that is not a terminal, or
`--json` - rather than the question being skipped. Compose removes the volumes
its files still declare, so a volume left over from a disabled model survives;
`varde doctor` finds those, and `varde models disable <id> --purge` removes
them. There is no `-v` for this: `-v` is the global `--verbose` flag, so a `-v`
that reaches the passthrough (`varde down -- -v`) is refused and told to use
`--volumes`, rather than being passed on to compose or quietly dropped.

## Models

| Command | What it does |
| --- | --- |
| `varde models list` | List marketplace models (`--all`, `--templates`, `--enabled`). |
| `varde models search QUERY` | Search id, name and summary. |
| `varde models info ID` | Everything known about one model. |
| `varde models test [ID..] [--all]` | Make each model train and predict, and say whether it could: `chapkit test` in its own container, or `--backtest` for the whole way round through chap-core (`--seed`, `--timeout`, `--keep`). |
| `varde models add SOURCE` | Add a model the marketplace does not list, from a GitHub repository URL or a ghcr image reference, and enable it (`--id`, `--service-id`, `--name`, `--port`, `--bind`, `--data-dir`, `--user`, `--runtime-amd64`). A source the marketplace lists is enabled as that marketplace model unless `--id` is given; `--id auto` picks a free id. |
| `varde models remove ID [--purge]` | Disable such a model if it is on, then drop its definition from `.varde/models-manual.yaml`; `--purge` removes its data volume too. |
| `varde models enable ID` | Record the model in `.varde/models.yaml` and write its overlay (`--channel`, `--version`, `--port`, `--bind`, `--data-dir`, `--user`, `--allow-template`). |
| `varde models disable ID [--purge]` | Drop the model from `.varde/models.yaml` and remove its overlay, keeping its data volume and naming it; `--purge` removes that volume too. |
| `varde models expose ID [--port N\|auto] [--bind ADDR]` | Publish a host port for an enabled model, without touching the version it is pinned to; `--bind 127.0.0.1` keeps it on this machine. See [Ports](./ports.md#which-address-a-model-port-is-published-on). |
| `varde models unexpose ID` | Take that host port away again. |
| `varde ui` | Open the browser: marketplace models and components, on two pages `Tab` moves between. One `s` saves both. |

`list`, `search` and `info` read the catalogue and work outside a project. The
rest need a project; `test` is the only one of them that changes nothing.
Registration is a heartbeat, so `varde status` and `varde doctor` can be green
while a model cannot predict at all - `varde models test` is the check that
settles it. See [Testing a model](./models.md#testing-a-model). `add` is the only one that needs the network for
something other than the catalogue: it reads GitHub and ghcr to resolve what it
was given. See [Models and the marketplace](./models.md).

## Components

| Command | What it does |
| --- | --- |
| `varde components list` | Every component, whether this deployment has it and where it is reached. |
| `varde components enable NAME [--port N]` | Turn a component on, or change the settings of one that already is, then sync. `ocs` also takes `--ocs-name`, `--ocs-country` and `--ocs-bbox` for the instance config it scaffolds, plus `--base-url` and `--read-only`/`--read-write`. A port already in use is a warning, not a refusal. |
| `varde components disable NAME [--purge]` | Turn it off, remove its compose file and sync, keeping its data volumes and naming each one; `--purge` removes them too, and is refused for chap-core. |

A component is a service (or a small group) that `varde sync` renders one
compose file for: `chap-core`, which is Chap itself and is on unless you turn it
off, `ocs` (Open Climate Service), `s3` (the object store OCS will use) and
`dhis2` (a demo or development DHIS2 with its own database). The set lives in
`.varde/components.yaml`. `varde init --with ocs,s3`, `--without chap-core` and
`--only ocs,s3` set it at creation time. `varde open NAME` opens one of them in a browser, and
`o` does the same from the browser's components page. See
[Components](./components.md),
[Reaching a component from a browser](./components.md#reaching-a-component-from-a-browser)
and [DHIS2](./dhis2.md).

## DHIS2

For a deployment with the [`dhis2` component](./dhis2.md), or with an
[external DHIS2](./dhis2.md#a-dhis2-that-runs-elsewhere) recorded by `varde dhis2
use`. Apart from `use`, which records a DHIS2 in `.varde/components.yaml`, and
`connect`, which records when it ran in the same file, nothing here touches
Docker or a file: every verb is requests to DHIS2's own API.

| Command | What it does |
| --- | --- |
| `varde dhis2 show [--user NAME]` | Where the `chap` route points, what DHIS2's analytics timestamp is worth, which of the two apps are installed, and each piece that is missing. Writes nothing. |
| `varde dhis2 route` | Create the `chap` route, or repoint one that points elsewhere, is disabled or has lost the `F_CHAP_MODELING_APP` authority - then proxy a request through it to prove chap-core answers. |
| `varde dhis2 analytics [--timeout SECONDS] [--no-wait]` | Generate the analytics tables the Modeling and Climate apps read, and wait for the run; a run that is already going is watched rather than queued behind, since DHIS2 runs one at a time. |
| `varde dhis2 apps` | Install the Modeling App and the DHIS2 Climate App from the App Hub, at the newest version this DHIS2 can run. The one verb that needs the network. |
| `varde dhis2 connect [--timeout SECONDS] [--no-wait]` | The route, the apps, then analytics. Under `--offline` the apps are a reported skip and the other two still happen. On an external DHIS2 only the route; the other two are left to its admin. |
| `varde dhis2 use [URL] [--chap-url URL] [--clear]` | Record a DHIS2 that runs elsewhere, and where it reaches chap-core, then ask it whether it answers and takes the credential. Bare, it says which DHIS2 is used; `--clear` forgets it. |

Every one of them is idempotent: a second run repoints nothing, reinstalls nothing
and says so. All of them take `--user NAME` and `--wait SECONDS` (how long to wait
for DHIS2's API: 20 minutes by default for the `dhis2` component, because a first
start takes minutes, and 60 seconds for an external DHIS2). The credential comes
from `DHIS2_API_TOKEN` or `DHIS2_ADMIN_USERNAME`/`DHIS2_ADMIN_PASSWORD` in
`.env`, else `VARDE_DHIS2_TOKEN` or `VARDE_DHIS2_PASSWORD` in the environment,
else the seed password, else `admin`/`district` (only on a DHIS2 that varde
deployed). See [The
credentials](./dhis2.md#the-credentials-and-where-they-come-from). There is no
`--password` flag, because a password on a command line is in the shell history. `varde up` does none of this and
[the chapter says why](./dhis2.md#varde-up-does-none-of-this-on-purpose) - so
`varde up` and `varde status` each end by naming `varde dhis2 connect` until one
has been recorded in `.varde/components.yaml`. That record suppresses the hint
and is never evidence that the route is right; `varde dhis2 show` is what asks
DHIS2. See [Connecting the Modeling App to
Chap](./dhis2.md#connecting-the-modeling-app-to-chap) and [`connected_at`, and
what it is not](./dhis2.md#connected_at-and-what-it-is-not).

## Authentication

| Command | What it does |
| --- | --- |
| `varde auth show [--reveal]` | Whether the API token and the registration key are set, and the token itself only with `--reveal`. |
| `varde auth token` | The token alone on stdout, for `TOKEN=$(varde auth token)`; nothing on stdout and exit 1 when authentication is off. |
| `varde auth enable [--token VALUE]` | Write both secrets to `.env`, record them in `.varde/project.yaml` and re-render the overlays. |
| `varde auth disable` | Comment both `.env` lines out, keeping their values, and re-render. |
| `varde auth rotate` | Replace both secrets with freshly generated ones. |

All four touch only those two `.env` lines, and none of them restarts anything:
chap-core and the models read `.env` when Compose creates them, so `varde up`
has to follow. `varde init --api-token` does the same thing at creation time.
See [Authentication](./auth.md).

## The chap CLI

| Command | What it does |
| --- | --- |
| `varde chap [--tag TAG] [--image core\|worker] [--group NAME] [--stop] [--timeout S] [--docker] CHAP_ARGS..` | Run chap-core's own `chap` CLI in a container, with the current directory mounted at the same path, so its file arguments and outputs are on this machine. A model id in `--model-name` starts that model, without chap-core and without `varde up`, and `--stop` stops it after the run. The image follows `--model-name`; a deployment gives its tag and its network. `--docker` gives the container the docker socket, for `docker_env` models. Works anywhere. |

You need no Python, uv or R for it. See [The chap CLI](./chap-cli.md).

## Registry

| Command | What it does |
| --- | --- |
| `varde registry update` | Fetch the catalogue now and refresh the cache. |
| `varde registry show` | Where the catalogue came from and what it holds. |

## Docker

The Docker plumbing you only reach for when you already know what Docker is
doing lives under `varde docker`, so the top-level list stays readable if you
have never used Compose.

| Command | What it does |
| --- | --- |
| `varde docker ps [EXTRA..]` | List this project's containers (`docker compose ps`). |
| `varde docker pull` | Download the pinned images into the local Docker daemon; no files change. |
| `varde docker exec SERVICE [CMD..]` | Run a command in a running container, which has to be up already (`varde up`). `CMD` defaults to a shell, and `-T` is passed for you when there is no terminal, so it works in scripts. |
| `varde docker run -- ARGS..` | Any `docker compose` command, behind the project's `-f` list. |
| `varde docker config [-- EXTRA..]` | The finished configuration: every compose file merged into one document (`--json` prints it as JSON). |

`varde docker config` is what Docker actually reads after the `-f` list, the
`include:` and the `.env` substitutions have been applied, which is the fastest
way to find out why a setting is not taking effect.

## Backup

| Command | What it does |
| --- | --- |
| `varde backup create [--out PATH] [--no-db] [--no-models] [--no-components]` | Write the chap-core database (pg_dump), the model data, the component data (the DHIS2 database as a pg_dump) and the project files to one `tar.gz`. |
| `varde backup restore ARCHIVE [--yes] [--files-only] [--db-only] [--no-models] [--no-components] [--adopt-identity] [--no-start]` | Put a deployment back from such an archive, after printing what it overwrites. |

See [Backup and restore](./backup.md).

## varde itself

The two commands that are about the CLI rather than about a deployment. Both
work anywhere, inside a project or not.

| Command | What it does |
| --- | --- |
| `varde self update [--check] [--version TAG] [--yes]` | Replace this binary with the newest release: download the archive for this target, check it against the release's `SHA256SUMS`, rename it over the running one. `--check` only reports. |
| `varde self version` | Version, git revision, target triple, binary path and whether it came from a release archive, `cargo install` or a `cargo build` in a checkout. |
| `varde completions SHELL` | Print a completion script for bash, zsh, fish, PowerShell or elvish. |

`varde update` moves a deployment's pins; `varde self update` replaces the
`varde` binary. Nothing in the first touches the second. See
[Install](./install.md) for both of these and for the once-a-day notice that a
newer release exists.

## Global options

They are accepted before or after the subcommand, on every command, and the
[reference](./reference.md#varde) lists them with their defaults. The last
three are the registry options. The help lists them under "Registry options"
only for the commands that use them; [Global
options](./status.md#global-options) names those commands. On the other
commands they change only the once-a-day notice that a newer release exists.

| Option | What it does |
| --- | --- |
| `--json` | Machine-readable output: exactly one JSON document on stdout. |
| `--no-color` | Never colour the output; `NO_COLOR` in the environment does the same. |
| `-v, --verbose` | Show the hints: the next commands and the background. `-vv` also narrates on stderr what runs: commands, HTTP requests, the registry source, the files `sync` compared. |
| `-d, --debug` | Everything `-vv` says, plus response bodies and resolved paths. |
| `-C, --project-dir DIR` | Where to look for the project; found like git finds `.git`. |
| `--registry-url URL` | A different marketplace index, for a fork or a mirror. Inside a deployment the default is the one `init` recorded. |
| `--offline` | Never touch the network; use the cache or the embedded snapshot. Shown by the commands that read the registry, and by `dhis2 apps`, `dhis2 connect` and `self update`, which use it for the App Hub and the release feed. |
| `--cache-dir DIR` | Override the cache directory for one invocation: the registry snapshot, and the downloads of `self update`. |

## `--json` for scripts and tools

Under `--json` every command prints exactly one JSON document on stdout, and
everything meant for a person - progress, notes, warnings - goes to stderr.
A tool that drives varde reads stdout and branches on one field:

- **Success** of a command that changes something (`models enable`, `disable`,
  `add`, `remove`, `expose`, `unexpose`, `up`, `models test`) carries
  `"ok": true`. The read-only commands (`status`, `models list`, `models info`)
  print their report as it is; `status` says how things are in its own exit
  code.
- **Failure** of any command is

  ```json
  {"ok": false, "error": "...", "hint": "run `varde ...`", "causes": []}
  ```

  `hint` is the way out: the message's last clause when that clause names a
  command or a flag to use instead (`pass \`--id auto\` ... to add this one
  beside it`), and `null` when the message has no way out to give. `error` is
  the message up to that clause, so the two never say the same thing; the
  terminal shows them together after `error:`. A usage error - an unknown flag, a
  missing argument - is the same document, with clap's message as `error` and
  the command's `--help` as `hint`, and exits 2.
- `models enable` and `models add` list what they enabled under `models`, one
  object per model with `id`, `service_id`, `port`, `bind` and `url` (the
  model's own port, else chap-core's proxy to it, else `null`), next to the
  full change report.
- `up` prints `api_url`, the compose services `running` and the ones this run
  `started`, every enabled model the same way, and under `--wait` what the wait
  found: `wait.ready`, `wait.waited_s` and each model's `state` and `url`.
- `models test` prints `{"ok": ..., "models": [...]}`, one object per model with
  its `result` (`pass`, `fail` or `skip`), the `summary` and the `detail` that
  says why and what to do.

The exit code agrees with `ok`: 0 for `true`, non-zero for `false`, and
`models test` exits non-zero when any model failed even though it printed
`"ok": false` rather than an error.
