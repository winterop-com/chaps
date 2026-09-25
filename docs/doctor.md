# Doctor

`chaps doctor` is the first command to run on a machine you have not deployed
on before, and the first one to run when something is wrong. It asks, in one
pass, every question the other commands assume the answer to: is Docker there,
is its daemon up, is Compose new enough, is there disk, can this machine reach
the hosts CHAP pulls from, and is this the newest `chaps`. Inside a deployment
directory it goes on to the project: its compose project name, the files, the
compose files, `.env`, the volumes, the host ports, the pins, the images and
whether CHAP is up.

```sh
chaps doctor
```

```text
ok    docker cli                        docker 29.8.1
ok    docker daemon                     Docker Engine 29.8.0
ok    docker compose                    v2.24.6
warn  os and arch                       macos/aarch64; CHAP images are linux/amd64 only, so they run under emulation
      nothing to do: Rosetta runs the amd64 images, they are only slower
ok    disk space                        633.1 GB free on /Users/you/mychap
ok    network ghcr.io                   reachable (HTTP 401)
ok    network marketplace               reachable (HTTP 200)
ok    network releases                  reachable, newest chap-core is v2.3.1
ok    github api                        reachable, 4990 of 5000 requests left this hour (token)
ok    chaps                             v0.2.0, aarch64-apple-darwin, release archive
ok    project                           mychap-1ab2c3 (recorded in .chaps/project.yaml)
ok    project files                     all 7 present
ok    compose files                     in sync (0 to write, 7 unchanged, 0 to remove)
ok    .env                              auth off, POSTGRES_PASSWORD set, CHAP_IMAGE_TAG present
ok    volumes                           6 volumes named mychap-1ab2c3_*
fail  api port                          8000 is in use by something else
      port 8000 is already in use on this machine (needed by chap); free it, or run
      `chaps init --api-port 8001 --force` here / set CHAP_API_PORT=8001 in .env
ok    chap-core pin                     v2.3.1 is the newest release
ok    image chapkit-ewars-model         ghcr.io/chap-models/chapkit_ewars_model:sha-8d4a7ea (linux/amd64)
ok    registry pin chapkit_ewars_model  sha-8d4a7ea is the newest build on main
skip  health                            no container of this project is running
      run `chaps up` to start CHAP

20 checks: 17 ok, 1 warn, 1 fail, 1 skipped
```

Four words carry the verdict, and only one of them is a problem:

| Word | Colour | What it means |
| --- | --- | --- |
| `ok` | green | Nothing to do. |
| `warn` | yellow | It works, but something will bite later. |
| `fail` | red | CHAP will not work until this is dealt with. |
| `skip` | dim | Not evaluated: there was nothing to ask, or asking was ruled out. |

A line that is not `ok` carries an indented line under it saying what to do
about it, and that line is the same sentence the command it belongs to would
have failed with. The exit code is non-zero **only when something failed**, so
`chaps doctor` in a CI step goes red on the checks that would have stopped the
deployment and stays green on the ones that are merely worth knowing.

Every check is bounded. Each external command is killed if it overruns, each
network probe has a three-second timeout, and the probes run in parallel, so
`chaps doctor` always finishes and normally does so in a few seconds.
`-v` prints every command and request it made.

## Machine checks

These run everywhere, inside a deployment directory or not.

| Check | What it asks | What a bad answer means |
| --- | --- | --- |
| `docker cli` | `docker version` answers | `docker` is not on `PATH`. Install Docker Desktop or the docker CLI from <https://docs.docker.com/get-docker/>. |
| `docker daemon` | `docker info` reaches a daemon | Either nothing is listening, which is `open Docker Desktop` or `sudo systemctl start docker`, or the socket refuses this user, which is `sudo usermod -aG docker $USER` and a fresh login. The two are told apart by what Docker printed, because the fixes have nothing in common. |
| `docker compose` | `docker compose version` and how old it is | Below 2.24.4 the API port override will not work: `compose.chaps.yml` uses `!override`, which arrived in 2.24.4, so the API is published on two ports. Below 2.20 the model overlays will not load either: `compose.marketplace.yml` uses `include:`, which arrived in 2.20. A warning rather than a failure, because the base services still run. |
| `os and arch` | what this host is | On an arm64 host it warns: chap-core and every marketplace image are published for amd64 only. On macOS that is nothing to do, Rosetta runs them. On arm64 Linux it needs qemu/binfmt: `docker run --privileged --rm tonistiigi/binfmt --install amd64`. Nothing is probed, because probing means running an amd64 container, which is far too slow for a checklist. |
| `disk space` | free space where the images land | Warns below 10 GB and fails below 3 GB. The measured filesystem is Docker's own data root when this host can see it, and the working directory otherwise: Docker Desktop reports a path inside its Linux VM, which says nothing about the disk out here. |
| `network ghcr.io` | can this machine reach the image registry | Without it no image can be pulled. A warning, not a failure: `--offline` keeps `chaps` itself working from the cache or the catalogue built into the binary. |
| `network marketplace` | can it reach `registry.yaml` | The catalogue falls back to the cache and then to the embedded snapshot, so this is a warning too. `chaps registry show` says which one is in use. |
| `network releases` | can it reach the chap-core release list | `chaps update` needs it; everything else works without it. The answer doubles as the newest release, which `chap-core pin` below compares against. |
| `github api` | how much of this address's hour is left on GitHub's REST API | `GET /rate_limit`, which GitHub documents as not counting against the limit it reports. Unauthenticated that limit is 60 requests an hour **per address**, which the release lookup, the `chap-core pin` line, every `registry pin <id>` line and every `chaps models add` all spend - a busy operator, or a CI runner sharing its egress address, runs out before lunch. The line says which of the two limits this is: `4990 of 5000 requests left this hour (token)`, or `43 of 60 requests left this hour (no token; set GITHUB_TOKEN for 5000)`. It warns when nothing is left, with the clock time the window resets at; it never fails, because every check that needs GitHub already degrades to a skip with the reason on its own line. See [a used-up rate limit](./troubleshooting.md#githubs-rate-limit-is-used-up). |
| `chaps` | version, target, install method, newer release | Warns when a newer release exists: `chaps self update`. Under `--offline` the line still names the build, as a `skip`: whether there is an update is the one thing this run did not ask. |

An anonymous request to `ghcr.io/v2/` answers `401`, which is the registry
answering; any HTTP status counts as reachable, because a host that refuses us
is still a host this machine can reach.

### Raising the GitHub limit

Set `GITHUB_TOKEN` (or `GH_TOKEN`, which is what the `gh` CLI uses) and every
GitHub request `chaps` makes carries it, for 5000 requests an hour instead of
60:

```sh
GITHUB_TOKEN=$(gh auth token) chaps doctor
```

```text
ok    github api                        reachable, 4990 of 5000 requests left this hour (token)
ok    registry pin chapkit_ewars_model  sha-8d4a7ea is the newest build on main
```

A classic token with no scopes at all, or a fine-grained token with read
access to public repositories, is enough: everything `chaps` asks GitHub is a
public read. `chaps` never writes the token anywhere - not to `.chaps/`, not
to `.env`, not to the cache - and never prints it: a `-v` trace says
`Authorization: Bearer <token>` and nothing more. Nothing on the command line
sets it, because a token on a command line is a token in the shell history.

## Project checks

These only run when the command found a deployment, which it does the way git
finds `.git`: from the current directory (or `-C DIR`) upwards. Outside one,
`doctor` prints the machine half and a single line saying so:

```text
project: none here (run chaps doctor inside a deployment directory for more)
```

| Check | What it asks | What a bad answer means |
| --- | --- | --- |
| `project` | what compose calls this deployment, and whether that name is its own | Warns while the name is only the directory name, which another deployment in another directory of the same name also has - they then share every container name and every named volume. `chaps sync` writes the name down unchanged, so nothing is renamed and no volume is orphaned; only a deployment created by `chaps init` from this version gets a name with a suffix of its own. See [the compose project name](./concepts.md#the-compose-project-name). |
| `volumes` | which named volumes docker holds under this deployment's name, whether the database one is this deployment's, and whether the rest still belong to something it enables | Warns when `<project>_chap-db` was created before `.chaps/project.yaml` was: it belongs to something else, most often an earlier deployment of the same name, and it still holds that deployment's role password - which is exactly the state in which chap-core cannot log in and never becomes healthy. That comparison is skipped where the filesystem records no creation time. Failing that, it warns about leftover volumes: a `ck_<id>_data` whose model is no longer in `.chaps/models.yaml`, or an `ocs_data` or `s3_data` whose component is off. Their data is kept on purpose, so that a model or component enabled again finds it, but nothing declares those volumes any more and `chaps down --volumes` no longer reaches them - `chaps models disable <id> --purge`, `chaps components disable <name> --purge` or `docker volume rm <name>` is what removes one. While the `ocs` container is down, the line also says how much its data volume holds (`; mychap-1ab2c3_ocs_data holds 3.0 GB`), so an operator can see how much data a `chaps down --volumes` would destroy before typing it. While that container is up the size is not measured here at all, because the `components` line already has it from inside the container. The size comes from a `du -sk` in a throwaway busybox mounting the volume read-only, bounded at ten seconds, because neither `docker volume inspect` nor `docker system df -v` reports one reliably. |
| `project files` | are `.chaps/project.yaml`, `.chaps/models.yaml`, `.chaps/components.yaml`, `.env`, `compose.yml`, `compose.chaps.yml` and `compose.marketplace.yml` all there, plus one file per enabled component | It names the missing ones. `chaps sync` renders the compose files again; `chaps init --force` writes the whole deployment. A deployment with `chap-core` disabled is not asked for the base compose file, which it does not have. |
| `components` | which components this deployment has, and whether the files they need are there | Fails when the `ocs` component is on and `ocs/climate-service.yaml` is missing: the container would start with no instance to be. Warns while that file still holds OCS's example values, so nobody deploys Sierra Leone by accident - edit it and delete the note at the top. Once the file is the operator's own, the line also reports two things it never judges: `ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)` when `.env` sets none of the five data source variables, and `plugins/: N files` when the deployment has an `ocs/plugins/` directory. Neither is a problem - a deployment with no credentials serves WorldPop and CHIRPS3 perfectly well - so they are information on an `ok` line. While the container is running, the line ends with the same two facts [`chaps status`](./status.md) puts on the OCS line: `3 datasets` from `GET /datasets?f=json` and `212.0 MB data` from `du -sk /app/data` inside the container. Both are bounded and both are silent when they could not be had. See [Components](./components.md). |
| `compose files` | `chaps sync --check`, without its exit code | The rendered files no longer match `.chaps/`, which is drift the next `chaps up` would undo anyway. Run `chaps sync`. |
| `.env` | is it readable, and does it still say what it should | Warns when `POSTGRES_PASSWORD` is the default `chap` or unset, and when the `CHAP_IMAGE_TAG` pin comment is gone. On a protected deployment it also warns when `compose.chaps.yml` does not pass `SERVICEKIT_REGISTRATION_KEY` to chap-core, which is only possible for one rendered by an older `chaps`: without it every model registration is answered with a 401. Whether authentication is on is reported either way: `auth: off` is a fact to be able to see, not a fault. |
| `api port`, `port <model>`, `port <component>` | is every host port the deployment publishes free | Fails when something else holds one, with the same three ways out `chaps up`'s preflight offers. A port held by this project's own running container is not a conflict, exactly as `up` treats it. See [Ports](./ports.md). |
| `manual <id>` | is a model added with `chaps models add` still on the newest build its branch has published (one line per such model) | Warns when the branch has moved on: `chaps update` moves the pin. A model added from an image reference is pinned on purpose and skipped as such, and a repository or registry that would not answer is a check that was not made rather than a fault in the deployment. Skipped under `--offline`. See [Models outside the marketplace](./models.md#models-outside-the-marketplace). |
| `chap-core pin` | is the pinned tag still the newest release (left out when `chap-core` is not a component of this deployment) | Warns when a newer chap-core has been released: `chaps update --dry-run` says what would move. A moving tag (`latest`, `master`, `dev`) is not behind anything, so it passes whether or not the list was read, and says which build it is on instead - `dev (moving tag, running 7f3a1c2e9b4d)` - plus the two commands that leave it: `chaps update` re-pulls it, `chaps update --pin-chap-core` pins a release. The digest comes from the image chap-core's container runs and is left out when nothing is running. See [Switching chap-core's tag](./updating.md#switching-chap-cores-tag). Skipped when the release list was not reachable, and under `--offline`, where it was never asked for. |
| `image <model>` | does each enabled model's exact tag exist on ghcr, with a linux/amd64 image | `docker manifest inspect` per model, in parallel, ten seconds each. A tag that is gone fails and names the model to re-resolve with `chaps models enable <id>`. Skipped, with the reason on the line, under `--offline`, when there is no docker CLI to ask through, and when `ghcr.io` was unreachable. `--offline` is the reason given first, so the line reads the same on a machine with Docker and one without. |
| `registry pin <id>` | is the commit the marketplace pins for each enabled model still the newest build that model's own repository has published (one line per enabled marketplace model) | Warns when the catalogue is behind: the model repository has built a newer commit than the version this deployment's channel resolves to, which is the state in which every deployment runs a fortnight-old image without anything saying so. No `chaps` command moves that pin - it lives in the marketplace, and `chaps models add` refuses an id the catalogue already lists - so the line asks for a new pin there. A commit the branch has not built yet, and one taken from a branch or a tag of its own, are ahead rather than behind and pass. The default branch, one commit listing and at most five registry lookups per model, run in parallel; a repository that will not answer, a rate limit and a branch with no published build are all skips with the reason on the line. Models added with `chaps models add` are left out, because `manual <id>` above already asks the same question. Skipped under `--offline`. See [Channels and versions](./models.md#channels-and-versions). |
| `user <model>` | does the account the overlay runs each enabled model as still match what its image declares (one line per enabled model) | `docker image inspect --platform linux/amd64` of the exact tag the overlay pins, read from the local daemon only - no network, so this line is answered under `--offline` too. Warns when the two have drifted apart, which is what a `user:` written by an older `chaps` (or by hand) looks like: an image that runs as root under `user: 1000:1000` starts fine and then fails on its own binaries. `chaps models enable <id>` reads the account off the image again, and `chaps update` does it for a model it moves to a new tag. Skipped when the amd64 variant of the tag is not in this machine's image store - the line names the `chaps docker pull` that would put it there, which is also what an arm64 host holding only its own architecture is told - when there is no docker CLI to ask through, and when the image runs as an account name only the image itself can turn into numbers. See [Data directories and users](./models.md#data-directories-and-users). |
| `image <component>` | does each enabled component's image exist | The same `docker manifest inspect`, without the linux/amd64 question: both component images are multi-arch and no component pins a platform, so "the tag exists" is the whole answer. Skipped for the same three reasons. |
| `health` | chap-core's health and whether every model registered | Skipped when no container of this project is running, which is `chaps up`. Otherwise it is the verdict of [`chaps status`](./status.md) as one line. A `chap` container that is up and failing its healthcheck is reported as such, with the last error line from its own log and the fix that goes with it. A deployment without chap-core is judged by its components instead. When every model **is** registered the line is `ok` and still carries a next step - `chaps models test --all` - because registration is only a heartbeat: the check that settles whether a model can predict has to run the models, which takes minutes and is therefore not something `doctor` does itself. See [Testing a model](./models.md#testing-a-model). |

## `--json`

The same checklist as one document: an array of checks and the counts they add
up to.

```sh
chaps doctor --json
```

```json
{
  "checks": [
    {
      "id": "docker-cli",
      "name": "docker cli",
      "status": "ok",
      "detail": "docker 29.8.1",
      "fix": null
    },
    {
      "id": "api-port",
      "name": "api port",
      "status": "fail",
      "detail": "8000 is in use by something else",
      "fix": "port 8000 is already in use on this machine (needed by chap); free it, or run `chaps init --api-port 8001 --force` here / set CHAP_API_PORT=8001 in .env"
    }
  ],
  "summary": { "ok": 1, "warn": 0, "fail": 1, "skip": 0 }
}
```

`id` is stable, so a script can pick one line out of the report:

```sh
chaps doctor --json | jq -r '.checks[] | select(.status == "fail") | "\(.id): \(.detail)"'
```

Outside a deployment directory the project checks are simply absent from
`checks`, which is how a consumer tells the two cases apart.

## `--offline`

`--offline` skips every check that would touch the network, reporting each as
`skip` with the reason, and the reason always names the flag:

```text
skip  network ghcr.io                   --offline: the image registry was not probed
skip  network marketplace               --offline: the model catalogue was not probed
skip  network releases                  --offline: the chap-core release list was not probed
skip  github api                        --offline: the GitHub API rate limit was not asked
skip  chaps                             v0.2.0, aarch64-apple-darwin, release archive; --offline: the release list was not asked
skip  chap-core pin                     v2.3.1 pinned; --offline: the release list was not asked
skip  image chapkit-ewars-model         --offline: the registry was not asked
skip  registry pin chapkit_ewars_model  --offline, so the model repositories were not asked about newer builds
```

`--offline` is the reason on every one of those lines whatever else the
machine has: a host without a docker CLI has no image manifest to ask for
either, but the flag is the reason the user gave, so it is the reason
reported, and the missing CLI is already a failure on the `docker cli` line.
That makes the offline report the same everywhere, which is what a test or a
CI step can rely on.

Everything else still runs, so `chaps doctor --offline` is the whole local
half of the checklist on a machine with no way out.
