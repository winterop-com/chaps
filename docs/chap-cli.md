# The chap CLI

chap-core has its own command line, `chap`. It evaluates a model on a dataset
of your own, plots the result and exports the scores. It is usually installed
with uv (`uvx --from chap-core chap ...`). That needs Python and uv, and for
many models also R.

`varde chap` runs the same `chap` in a container. You need Docker and
nothing more:

```sh
varde chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file r.nc
varde chap plot-backtest r.nc --output-file r.html
varde chap export-metrics r.nc --output-file r.csv
```

For a guide from an empty machine to a comparison of two models, see
[Evaluating a model on your own data](./use-cases/evaluate-with-chap-cli.md).

Each argument after `chap` goes to chap unchanged. To read chap's own help,
run `varde chap` with no arguments, or `varde chap eval --help` for one
command. `varde chap --help` shows only the options of varde itself. These
come between `chap` and chap's own command:

| Option | What it does |
| --- | --- |
| `--tag TAG` | The chap-core version to run. See [Which version](#which-version). |
| `--image core` or `--image worker` | The image to run in. See [Which image](#which-image). |
| `--group NAME` | The `varde run` group whose models chap can reach. See [Chapkit models](#chapkit-models-no-varde-up). |
| `--stop` | Stop the model that this run started, after the run. See [Chapkit models](#chapkit-models-no-varde-up). |
| `--timeout SECONDS` | How long varde waits for a model that it starts (300). |
| `--docker` | Give the container the docker socket. See [Models that run in docker](#models-that-run-in-docker). |

For example: `varde chap --tag master --docker eval --model-name ...`.

## Your files

varde mounts the current directory in the container at the same path. Thus:

- a relative path such as `laos_subset.csv` or `out/r.nc` means the same file
  in the container and on your machine;
- every file chap writes in the current directory is on your machine after
  the run, and belongs to your user;
- a dataset CSV finds its GeoJSON beside it, as it does without varde.

A path outside the current directory also works: an absolute path, a `../`
path, or the same after `=` (`--output-file=/data/out/r.nc`). varde mounts
the nearest existing directory above the path, at the same path. chap does not
make a directory that does not exist, so make the output directory first.

Before the run, varde writes on stderr which directories chap can see. After
the run, it names the files that are new or changed, and the next command:

```text
running `chap eval` in ghcr.io/dhis2-chap/chap-worker:v2.3.1
files: chap reads and writes in /home/me/work
...
chap finished; it wrote r.nc
`varde chap plot-backtest r.nc --output-file r.html` plots it
```

In a deployment, run `varde chap` in a subdirectory, for example `eval/`, so
that chap's files do not mix with the compose files. varde finds the
deployment from a subdirectory too. When you run it in the deployment
directory itself, varde writes a line that says so.

chap's own output is not changed. varde writes its lines on stderr, and
`--json` does not apply to this command. A run that only asks for help
(`varde chap`, `--help`, `--version`) gets only the first line.

## Which image

chap-core publishes two images with the same tags. varde selects one from
`--model-name`:

| The model | The image | Size |
| --- | --- | --- |
| A GitHub repository, or a model directory | `chap-worker` | about 12 GB |
| A chapkit service (an `http://` URL that is not GitHub, or a model id) | the image that is on this machine; else `chap-core` | |
| No model (`--help`, `plot-backtest`, `export-metrics`, ...) | the image that is on this machine; else `chap-core` | |

The worker image has uv, R and INLA, and chap-core's own worker runs models in
it, so a model that needs R always runs there. Everything else is plain chap
and HTTP, and both images can do it, so varde takes the image that this
machine has at the tag of the run, and downloads nothing:

- Only `chap-worker` is here, as in a deployment that runs chap-core: varde
  uses it.
- `chap-core` is here, or neither image is: varde uses `chap-core`, the smaller
  download (about 2.4 GB). A deployment without chap-core, such as `varde init
  --only dhis2`, does not pull the 12 GB worker image for `varde chap --help`.

`--image core` or `--image worker` overrides the choice. A model that needs R
fails in the core image with `Rscript: not found`.

The first run pulls the image. The line before the run says so, with the
size.

## Which version

- In a deployment (in its directory, or with `-C`), varde uses the
  deployment's chap-core tag. The CLI and the server are then the same
  version.
- Outside a deployment, varde uses the newest chap-core release.
- With `--offline`, or when the newest release cannot be read, varde uses the
  newest tag on this machine: of `chap-worker` when the run needs it, and else
  of either image.
- `--tag TAG` overrides all of these, for example `--tag master`.

## Chapkit models: no `varde up`

A chapkit model is a model service: chap sends it data over HTTP, and the
service trains and forecasts. `chap eval` needs only that service. It does not
need chap-core, its database or its worker. So you do not run `varde up` for
an evaluation. Give the model's id, and varde starts the model:

```sh
varde chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

What varde does with the id:

| Where you are | The id | What varde does |
| --- | --- | --- |
| In a deployment that has the model | `chapkit_ewars_model` | Starts only that model's container, with no chap-core, if it does not run. |
| In a deployment that does not have it | `auto_arima_chapkit` | Stops, and names `varde models enable auto_arima_chapkit`. varde does not change the deployment's models for an evaluation. |
| Outside a deployment | any marketplace id | Starts the model in a `varde run` group, as `varde run ID` does, and joins that group's network. `--group NAME` selects the group. |

Before the run, varde checks that the model answers. Then it gives chap the
model's URL on the network, for example `http://chapkit-ewars-model:8000`,
and writes:

```text
starting chapkit-ewars-model for this run, without chap-core
model: chapkit_ewars_model at http://chapkit-ewars-model:8000 answers
```

A model that varde started keeps running, so the next evaluation starts at
once. The last line says how to stop it. `--stop` stops it after the run
instead. A model that ran before the command is never stopped. `--timeout
SECONDS` (300) is how long varde waits for a model that it starts.

A model that varde starts in a deployment does not register with chap-core,
because chap-core does not run. If you run `varde up` later, compose recreates
the model's container, and the model then registers as usual.

A word is a model id only when no file or directory of that name is in the
current directory. A directory of the same name is used as a model directory,
as chap does.

### A model URL

You can also give a URL, for example a model on another server:
`--model-name https://models.example.org`. varde asks the URL for
`/api/v1/info` before the run. If it does not answer, varde stops with a note,
and chap does not start:

```text
error: the model server at http://nothing:8000 is not running (no answer on /api/v1/info), so chap was not started; ...
```

The URL of a model of the deployment or group, such as
`http://chapkit-ewars-model:8000`, works like its id: varde starts the model
if it does not run.

Do not use the `http://localhost:PORT` URL that `varde ps` or `varde status`
shows. In the container, `localhost` is the container itself, so that URL
reaches nothing. varde refuses such a URL before the run, and names the
service URL of the same model when it knows it.

An evaluation of a chapkit service stores a model configuration and its
artifacts in that service's own database, as it does without varde.

## Models that run in docker

A model whose `MLproject` has `docker_env` starts a container of its own. For
that, chap needs the docker socket. `--docker` gives it:

```sh
varde chap --docker eval --model-name https://github.com/dhis2-chap/chap_auto_ewars \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

The socket gives the container full control of docker on this machine, so the
flag is never the default, and varde writes a warning when it is given. The
model's run directory is mounted at the same path on the host and in the
container, so the paths that chap gives docker are correct.

varde reads the `MLproject` before the run: from the directory, or from
GitHub. If the model uses `docker_env` and `--docker` is not given, varde
stops before the run and tells you to add the flag. If varde cannot read the
file (for example, with `--offline`), it does not check it.

## The caches

The model runs and the uv, Python and R caches are in `chap/` under the data
directory of varde: `$VARDE_DATA_DIR`, else `$XDG_DATA_HOME/varde`, else
`~/.local/share/varde`. The second run of a model thus installs nothing again. There is one directory for
each model in `chap/runs`, and the R packages can use some hundred megabytes.
To get the space back, delete the directory:

```sh
rm -rf ~/.local/share/varde/chap
```

## Limits

- varde runs the amd64 images. On Apple silicon, Docker runs them under
  emulation, which is slower.
- `chap ui` is not part of chap-core yet, so `varde chap ui` does not work.
- On Windows, use WSL with the Linux binary, as for the rest of varde.
