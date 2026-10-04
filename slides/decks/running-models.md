---
marp: true
theme: chaps
paginate: true
footer: chaps - running models with chaps run
title: Running models with chaps run
description: Start a forecasting model with one command, with no deployment directory, and use its URL
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# Running models

## `chaps run`: one command starts a model and gives you its URL

No deployment directory, no compose file, no chap-core

---

## Why `chaps run`

A forecasting model is a service: a container with an HTTP API. Often you
want only that service:

- to call the model's API from a notebook or a script
- to evaluate it with `chaps chap eval`
- to try a model before you add it to a deployment
- to run two or three models side by side and compare them

`chaps init` and `chaps up` make a full deployment. `chaps run` makes nothing
that you must keep.

---

## The first model

```sh
chaps run chapkit_ewars_model
```

```text
starting chapkit-ewars-model (chapkit_ewars_model in
  /home/me/.local/share/chaps/run/default)
running chapkit_ewars_model on http://localhost:5001 (answered in 41s)
stop it with `chaps stop chapkit_ewars_model`;
  `chaps -C /home/me/.local/share/chaps/run/default logs chapkit-ewars-model`
  shows its log
```

- The command returns when the model answers its own `/health`.
- So the URL works when the line is printed.

---

## What the model can be

| `MODEL` | What `chaps run` does |
| --- | --- |
| a marketplace id: `chapkit_ewars_model` | enables it at the catalogue's stable version |
| a GitHub repository the marketplace lists | the same: the catalogue's reviewed pin |
| any other GitHub repository URL | adds it: the newest published `sha-` build of the default branch |
| a ghcr image with a tag or digest | pins exactly that image |
| a local image: `name:tag` | runs the image from your image store, never pulled |

A word with no `:`, `/` or `@` is a marketplace id, never a local image.

---

## Four ways to start a model

```sh
chaps run chapkit_ewars_model
chaps run https://github.com/chap-models/chapkit_ghr_model
chaps run ghcr.io/my-org/chapkit_dengue_model:sha-1eb8cf1
chaps run my-dengue-model:dev
```

- A first run downloads the model's image: 1 GB for a Python model, 6 to 7 GB
  for an R model.
- A second `chaps run` of the same model starts it as it is. It does not move
  its version or its port.

---

## Where the model runs: groups

Outside a deployment, `chaps run` keeps its models in a **group**:

- a deployment that chaps owns, in `~/.local/share/chaps/run/<group>/`
- with no chap-core and no component: only model services
- the first `chaps run` into a group creates it
- `--group NAME` selects a group; the default group is `default`

Each group is a compose project of its own. Its containers, volumes and
network carry its name, so a stop in one group touches nothing else.

---

## Two groups side by side

```sh
chaps run chapkit_ewars_model                       # group default
chaps run auto_arima_chapkit --group trial
chaps run chapkit_ghr_model --group trial
chaps ps
```

```text
GROUP    ID                  SERVICE             STATE        URL
default  chapkit_ewars_model chapkit-ewars-model up           http://localhost:5001
trial    auto_arima_chapkit  auto-arima-chapkit  up           http://localhost:5002
trial    chapkit_ghr_model   chapkit-ghr-model   not running  http://localhost:5003
groups live in /home/me/.local/share/chaps/run
```

---

## Ports

- `--port auto`, the default, takes the lowest port in **5001-5999** that
  nothing on the machine holds and that no group gave to a model.
- So two groups never collide, also with models that are stopped.
- `--port 5050` asks for one port. A port that is in use is refused, with
  the fix.

Two `chaps run` commands at the same time wait for each other, and get
different ports.

---

## Only this machine

- A group publishes every model port on **127.0.0.1**.
- So a model that `chaps run` started is reachable from this machine, not
  from the network. A model has no login.
- `--bind 0.0.0.0` publishes one model on every address of the machine.

```sh
chaps run chapkit_ewars_model --bind 0.0.0.0
```

Do that only on a network that you trust.

---

## The states of a model

`chaps ps` gives each model the state that `chaps status` gives:

| State | What it means |
| --- | --- |
| `up` | its `/health` answers |
| `running, not answering` | the container runs, and `/health` does not answer yet, or never will |
| `not running` | the container is gone |

`chaps ps --group trial` lists one group.

---

## Waiting, and when a start fails

- `chaps run` waits up to **300 seconds** for the model to answer.
  `--timeout SECONDS` changes that. A first R model can take some minutes.
- `--no-wait` returns as soon as the container started.
- When the container does not start, chaps says what compose said:

```text
error: chapkit-ewars-model did not start (Error response from daemon:
  pull access denied ...); fix that, then
  `chaps run chapkit_ewars_model` tries again
```

- chaps then takes the model out again, so `chaps ps` does not list it.

---

## Stopping

```sh
chaps stop chapkit_ewars_model         # one model, in whichever group has it
chaps stop --group trial               # every model in one group
chaps stop --all                       # every model in every group
```

- The model's data volume stays. `chaps run ID` starts it again.
- When two groups have the same model, `chaps stop` asks for `--group`.

---

## Stopping, and removing the data

```sh
chaps stop chapkit_ewars_model --purge
chaps stop --group trial --purge
chaps stop --all --purge
```

- `--purge` also removes the data volume.
- A group that has no model left goes too: its network, its directory and
  every volume of it.
- A volume that docker will not remove keeps its group. Run the same command
  again when the volume is free.

---

## A group is a deployment

Every other chaps command works on a group with `-C`:

```sh
chaps -C ~/.local/share/chaps/run/default logs chapkit-ewars-model
chaps -C ~/.local/share/chaps/run/default status
chaps -C ~/.local/share/chaps/run/default models test chapkit_ewars_model
```

The messages from a group name their commands in that way, so you can copy
them.

---

## In a deployment of your own

- In a deployment directory (or with `-C DIR`), `chaps run`, `chaps ps` and
  `chaps stop` work on that deployment.
- `--group` is then refused: the deployment is the group.
- `chaps run` publishes the model as the deployment does.

So a tool that only wants "start this model and give me its URL" has one
code path, with or without `chaps init`.

---

## Watching everything: `chaps top`

```text
 chaps top   3 deployments  7/9 running  at 14:02:41 (every 2s)
│▾ mychap  init                     5/6 running
│  ├─ chap                          running (healthy)   1.20%   612MiB / 15.6GiB
│  └─ chapkit-ewars-model           exited
│▾ run/default  run                 1/1 running
│  └─ chapkit-ghr-model             running             0.50%   230MiB / 15.6GiB
│▸ run/trial  run                   1/2 running
 j/k move  enter fold  l logs  s stop model  r refresh  q quit
```

Every deployment and every group on the machine, live. `l` shows a log, and
`s` stops the model under the cursor.

---

## Evaluate a model that runs

A model that `chaps run` started is on its group's network. `chaps chap` joins
that network:

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

- chap reaches the model at `http://chapkit-ewars-model:8000`.
- Do not give chap the `http://localhost:5001` URL. In chap's container,
  `localhost` is that container.

---

## Or let `chaps chap` start it

Give `chaps chap` a marketplace id of a model that does not run. chaps
starts it in a group, as `chaps run` does:

```text
starting auto-arima-chapkit (auto_arima_chapkit in /home/me/.local/share/chaps/run/default)
running `chap eval` in ghcr.io/dhis2-chap/chap-core:v2.3.1
model: auto_arima_chapkit at http://auto-arima-chapkit:8000 answers
chap finished; it wrote auto_arima.nc
auto_arima_chapkit keeps running for the next run; `chaps stop auto_arima_chapkit`
stops it, and `--stop` stops it after a run
```

---

## Call the model's API yourself

The URL from `chaps run` is the model's own API:

```sh
curl -s http://localhost:5001/health
curl -s http://localhost:5001/api/v1/info
```

- `/health` answers when the model is ready.
- `/api/v1/info` says what the model is, and which data it needs.
- The model's API documentation is at `/docs`: open
  http://localhost:5001/docs in a browser.

---

## For tools: `--json`

```sh
chaps --json run chapkit_ewars_model
```

```json
{
  "ok": true,
  "id": "chapkit_ewars_model",
  "service_id": "chapkit-ewars-model",
  "port": 5001,
  "bind": "127.0.0.1",
  "url": "http://localhost:5001",
  "group": "default",
  "project_dir": "/home/me/.local/share/chaps/run/default",
  "enabled": true,
  "wait": {"ready": true, "waited_s": 41, "models": [...]}
}
```

---

## `--json` for `ps` and `stop`

- `chaps ps --json`: `{"models": [...]}`, with the same fields for each model,
  plus `state`.
- `chaps stop --json`: the models it `stopped`, the groups it `removed`, and
  the volumes that went with them, `removed_volumes`.
- A failure: `{"ok": false, "error": ..., "hint": ...}`.
- docker's own progress, such as a pull, goes to stderr. stdout is the one
  JSON document.

---

## Where the files are

```text
~/.local/share/chaps/
  run/
    default/          the default group: a chaps deployment
      .chaps/         its state: project.yaml, models.yaml
      compose.*.yml   its compose files, rendered by chaps
    trial/            the group `trial`
  chap/               the caches of `chaps chap`
```

`$CHAPS_DATA_DIR`, else `$XDG_DATA_HOME/chaps`, else `~/.local/share/chaps`.

---

## Summary

| You want | Command |
| --- | --- |
| a model and its URL | `chaps run ID` |
| the models that run | `chaps ps` |
| a live view | `chaps top` |
| its log | `chaps -C ~/.local/share/chaps/run/default logs SERVICE` |
| to evaluate it | `chaps chap eval --model-name ID ...` |
| to stop it | `chaps stop ID` |
| to remove it and its data | `chaps stop ID --purge` |

The chapter: **https://winterop-com.github.io/chaps/run.html**
