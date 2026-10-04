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

Each argument after `chap` goes to chap unchanged. To read chap's own help,
run `chaps chap` with no arguments, or `chaps chap eval --help` for one
command. `chaps chap --help` shows only the chaps options: `--tag`, `--image`,
`--group` and `--docker`.

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

chap's own output is not changed. chaps writes its lines on stderr, and
`--json` does not apply to this command.

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
has it already. `--image core` or `--image worker` overrides the choice. A
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

## The models of a deployment

In a deployment that runs, the container joins the deployment's network. Its
models answer at `http://<service>:8000`, and the line before the run lists
them:

```text
models: chapkit_ewars_model at http://chapkit-ewars-model:8000
```

Give that URL to `--model-name`. If the deployment does not run, chaps says so
and names `chaps up`. Outside a deployment, the container joins the network of
the default `chaps run` group, if it exists. Thus a model that
`chaps run auto_arima_chapkit` started answers at its service name too.
`--group NAME` selects another group.

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
