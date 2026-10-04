---
marp: true
theme: chaps
paginate: true
footer: chaps - deploy Chap with Docker
title: chaps - an overview
description: What chaps is, what it deploys and how it works, for technical people
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# Deploying Chap with one binary

## The Climate Health Analytics Platform, its models, OCS and DHIS2, with Docker

An overview for technical people

---

## The problem

To run Chap today, you need many parts that work together:

- **chap-core**: the API, a worker, PostgreSQL and Redis
- **Forecasting models**: each one is a container with its own image
- **Open Climate Service (OCS)**: climate data, with an S3 store
- **DHIS2**: with the Modeling App and the Climate App

Each part has its own compose file, its own ports and its own settings. You
must keep the versions in step, and you must connect the parts.

---

## What chaps is

- **One static binary.** You need Docker and Compose. You do not need Python,
  uv or a checkout of chap-core.
- **It writes a deployment directory**: compose files, `.env` and `.chaps/`.
- **It runs the deployment** with `docker compose`, and tells you what it did
  and what to do next.
- **Linux and macOS.** On Windows, use WSL.

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps doctor
```

---

## The first deployment: three commands

```sh
chaps init mychap --models default
cd mychap
chaps up --wait
```

```text
chap-core   up   http://localhost:8700   2.3.1   auth: off

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  4s ago

1 model registered
```

`chaps status` says "up" only when chap-core answered, not when compose
returned.

---

## What a deployment can hold

| Component | What it is | Turn it on |
| --- | --- | --- |
| `chap-core` | Chap itself, on by default | `--without chap-core` turns it off |
| models | marketplace models, or your own | `--models ID,ID`, `chaps models enable` |
| `ocs` | Open Climate Service | `--with ocs` |
| `s3` | the object store for OCS | `--with ocs,s3` |
| `dhis2` | a DHIS2 with its own database | `--with dhis2` |

Any part can run on its own: `--only ocs,s3`, `--only dhis2`, or one model with
`chaps run`.

---

## How it works

<div class="columns">
<div>

**The state** is YAML in `.chaps/`:

- `project.yaml`: the pins and ports
- `models.yaml`: the enabled models
- `components.yaml`: the components

You change the state with commands, never by hand.

</div>
<div>

**The compose files** are rendered from the state by `chaps sync`:

- `compose.yml`: chap-core's own file, byte for byte
- `compose.chaps.yml`: chaps' changes
- one overlay for each model

`chaps up` runs `sync`, then `docker compose up`.

</div>
</div>

---

## Versions: up, update, restart

| Command | What it does |
| --- | --- |
| `chaps up` | Starts what is on disk. Changes no pin. |
| `chaps update` | Moves the pins to what upstream publishes now, and pulls. Touches no container. |
| `chaps restart` | Recreates the services whose image or configuration changed. |

So `up` and `update` are safe to run at any time. `chaps update --chap-tag`
moves chap-core to a release, `latest`, `master` or `dev`, and asks first when
the move goes backwards.

---

## Operating a deployment

```sh
chaps doctor              # a checklist: docker, ports, disk, pins, images
chaps status              # chap-core health and which models registered
chaps logs -f chap        # the logs of one service
chaps jobs                # the backtests and predictions, and why one failed
chaps backup create       # the database, the model data and the files
chaps auth enable         # an API token for chap-core
chaps top                 # every deployment on this machine, live
```

Each command ends with what it found and the next command.

---

## One model, no deployment

```sh
chaps run chapkit_ewars_model
```

```text
starting chapkit-ewars-model (chapkit_ewars_model in ~/.local/share/chaps/run/default)
running chapkit_ewars_model on http://localhost:5001 (answered in 41s)
stop it with `chaps stop chapkit_ewars_model`
```

- A marketplace id, a GitHub URL, a ghcr image or a local image
- `chaps ps` lists them, `chaps stop` stops them
- **A first run downloads the model's image:** 1 to 7 GB for each model

---

## The chap CLI, without Python

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
chaps chap plot-backtest ewars.nc --output-file ewars.html
```

- The `chap` CLI runs in chap-core's own image.
- Your directory is mounted, so the files are on your machine.
- A model id starts that model. No `chaps up`, no chap-core.

---

## DHIS2 and the Modeling App

```sh
chaps init mychap --with dhis2 --models default
cd mychap && chaps up
chaps dhis2 connect
```

`chaps dhis2 connect` does three things:

1. It creates the `chap` route from DHIS2 to chap-core.
2. It installs the Modeling App and the Climate App.
3. It generates the analytics tables.

`chaps dhis2 use URL` does the same for a DHIS2 that runs elsewhere.

---

## Rules chaps keeps

- **Every command reports.** It says what it found, what changed and what to
  do next. No command exits in silence.
- **Messages name the file or command,** and give the way out on the same line.
- **Nothing needs the network that does not have to.** `--offline` works from
  the cached or embedded marketplace.
- **The `.env` is written once.** The database password in it outlives
  everything else.

---

## Where to read more

- The book: **https://winterop-com.github.io/chaps/**
- The code: **https://github.com/winterop-com/chaps**
- For an AI assistant: the "For AI assistants" chapter tells it how to set up
  chaps for you, command by command.

```sh
chaps --help
chaps doctor
```
