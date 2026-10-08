# A new model of your own

For starting a new forecasting model: `varde models new` makes the project,
Docker builds its image, and a varde deployment trains it, predicts with it
and scores it through chap-core. Python, uv, R and the chapkit CLI are not
necessary on the machine.

## Make the project

```sh
varde models new dengue-lags
```

```text
created dengue-lags in dengue-lags (fn-py template, service id dengue-lags)
next: build the image as `dengue-lags/README.md` says, then run `varde run dengue-lags:dev`
```

The default type, `fn-py`, is a chapkit service with the model in Python, in
`main.py`. For a model in R scripts, add `--template shell-r`. For a model in
the MLproject format, use `--template mlproject-py` or `--template
mlproject-r`. [The types](../models.md#the-types) lists all nine.

The example model predicts, for each location, the mean of `disease_cases` in
the last `window` periods. Replace it with your model. `README.md` in the
project lists the files to change.

## Build the image

In the project:

```sh
cd dengue-lags
git init && git add . && git commit -m "feat: first version"
docker build --platform linux/amd64 --build-arg GIT_REVISION=$(git rev-parse HEAD) -t dengue-lags:dev .
cd ..
```

chap-core stores a model only if the image has a git revision, so build from
a commit with `GIT_REVISION`. The first build pulls the base image,
`ghcr.io/dhis2-chap/chapkit-py`. A rebuild takes some seconds.

## Run it with chap-core

```sh
varde init lab --models none
cd lab
varde models add dengue-lags:dev
varde up --wait
varde status
```

`varde up --wait` returns when the model has registered, and it creates the
configured model `default`. `varde status` then shows:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL        STATE       REACH          LAST PING
dengue-lags  registered  via chap-core  3s ago

1 model registered
```

## Test it through chap-core

```sh
varde models test dengue_lags --backtest --seed 7
```

```text
testing 1 model (through chap-core: a dataset, a backtest and its scores)
dengue-lags    pass      16s   crps 35.5  mae 35.5  rmse 38.8
  configured model: default (id 14)

1 of 1 model passes
```

The test makes a dataset from the model's own sample data, runs a backtest in
chap-core, and prints its scores. `--seed` makes the data the same on every
run, so two runs can be compared.

## Give an option another value

`window` is an option of the model, with 3 as the default. A configured model
gives it another value:

```sh
varde models configs add dengue_lags --name long --set window=6
varde models test dengue_lags --backtest --seed 7 --config long
```

```text
created configured model long of dengue_lags
testing 1 model (through chap-core: a dataset, a backtest and its scores)
dengue-lags    pass      13s   crps 41.4  mae 41.4  rmse 43.2
  configured model: long (id 15)

1 of 1 model passes
```

The Modeling App shows `long` as a model to pick, next to `default`. See
[Configured models](../models.md#configured-models).

## After a change

chap-core stores each version of a model only once. If you build a new commit
under the same version, the model registers, but its backtests fail:

```text
dengue-lags    FAIL       8s   ValueError: Model template 'dengue-lags' version '0.1.0' is stored from revision 'db139b6b09185e0be0971d31cafb39f126c910...
```

So, before you build again, raise `version` in `main.py` (in `MLproject` for
an MLproject type), for example to `0.1.1`. Then commit, and build with the
same `docker build` line. In `lab`:

```sh
varde restart
varde models test dengue_lags --backtest --seed 7
```

```text
dengue-lags    pass      13s   crps 39.5  mae 39.5  rmse 42.3
  configured model: default (id 16)

created configured models in chap-core for dengue_lags (default)
1 of 1 model passes
```

In this run, the change was `window: int = 4` in `main.py`, so the score is
different. The backtest created the configured model `default` of version 0.1.1. A configured
model belongs to one version, so `long` is not there for 0.1.1: add it again
with `varde models configs add`.

To run the model service alone, with no deployment: `varde run
dengue-lags:dev`. To publish the image, push the project to GitHub: the
workflow in `.github/workflows/publish.yml` builds it and publishes it to GHCR.
Then `varde models add` takes the repository URL, as in
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

These lines come from varde 0.101.0 with this change, chap-core 2.4.0 and
chapkit 2.3.1 on macOS (arm64, the image under amd64 emulation), on
2026-10-08.

More: [A new model project](../models.md#a-new-model-project).

All shapes: [Use cases](../use-cases.md).
