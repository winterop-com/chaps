---
marp: true
theme: varde
paginate: true
footer: varde - the chap CLI without Python
title: The chap CLI, without Python
description: For people who use `uvx --from chap-core chap`: the same commands through varde chap, and how it works
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# The chap CLI, without Python

## `uvx --from chap-core chap` becomes `varde chap`

For people who use the chap CLI already

---

## What this deck covers

1. What changes for you, and what does not
2. How `varde chap` runs chap: the container, the mounts, the user
3. Which image and which chap-core version it runs
4. Models: GitHub, directories, URLs, and marketplace ids
5. Models that run in docker
6. Your files: what is visible, what is written, what varde reports
7. Every message that stops a run, and what to do
8. A full example: three models, compared
9. Troubleshooting

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

## What you do with varde

```sh
varde chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file r.nc
varde chap plot-backtest r.nc --output-file r.html
```

- **The same commands, the same options.** Each argument after `chap` goes to
  chap unchanged.
- You need **Docker and nothing more**. chap runs in chap-core's own image.
- R, INLA and uv are in the image already.

---

## One change of words

| Today | With varde |
| --- | --- |
| `chap eval ...` | `varde chap eval ...` |
| `chap plot-backtest ...` | `varde chap plot-backtest ...` |
| `chap export-metrics ...` | `varde chap export-metrics ...` |
| `chap validate data.csv` | `varde chap validate data.csv` |
| `chap --help` | `varde chap` |
| `chap eval --help` | `varde chap eval --help` |
| `uvx --from chap-core==2.3.1 chap ...` | `varde chap --tag v2.3.1 ...` |

---

## The options of varde itself

They come **between** `chap` and chap's own command:

| Option | What it does |
| --- | --- |
| `--tag TAG` | The chap-core version to run |
| `--image core` or `--image worker` | The image to run in |
| `--group NAME` | The `varde run` group whose models chap can reach |
| `--stop` | Stop the model that this run started, after the run |
| `--timeout SECONDS` | How long to wait for a model that varde starts (300) |
| `--docker` | Give the container the docker socket |

For example: `varde chap --tag master --docker eval ...`. `varde chap --help`
shows these options, and `varde chap` alone shows chap's own help.

---

## Under the hood: one `docker run`

```text
docker run --rm -i [-t] --init --platform linux/amd64
  --user <your uid>:<your gid>
  -v <cwd>:<cwd> -w <cwd>
  -v <data>/chap:<data>/chap
  -e HOME=/tmp -e CHAP_RUNS_DIR=<data>/chap/runs ...
  [--network <deployment>_default]
  --label com.winterop.varde.kind=cli
  ghcr.io/dhis2-chap/chap-worker:v2.3.1 chap eval ...
```

- `--rm`: the container goes when chap stops. `--init` passes Ctrl-C on.
- `--platform linux/amd64`: the chap-core images are amd64. On Apple
  silicon, they run under emulation.
- `-t` only when your terminal is a terminal, so a pipe works too.

---

## The same path, inside and outside

`-v <cwd>:<cwd> -w <cwd>` mounts your directory **at the same path**:

- `--dataset-csv laos_subset.csv` reads your file, and finds the
  `.geojson` beside it, as before.
- `--output-file r.nc` writes into your directory.
- An absolute path in the output means the same file on both sides.

`--user <uid>:<gid>` runs chap as you, so the files belong to you, not to
root.

---

## The caches: the second run is faster

`<data>` is varde's data directory: `~/.local/share/varde`, or
`$VARDE_DATA_DIR`. varde sets:

| Variable | Value | What it keeps |
| --- | --- | --- |
| `CHAP_RUNS_DIR` | `<data>/chap/runs` | one directory for each model |
| `UV_CACHE_DIR` | `<data>/chap/cache/uv` | Python packages |
| `UV_PYTHON_INSTALL_DIR` | `<data>/chap/cache/python` | Python versions |
| `RENV_PATHS_ROOT` | `<data>/chap/cache/renv` | R packages |
| `RENV_CONFIG_SANDBOX_ENABLED` | `FALSE` | no read-only sandbox |
| `MPLCONFIGDIR` | `<data>/chap/cache/matplotlib` | the font cache |

With the sandbox off, `rm -rf ~/.local/share/varde/chap` deletes all of it.

---

## Which image

| `--model-name` | Image | Size on disk |
| --- | --- | --- |
| a GitHub repository, a model directory | `chap-worker` | about 12 GB |
| a chapkit URL, a model id | `chap-core` | about 2.4 GB |
| no model (`plot-backtest`, `export-metrics`) | `chap-core` | about 2.4 GB |
| anything, in a deployment | `chap-worker` | already there |

- The worker image has uv, R and INLA. chap-core's own worker runs models in
  it.
- `--image core` or `--image worker` overrides the choice.
- `--image core` with a GitHub model gets a warning first: an R model fails
  there with `Rscript: not found`.

---

## Which chap-core version

1. `--tag TAG`, when you give it.
2. In a deployment: **the deployment's tag**, so the CLI and the server are
   the same version.
3. Outside a deployment: **the newest chap-core release**, from GitHub.
4. With `--offline`: the newest image of that kind on this machine.

If GitHub does not answer, varde uses the newest local image, and says so:

```text
warning: the newest chap-core release could not be read; using v2.3.1, the
newest image on this machine
```

---

## The lines before the run

```text
running `chap eval` in ghcr.io/dhis2-chap/chap-worker:v2.3.1; the first run pulls it, about 12 GB
files: chap reads and writes in /home/me/work
model: chapkit_ewars_model at http://chapkit-ewars-model:8000 answers
```

- The image and its tag, and the size when it is not local yet.
- The directories that chap can see.
- The model, when varde checked that it answers.

A run that only asks for help (`varde chap`, `--help`, `--version`) gets
only the first line.

---

## Models: four kinds

| `--model-name` | What happens |
| --- | --- |
| `https://github.com/...` | chap clones it, and runs it in the worker image |
| `./my_model` (a directory) | chap runs it from your directory |
| `http://...` (a chapkit service) | varde checks that it answers, then chap calls it |
| **`chapkit_ewars_model` (an id)** | **varde starts the model, then chap calls it** |

A word is a model id only when no file or directory of that name is in the
current directory. A directory of the same name wins, as it does for chap.

---

## A model id: varde starts the model

Outside a deployment, varde starts the model in a `varde run` group:

```text
starting chapkit-ewars-model (chapkit_ewars_model in ~/.local/share/varde/run/default)
running `chap eval` in ghcr.io/dhis2-chap/chap-core:v2.3.1
model: chapkit_ewars_model at http://chapkit-ewars-model:8000 answers
chap finished; it wrote ewars.nc
chapkit_ewars_model keeps running for the next run; `varde stop chapkit_ewars_model`
stops it, and `--stop` stops it after a run
```

The container joins the group's network, where the model has its service
name. varde gives chap that URL in place of the id.

---

## A model id in a deployment: no `varde up`

```sh
cd ~/mychap && mkdir -p eval && cd eval
varde chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

```text
starting chapkit-ewars-model for this run, without chap-core
```

- varde starts **only that model**, with `--no-deps`: chap-core, postgres and
  redis stay down.
- For this start, the model gets no registration settings, so it does not wait
  for a chap-core that does not run.
- A later `varde up` recreates the model, and it registers as usual.

---

## Keep it running, or stop it

| Situation | After the run |
| --- | --- |
| varde started the model | It keeps running, so the next run starts at once |
| varde started it, with `--stop` | varde stops it |
| the model ran before | varde never stops it |

```text
chapkit_ewars_model ran before this run, so --stop left it running
```

`--timeout SECONDS` (300) is how long varde waits for a model that it
starts. A first start pulls the model's image (1 to 7 GB), and varde shows
that pull.

---

## A chapkit URL: checked before chap starts

varde asks the URL for `/api/v1/info`, from a container on the same network.
If nothing answers, chap is not started:

```text
error: the model server at http://nothing:8000 is not running (no answer on
/api/v1/info), so chap was not started; `nothing` is not a model of this
deployment or group. Give a model id instead (`--model-name
chapkit_ewars_model`) and varde starts it, or start one with `varde run ID`;
the `models:` line lists the URLs that answer
```

For a server elsewhere, the message ends with "check that the server runs and
that this machine can reach it".

---

## Not `localhost`

`varde ps` shows a model at `http://localhost:5001`. That URL works on your
machine. In the container, `localhost` is the container itself:

```text
error: `http://localhost:5001` is this machine, and in the container
`localhost` is the container itself; use `--model-name
http://auto-arima-chapkit:8000`, the same model on the network
```

When varde knows which model has that port, the message names its service
URL. Otherwise, it tells you to give the model id.

---

## Models that run in docker

A model with `docker_env` in its `MLproject` starts a container of its own.
varde reads the `MLproject` first (from GitHub, or from your directory):

```text
error: `https://github.com/dhis2-chap/chap_auto_ewars` runs in docker
(`docker_env` in its MLproject), and the container has no docker socket;
run it again with `varde chap --docker ...`
```

```sh
varde chap --docker eval --model-name https://github.com/dhis2-chap/chap_auto_ewars \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

---

## What `--docker` does

- It mounts `/var/run/docker.sock` in the container. Docker Desktop, Colima
  and OrbStack all answer that path.
- It adds the socket's group to the container user, as the container sees it
  (on Docker Desktop, group 0).
- The run directories are at the same path on both sides, so the paths that
  chap gives docker are real paths on your machine.

```text
warning: --docker gives the container control of docker on this machine
```

The socket is full control of docker. So `--docker` is never the default.

---

## Your files: what varde does for you

- **Output directories:** varde makes the directory of each `--output-file`
  that does not exist. chap does not, and it fails with a traceback.
- **Paths outside the directory:** `../out/r.nc`, `/data/r.nc` or
  `--output-file=/data/r.nc`. varde mounts the nearest existing directory
  above the path, at the same path.
- **The deployment directory:** if you run `varde chap` there, varde tells
  you that a subdirectory (`mkdir eval && cd eval`) keeps chap's files apart
  from the compose files.

---

## What varde reports after the run

```text
chap finished; it wrote ewars.nc
`varde chap plot-backtest ewars.nc --output-file ewars.html` plots it
```

- varde looks at the mounted directories before and after the run, and names
  the files that are new or changed.
- It also names the files that the arguments name, wherever they are.
- For a `.nc` file, it gives the command that plots it.
- When chap fails, varde exits with chap's own status:

```text
error: chap exited with status 1; its own message is above
```

---

## Every message that stops a run

| varde says | What to do |
| --- | --- |
| `--json does not apply` | Drop `--json`: the output is chap's own |
| `` `./x` is not a directory here `` | Give a directory, a GitHub URL or a model id |
| `is not a model of the deployment` | `varde models enable ID`, then run again |
| `there is no marketplace model` | `varde models search WORD` finds the id |
| ``there is no `varde run` group called`` | `varde ps` lists the groups |
| `no directory above it exists to mount` | Make the directory first |
| `--offline needs a ... image on this machine` | Run once without `--offline`, or give `--tag` |
| `` `chap plot-dataset` shows its plot in a browser `` | Use `plot-backtest`, which writes a file |

---

## chap's commands in a container

| Command | In `varde chap` |
| --- | --- |
| `eval` | yes, with every kind of model |
| `plot-backtest` | yes, it writes a file |
| `export-metrics` | yes |
| `validate` | yes |
| `test` | yes |
| `plot-dataset` | **no**: it opens a browser, and a container has none |

varde refuses `plot-dataset` before the run and names `plot-backtest`.

---

## A full example: three models, compared (1)

Get the data, and check it:

```sh
mkdir -p ~/compare && cd ~/compare
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.csv
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.geojson
varde chap validate laos_subset.csv
```

```text
Validation passed: no issues found.
```

---

## A full example: three models, compared (2)

```sh
varde chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file out/r.nc \
  --backtest-params.n-splits 3 --backtest-params.n-periods 3
varde chap eval --model-name auto_arima_chapkit \
  --dataset-csv laos_subset.csv --output-file out/arima.nc \
  --backtest-params.n-splits 3 --backtest-params.n-periods 3
varde chap --stop eval --model-name chapkit_simple_multistep_model \
  --dataset-csv laos_subset.csv --output-file out/multistep.nc \
  --backtest-params.n-splits 3 --backtest-params.n-periods 3
```

varde makes `out/` with the first run. The third run stops its model after.

---

## A full example: three models, compared (3)

```sh
varde chap export-metrics out/r.nc out/arima.nc out/multistep.nc \
  --output-file out/comparison.csv
varde chap plot-backtest out/arima.nc --output-file out/arima.html
varde stop auto_arima_chapkit
```

- `comparison.csv` has one row for each model: `crps`, `mae`, `rmse`,
  `coverage_10_90` and more. Lower is better for the errors.
- chap can print `RuntimeWarning` lines about `log1p`. They do not stop the
  run.

---

## Troubleshooting

| You see | Cause and fix |
| --- | --- |
| `Cannot reach the Docker daemon. The model ... requires Docker` | A `docker_env` model ran without the socket. Add `--docker`. |
| `Rscript: not found` | An R model in the `chap-core` image. Drop `--image core`. |
| `FileNotFoundError: [Errno 2]` | An output directory that varde could not make. Make it, then run again. |
| `did not answer on http://...:8000 within 300s` | A slow first start. Give a longer `--timeout`. |
| `docker run exited with status 125` | The image did not pull. Check the tag, or run `varde doctor`. |

Each message is in the book's Troubleshooting chapter, with more detail.

---

## Start now

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/varde/main/install.sh | sh
varde doctor
varde chap
```

- The full guide, from install to a comparison of two models:
  **https://winterop-com.github.io/varde/use-cases/evaluate-with-chap-cli.html**
- The chapter about `varde chap`:
  **https://winterop-com.github.io/varde/chap-cli.html**
