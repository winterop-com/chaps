---
marp: true
theme: chaps
paginate: true
footer: chaps - for model developers
title: Developing a model with chaps
description: For people who write forecasting models: run, test and evaluate your model with Chap
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# Developing a model with chaps

## Run it, test it, evaluate it, with Chap around it

For people who write forecasting models

---

## The loop of a model developer

1. **Run** the model: from its checkout, from an image you built, or from
   GitHub.
2. **Test** that it trains and forecasts at all.
3. **Evaluate** it with a backtest, on sample data and on real data.
4. **Compare** it with the models that exist.

chaps does each step with one command, and puts chap-core around the model
only when the step needs it.

---

## Run a model: four sources

```sh
chaps run chapkit_ewars_model                              # a marketplace id
chaps run https://github.com/chap-models/chapkit_ghr_model # a GitHub repository
chaps run ghcr.io/my-org/my_model:sha-1eb8cf1              # a published image
chaps run my-model:dev                                     # an image built here
```

```text
running chapkit_ewars_model on http://localhost:5001 (answered in 41s)
```

- No deployment directory: the model runs in a `chaps run` group.
- The first run of a model downloads its image: 1 GB for a Python model, 6 to
  7 GB for an R model.
- `chaps ps` lists the models, `chaps stop ID` stops one.

---

## An image you built yourself

```sh
docker build --platform linux/amd64 -t my-model:dev .
chaps run my-model:dev
```

In a deployment, with chap-core:

```sh
chaps models add my-model:dev
chaps up
chaps models test my_model
```

- A local image is never pulled: chaps starts what is in your image store.
- Build again with the same tag, then `chaps restart`, to use the new build.

---

## Your model from its checkout

chaps runs chap-core. You run the model with `uv run`, a debugger or hot
reload:

```sh
chaps init mychap --models none && cd mychap && chaps up
```

```sh
export SERVICEKIT_ORCHESTRATOR_URL='http://localhost:8700/v2/services/$register'
export SERVICEKIT_HOST=host.docker.internal
export SERVICEKIT_PORT=8001
uv run uvicorn main:app --host 0.0.0.0 --port 8001
```

**It worked when** `chaps status` lists the model with a recent LAST PING.

---

## Test: does it work at all?

```sh
chaps models test my_model             # chapkit test, in the model's own container
chaps models test my_model --backtest  # the whole way, through chap-core
chaps models test --all                # every model of the deployment
```

- `chaps status` can be all green while a model cannot predict at all:
  registration is a heartbeat.
- The model level trains and predicts on data the model generates itself.
- `--backtest` makes a dataset, runs a backtest and reads its scores, the way
  the Modeling App does.

---

## Evaluate on real data

In a subdirectory of the deployment (`mkdir eval && cd eval`):

```sh
chaps chap eval --model-name my_model \
  --dataset-csv laos_subset.csv --output-file mine.nc
```

Or straight from the checkout, with no deployment:

```sh
chaps chap eval --model-name ./my_model_checkout \
  --dataset-csv laos_subset.csv --output-file mine_local.nc
```

- A model id of the deployment: chaps starts that model, without chap-core.
- A directory with an `MLproject`: chap runs it in the worker image (uv, R,
  INLA). Add `--docker` when the `MLproject` uses `docker_env`.

---

## Compare with the models that exist

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
chaps chap export-metrics mine.nc ewars.nc --output-file comparison.csv
chaps chap plot-backtest mine.nc --output-file mine.html
```

One row for each model: `crps`, `mae`, `rmse`, the interval coverage and more.

---

## The marketplace

```sh
chaps models list               # what is published
chaps models search ewars       # find one
chaps models info chapkit_ewars_model
```

- A model in the marketplace is a version-pinned image with its metadata.
- `chaps models enable ID --version V` pins one version in a deployment.
- `chaps update` moves the pins to what the marketplace publishes now.

---

## Images and users: what chaps checks for you

- **Platform:** marketplace images are amd64 only, so chaps pins
  `linux/amd64`. On Apple silicon they run under emulation.
- **The user:** chaps reads the user from the image, and chowns the model's
  data volume to it in an init container.
- **Read-only:** the model runs with a read-only file system, a tmpfs on
  `/tmp`, and no capabilities. A model that writes outside its data directory
  fails here first, not in production.

---

## When something fails

```sh
chaps logs -f my-model        # the model's own log
chaps jobs                    # what chap-core ran, newest first
chaps jobs logs JOB_ID        # why a backtest failed: the model's stderr
chaps doctor                  # the machine and the deployment
```

A failed backtest says why in `chaps jobs logs` and nowhere else.

---

## Start now

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps run chapkit_ewars_model
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

Models and the marketplace: **https://winterop-com.github.io/chaps/models.html**
