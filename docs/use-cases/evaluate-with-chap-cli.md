# Evaluating a model on your own data

This guide goes from an empty machine to a comparison of two forecasting
models on one dataset. It uses chap-core's own command line, `chap`, through
`chaps chap`. You do not install Python, uv or R: `chaps chap` runs `chap` in
a container.

At the end you have:

- two evaluations (`.nc` files), one for each model;
- a plot of an evaluation (`.html`) that you open in a browser;
- a table that compares the two models (`.csv`).

All of these files are in your working directory, and they belong to your
user.

## What you need

- Linux or macOS. On Windows, use WSL with the Linux binary.
- Docker, and it must run. On macOS, start Docker Desktop.
- About 15 GB of free disk space. The first run downloads the chap-core
  images: about 12 GB for the image that runs R models, and about 2.4 GB for
  the smaller one.
- A connection to the internet for the first run.

The first run of each step takes some minutes, because Docker downloads the
images. On an Apple silicon Mac, Docker runs the images under emulation, so
each step is slower than on Linux. After the first run, an evaluation of the
example data takes less than a minute.

## Step 1: Install chaps

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps --version
```

It worked when `chaps --version` prints a version, for example `chaps 0.99.6`.
See [Install](../install.md) for other ways to install.

## Step 2: Check the machine

```sh
chaps doctor
```

It worked when the `docker cli`, `docker daemon` and `docker compose` lines
start with `ok`. A `warn` on the `os and arch` line of an Apple silicon Mac is
correct: it only tells you that the images run under emulation. If a line
starts with `fail`, do the fix that it names, then run `chaps doctor` again.

## Step 3: Make a working directory and get data

Make a directory for this work, and go into it:

```sh
mkdir -p ~/chap-eval
cd ~/chap-eval
```

Download chap-core's example data. It is monthly dengue cases, with rainfall,
temperature and population, for districts of Laos:

```sh
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.csv
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.geojson
```

It worked when `ls` shows `laos_subset.csv` and `laos_subset.geojson`.

**To use your own data instead**, put your CSV and its GeoJSON in this
directory. They must have the same name, for example `mydata.csv` and
`mydata.geojson`. Use the same columns as the example file:

```text
time_period,rainfall,mean_temperature,disease_cases,population,location
2010-01,37.965,20.04,1.0,75049.56,Bokeo
```

Each `location` must be a feature in the GeoJSON. In the steps below, change
`laos_subset.csv` to the name of your CSV.

## Step 4: Evaluate a model from GitHub

This model is a small R model from chap-core's examples:

```sh
chaps chap eval \
  --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv \
  --output-file minimalist_r.nc \
  --backtest-params.n-splits 3 \
  --backtest-params.n-periods 3
```

What the options do:

| Option | What it does |
| --- | --- |
| `--model-name` | The model: a GitHub repository, a model directory, or the URL of a model service. |
| `--dataset-csv` | Your data. The GeoJSON with the same name is found automatically. |
| `--output-file` | The evaluation that chap writes, a NetCDF (`.nc`) file. |
| `--backtest-params.n-splits` | How many times the model trains and forecasts, each time from a later point in the data. |
| `--backtest-params.n-periods` | How many periods (here months) each forecast goes ahead. |

chaps writes some lines of its own before chap starts:

```text
running `chap eval` in ghcr.io/dhis2-chap/chap-worker:v2.3.1; the first run pulls it, about 12 GB
files: chap reads and writes in /home/me/chap-eval
```

Then chap writes its own log lines. At the end, chaps writes:

```text
chap finished; it wrote minimalist_r.nc
`chaps chap plot-backtest minimalist_r.nc --output-file minimalist_r.html` plots it
```

It worked when you see `chap finished; it wrote minimalist_r.nc`.

chaps selected the large `chap-worker` image because a model from GitHub can
need R. The second time you evaluate the same model, chaps uses the copy and
the packages it keeps in its cache, so the run is faster.

## Step 5: Plot the evaluation

Run the command that the last line of step 4 gave you:

```sh
chaps chap plot-backtest minimalist_r.nc --output-file minimalist_r.html
```

It worked when chaps writes `chap finished; it wrote minimalist_r.html`. Open
the file in a browser:

```sh
open minimalist_r.html        # macOS
xdg-open minimalist_r.html    # Linux
```

The plot shows the real cases and the forecasts of each split, one panel for
each location.

## Step 6: Evaluate a second model, from the marketplace

A model from the [marketplace](../models.md) is a model service: a container
with an HTTP API. Start one with `chaps run`:

```sh
chaps run auto_arima_chapkit
```

It worked when chaps writes `running auto_arima_chapkit on
http://localhost:5001` (the port can be different).

**Do not give that `localhost` URL to `chaps chap`.** In the container that
runs chap, `localhost` is that container. chaps puts chap's container on the
same network as the model, where the model has its service name. The
`models:` line that `chaps chap` writes before each run shows that URL:

```text
models: auto_arima_chapkit at http://auto-arima-chapkit:8000
```

Evaluate the model with that URL:

```sh
chaps chap eval \
  --model-name http://auto-arima-chapkit:8000 \
  --dataset-csv laos_subset.csv \
  --output-file auto_arima.nc \
  --backtest-params.n-splits 3 \
  --backtest-params.n-periods 3
```

It worked when chaps writes `chap finished; it wrote auto_arima.nc`. This run
uses the smaller `chap-core` image, because chap only talks HTTP to a model
service.

If you give the `localhost` URL by mistake, chaps stops before the run, and
its message gives the correct URL.

## Step 7: Compare the two models

```sh
chaps chap export-metrics minimalist_r.nc auto_arima.nc --output-file comparison.csv
```

It worked when chaps writes `chap finished; it wrote comparison.csv`. chap can
write some `RuntimeWarning` lines about `log1p` before that. They do not stop
the run.

`comparison.csv` has one row for each model. Open it in a spreadsheet. Some
of the columns:

| Column | What it measures | Better when |
| --- | --- | --- |
| `crps` | How far the forecast distribution is from the real cases | lower |
| `mae` | The mean absolute error of the forecasts | lower |
| `rmse` | The root mean square error of the forecasts | lower |
| `coverage_10_90` | How often the real value is in the 10-90% forecast interval | near 0.8 |

## Step 8: Clean up

Stop the model service, and remove its data:

```sh
chaps stop auto_arima_chapkit --purge
```

Your `.nc`, `.html` and `.csv` files stay in `~/chap-eval`. To get the disk
space of the caches back, delete chaps' chap directory:

```sh
rm -rf ~/.local/share/chaps/chap
```

To remove the images too, list them, then remove each one with
`docker image rm`:

```sh
docker image ls 'ghcr.io/dhis2-chap/*'
docker image rm ghcr.io/dhis2-chap/chap-worker:v2.3.1
```

## If something goes wrong

| chaps writes | What to do |
| --- | --- |
| `` `http://localhost:...` is this machine `` | Use the URL from the `models:` line, as in step 6. |
| ``runs in docker (`docker_env` in its MLproject)`` | The model starts a container of its own. Run the same command with `chaps chap --docker ...`. See [Models that run in docker](../chap-cli.md#models-that-run-in-docker). |
| `FileNotFoundError: [Errno 2] No such file or directory` | The directory of the output file does not exist. Make it with `mkdir -p`, then run again. |
| `Rscript: not found` | The model needs R. Add `--image worker` after `chap`. |
| `chap exited with status N` | chap stopped with an error. The cause is in chap's own lines above. |
| `docker could not start ...` | Docker could not download or start the image. Run `chaps doctor`. |

More messages are in [Troubleshooting](../troubleshooting.md).

## Next

- To evaluate the models of a [deployment](../concepts.md), run `chaps chap`
  in the deployment's directory, or give it with `-C`:
  `chaps -C ~/mychap chap eval --model-name http://chapkit-ewars-model:8000 ...`.
  The `models:` line lists the deployment's models.
- To use another chap-core version, add `--tag`, for example
  `chaps chap --tag master eval ...`.
- For chap's own options, run `chaps chap eval --help`.
- [The chap CLI](../chap-cli.md) tells how `chaps chap` mounts your files,
  selects the image and keeps its caches.
