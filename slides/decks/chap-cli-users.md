---
marp: true
theme: chaps
paginate: true
footer: chaps - the chap CLI without Python
title: The chap CLI, without Python
description: For people who use `uvx --from chap-core chap`: the same commands through chaps chap
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# The chap CLI, without Python

## `uvx --from chap-core chap` becomes `chaps chap`

For people who use the chap CLI already

---

## What you do today

```sh
uv tool install chap-core --python 3.13
chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file r.nc
chap plot-backtest r.nc --output-file r.html
```

Before the first evaluation, you need:

- Python 3.13 and uv
- R, renv and INLA, for the R models
- Docker, for the `docker_env` models
- the same versions on every machine of your team

---

## What you do with chaps

```sh
chaps chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file r.nc
chaps chap plot-backtest r.nc --output-file r.html
```

- **The same commands, the same options.** Each argument after `chap` goes to
  chap unchanged.
- You need **Docker and nothing more**. chap runs in chap-core's own image.
- R, INLA and uv are in the image already.

---

## One change of words

| Today | With chaps |
| --- | --- |
| `chap eval ...` | `chaps chap eval ...` |
| `chap plot-backtest ...` | `chaps chap plot-backtest ...` |
| `chap export-metrics ...` | `chaps chap export-metrics ...` |
| `chap validate data.csv` | `chaps chap validate data.csv` |
| `chap --help` | `chaps chap` |
| `chap eval --help` | `chaps chap eval --help` |
| `uvx --from chap-core==2.3.1 chap ...` | `chaps chap --tag v2.3.1 ...` |

---

## Your files stay where they are

chaps mounts your current directory at **the same path** in the container.

- `--dataset-csv laos_subset.csv` reads your file. The `.geojson` beside it is
  found, as before.
- `--output-file r.nc` writes to your directory, and the file belongs to you.
- A path outside the directory (`../out/r.nc`, `/data/r.nc`) is mounted too.
- chaps makes the directory of an output file if it does not exist.

```text
files: chap reads and writes in /home/me/work
chap finished; it wrote r.nc
`chaps chap plot-backtest r.nc --output-file r.html` plots it
```

---

## Models: what is new

| `--model-name` | What happens |
| --- | --- |
| a GitHub repository | As before. It runs in the worker image (uv, R, INLA). |
| a model directory | As before. |
| a chapkit URL | chaps checks that it answers **before** chap starts. |
| **a marketplace id** | **chaps starts the model for you.** |

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

No `docker run`, no port, no URL to copy.

---

## A marketplace model, started for you

```text
starting chapkit-ewars-model (chapkit_ewars_model in ~/.local/share/chaps/run/default)
running `chap eval` in ghcr.io/dhis2-chap/chap-core:v2.3.1
model: chapkit_ewars_model at http://chapkit-ewars-model:8000 answers
chap finished; it wrote ewars.nc
chapkit_ewars_model keeps running for the next run; `chaps stop chapkit_ewars_model`
stops it, and `--stop` stops it after a run
```

- The model keeps running, so the next evaluation starts at once.
- `--stop` stops it after the run.
- `chaps models list` lists the ids.

---

## Models that run in docker

A model with `docker_env` in its `MLproject` starts a container of its own.

```sh
chaps chap --docker eval --model-name https://github.com/dhis2-chap/chap_auto_ewars \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

- chaps reads the `MLproject` first. Without `--docker`, it stops before the
  run and tells you to add the flag.
- `--docker` gives the container the docker socket, which is full control of
  docker. chaps warns when you give it.

---

## Fewer stack traces

chaps checks before chap starts, and says what is wrong in one line:

```text
error: the model server at http://nothing:8000 is not running (no answer on
/api/v1/info), so chap was not started; ...
```

```text
error: `http://localhost:5001` is this machine, and in the container `localhost`
is the container itself; give the model id instead ...
```

```text
error: `./no_such_model` is not a directory here; give a model directory, a
GitHub URL or a model id such as `chapkit_ewars_model`
```

---

## Versions and caches

- **Outside a deployment:** the newest chap-core release.
- **In a deployment:** the deployment's chap-core version, so the CLI and the
  server are the same.
- **`--tag master`**, or any tag, to try another version.
- **Caches:** the model runs, uv, Python and R packages are kept in
  `~/.local/share/chaps/chap`. The second run of a model installs nothing.

```sh
rm -rf ~/.local/share/chaps/chap     # to get the space back
```

---

## Compare models, as before

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
chaps chap eval --model-name auto_arima_chapkit \
  --dataset-csv laos_subset.csv --output-file arima.nc
chaps chap export-metrics ewars.nc arima.nc --output-file comparison.csv
```

One row for each model: `crps`, `mae`, `rmse`, `coverage_10_90` and more.

---

## What does not change, and what does not work

- **Does not change:** chap's own output, its options and its files.
- **`chap plot-dataset`** opens a browser, and a container has none. chaps
  says so before the run. Use `plot-backtest`, which writes a file.
- **On Apple silicon**, the images run under emulation, so a run is slower.
- **The first run downloads large images:** 2.4 GB for `chap-core`, 12 GB for
  `chap-worker`, and 1 to 7 GB for each marketplace model. chaps says so
  before it pulls. On a slow network, run `docker pull` in advance.

---

## Start now

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps doctor
chaps chap
```

The full guide, from install to a comparison of two models:
**https://winterop-com.github.io/chaps/use-cases/evaluate-with-chap-cli.html**
