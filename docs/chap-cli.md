# The chap CLI

chap-core has its own command line, `chap`. It evaluates a model on a dataset
of your own, plots the result and exports the scores. It is usually installed
with uv (`uvx --from chap-core chap ...`). That needs Python and uv, and for
many models also R.

`chaps chap` runs the same `chap` in a container. You need Docker and
nothing more:

```sh
chaps chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv --output-file r.nc
chaps chap plot-backtest r.nc --output-file r.html
chaps chap export-metrics r.nc --output-file r.csv
```

For a guide from an empty machine to a comparison of two models, see
[Evaluating a model on your own data](./use-cases/evaluate-with-chap-cli.md).

Each argument after `chap` goes to chap unchanged. To read chap's own help,
run `chaps chap` with no arguments, or `chaps chap eval --help` for one
command. `chaps chap --help` shows only the options of chaps itself. These
come between `chap` and chap's own command:

| Option | What it does |
| --- | --- |
| `--tag TAG` | The chap-core version to run. See [Which version](#which-version). |
| `--image core` or `--image worker` | The image to run in. See [Which image](#which-image). |
| `--group NAME` | The `chaps run` group whose models chap can reach. See [Chapkit models](#chapkit-models-no-chaps-up). |
| `--stop` | Stop the model that this run started, after the run. See [Chapkit models](#chapkit-models-no-chaps-up). |
| `--timeout SECONDS` | How long chaps waits for a model that it starts (300). |
| `--docker` | Give the container the docker socket. See [Models that run in docker](#models-that-run-in-docker). |

For example: `chaps chap --tag master --docker eval --model-name ...`.

## Your files

chaps mounts the current directory in the container at the same path. Thus:

- a relative path such as `laos_subset.csv` or `out/r.nc` means the same file
  in the container and on your machine;
- every file chap writes in the current directory is on your machine after
  the run, and belongs to your user;
- a dataset CSV finds its GeoJSON beside it, as it does without chaps.

A path outside the current directory also works: an absolute path, a `../`
path, or the same after `=` (`--output-file=/data/out/r.nc`). chaps mounts
the nearest existing directory above the path, at the same path. chap does not
make a directory that does not exist, so make the output directory first.

Before the run, chaps writes on stderr which directories chap can see. After
the run, it names the files that are new or changed, and the next command:

```text
running `chap eval` in ghcr.io/dhis2-chap/chap-worker:v2.3.1
files: chap reads and writes in /home/me/work
...
chap finished; it wrote r.nc
`chaps chap plot-backtest r.nc --output-file r.html` plots it
```

In a deployment, run `chaps chap` in a subdirectory, for example `eval/`, so
that chap's files do not mix with the compose files. chaps finds the
deployment from a subdirectory too. When you run it in the deployment
directory itself, chaps writes a line that says so.

chap's own output is not changed. chaps writes its lines on stderr, and
`--json` does not apply to this command. A run that only asks for help
(`chaps chap`, `--help`, `--version`) gets only the first line.

## Which image

chap-core publishes two images with the same tags. chaps selects one from
`--model-name`:

| The model | The image | Size |
| --- | --- | --- |
| A chapkit service (an `http://` URL that is not GitHub) | `chap-core` | about 2.4 GB |
| No model (`plot-backtest`, `export-metrics`, ...) | `chap-core` | about 2.4 GB |
| A GitHub repository, or a model directory | `chap-worker` | about 12 GB |

The worker image has uv, R and INLA, and chap-core's own worker runs models in
it. A chapkit service is plain HTTP, so the small image is sufficient for it.
In a deployment, chaps always uses the worker image, because the deployment
has it already. A model id, such as `chapkit_ewars_model`, is a chapkit
service too. `--image core` or `--image worker` overrides the choice. A
model that needs R fails in the small image with `Rscript: not found`.

The first run pulls the image. The line before the run says so, with the
size.

## Which version

- In a deployment (in its directory, or with `-C`), chaps uses the
  deployment's chap-core tag. The CLI and the server are then the same
  version.
- Outside a deployment, chaps uses the newest chap-core release.
- With `--offline`, chaps uses the newest image of that kind on this machine.
- `--tag TAG` overrides all of these, for example `--tag master`.

## Chapkit models: no `chaps up`

A chapkit model is a model service: chap sends it data over HTTP, and the
service trains and forecasts. `chap eval` needs only that service. It does not
need chap-core, its database or its worker. So you do not run `chaps up` for
an evaluation. Give the model's id, and chaps starts the model:

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

What chaps does with the id:

| Where you are | The id | What chaps does |
| --- | --- | --- |
| In a deployment that has the model | `chapkit_ewars_model` | Starts only that model's container, with no chap-core, if it does not run. |
| In a deployment that does not have it | `auto_arima_chapkit` | Stops, and names `chaps models enable auto_arima_chapkit`. chaps does not change the deployment's models for an evaluation. |
| Outside a deployment | any marketplace id | Starts the model in a `chaps run` group, as `chaps run ID` does, and joins that group's network. `--group NAME` selects the group. |

Before the run, chaps checks that the model answers. Then it gives chap the
model's URL on the network, for example `http://chapkit-ewars-model:8000`,
and writes:

```text
starting chapkit-ewars-model for this run, without chap-core
model: chapkit_ewars_model at http://chapkit-ewars-model:8000 answers
```

A model that chaps started keeps running, so the next evaluation starts at
once. The last line says how to stop it. `--stop` stops it after the run
instead. A model that ran before the command is never stopped. `--timeout
SECONDS` (300) is how long chaps waits for a model that it starts.

A model that chaps starts in a deployment does not register with chap-core,
because chap-core does not run. If you run `chaps up` later, compose recreates
the model's container, and the model then registers as usual.

A word is a model id only when no file or directory of that name is in the
current directory. A directory of the same name is used as a model directory,
as chap does.

### A model URL

You can also give a URL, for example a model on another server:
`--model-name https://models.example.org`. chaps asks the URL for
`/api/v1/info` before the run. If it does not answer, chaps stops with a note,
and chap does not start:

```text
error: the model server at http://nothing:8000 is not running (no answer on /api/v1/info), so chap was not started; ...
```

The URL of a model of the deployment or group, such as
`http://chapkit-ewars-model:8000`, works like its id: chaps starts the model
if it does not run.

Do not use the `http://localhost:PORT` URL that `chaps ps` or `chaps status`
shows. In the container, `localhost` is the container itself, so that URL
reaches nothing. chaps refuses such a URL before the run, and names the
service URL of the same model when it knows it.

An evaluation of a chapkit service stores a model configuration and its
artifacts in that service's own database, as it does without chaps.

## Models that run in docker

A model whose `MLproject` has `docker_env` starts a container of its own. For
that, chap needs the docker socket. `--docker` gives it:

```sh
chaps chap --docker eval --model-name https://github.com/dhis2-chap/chap_auto_ewars \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

The socket gives the container full control of docker on this machine, so the
flag is never the default, and chaps writes a warning when it is given. The
model's run directory is mounted at the same path on the host and in the
container, so the paths that chap gives docker are correct.

chaps reads the `MLproject` before the run: from the directory, or from
GitHub. If the model uses `docker_env` and `--docker` is not given, chaps
stops before the run and tells you to add the flag. If chaps cannot read the
file (for example, with `--offline`), it does not check it.

## The caches

The model runs and the uv, Python and R caches are in `chap/` under chaps'
data directory (`~/.local/share/chaps/chap`, or `$CHAPS_DATA_DIR/chap`). The
second run of a model thus installs nothing again. There is one directory for
each model in `chap/runs`, and the R packages can use some hundred megabytes.
To get the space back, delete the directory:

```sh
rm -rf ~/.local/share/chaps/chap
```

## Limits

- chaps runs the amd64 images. On Apple silicon, Docker runs them under
  emulation, which is slower.
- `chap ui` is not part of chap-core yet, so `chaps chap ui` does not work.
- On Windows, use WSL with the Linux binary, as for the rest of chaps.
