---
marp: true
theme: chaps
paginate: true
footer: chaps workshop - forecasting disease with climate data
title: Workshop - evaluating forecasting models
description: A hands-on session for students, from install to a comparison of two models
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# Evaluating forecasting models

## A hands-on workshop with Chap and chaps

From an empty laptop to a comparison of two models, in about one hour

---

## What we do today

1. Install chaps, and check the machine.
2. Get a dataset: dengue cases and climate, for districts of Laos.
3. Evaluate a model with a **backtest**.
4. Plot the forecasts against the real cases.
5. Evaluate a second model, and compare the two.

You need a laptop with **Linux or macOS** (or Windows with WSL), **Docker**,
and about **25 GB** of free disk space.

---

## Before the workshop: the downloads

> **The images are large, and a room of laptops on one network downloads
> them slowly.** Download them at home, the day before.

| Image | Size on disk | Used for |
| --- | --- | --- |
| `chap-worker` | about 12 GB | the model from GitHub (step 3) |
| `chap-core` | about 2.4 GB | plots, metrics, the model service (steps 4 to 6) |
| `auto_arima_chapkit` | about 6.2 GB | the model service (step 5) |

```sh
docker pull --platform linux/amd64 ghcr.io/dhis2-chap/chap-worker:v2.3.1
docker pull --platform linux/amd64 ghcr.io/dhis2-chap/chap-core:v2.3.1
chaps run auto_arima_chapkit && chaps stop auto_arima_chapkit
```

---

## Words we use

| Word | What it means |
| --- | --- |
| **Chap** | The Climate Health Analytics Platform: it trains models and makes forecasts |
| **model** | A program that learns from past cases and climate, and forecasts cases |
| **backtest** | Forecasts made from points in the past, compared with what happened |
| **split** | One point in the past from which the model trains and forecasts |
| **period** | One time step of the data, here one month |
| **chaps** | The tool that runs Chap and its models for us, with Docker |

---

## Step 1: Install and check

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps --version
chaps doctor
```

**It worked when** `chaps doctor` starts the `docker cli`, `docker daemon` and
`docker compose` lines with `ok`.

On an Apple silicon Mac, a `warn` on `os and arch` is correct: the images run
under emulation, which is slower.

---

## Step 2: The data

```sh
mkdir -p ~/chap-workshop && cd ~/chap-workshop
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.csv
curl -fsSO https://raw.githubusercontent.com/dhis2-chap/chap-core/master/example_data/laos_subset.geojson
head -3 laos_subset.csv
```

```text
time_period,rainfall,mean_temperature,disease_cases,population,location
2010-01,37.965,20.04,1.0,75049.5636160714,Bokeo
2010-02,8.527,22.22,1.0,75049.5636160714,Bokeo
```

The GeoJSON has the shape of each district. Its name is the same as the CSV's.

---

## Exercise 1: Look at the data

```sh
chaps chap validate laos_subset.csv
```

1. Which districts are in the file? (Look at the `location` column.)
2. Which months does it cover? (Look at the first and the last `time_period`.)
3. Which columns can a model use to forecast `disease_cases`?

**It worked when** chap says `Validation passed: no issues found.`

---

## Step 3: A backtest

```sh
chaps chap eval \
  --model-name https://github.com/dhis2-chap/minimalist_example_r \
  --dataset-csv laos_subset.csv \
  --output-file minimalist_r.nc \
  --backtest-params.n-splits 3 \
  --backtest-params.n-periods 3
```

- The model trains on the data before a split, and forecasts 3 months.
- It does that from 3 splits, so we get 3 forecasts to compare with the truth.
- If you did not download the images before: the first run downloads about
  12 GB. Start it early.

**It worked when** chaps writes `chap finished; it wrote minimalist_r.nc`.

---

## What happened

```text
running `chap eval` in ghcr.io/dhis2-chap/chap-worker:v2.3.1
files: chap reads and writes in /home/me/chap-workshop
...
chap finished; it wrote minimalist_r.nc
`chaps chap plot-backtest minimalist_r.nc --output-file minimalist_r.html` plots it
```

- chap ran in a container, with R and the model's packages in it.
- The model came from GitHub. Its packages are now in a cache, so the next
  run is faster.
- `minimalist_r.nc` holds the forecasts and the real cases for each split.

---

## Step 4: Plot it

```sh
chaps chap plot-backtest minimalist_r.nc --output-file minimalist_r.html
open minimalist_r.html          # macOS
xdg-open minimalist_r.html      # Linux
```

**Exercise 2:**

1. For which district is the forecast closest to the real cases?
2. Is the true value inside the shaded interval most of the time?
3. Does the forecast get worse further ahead?

---

## Step 5: A second model, from the marketplace

```sh
chaps chap eval \
  --model-name auto_arima_chapkit \
  --dataset-csv laos_subset.csv \
  --output-file auto_arima.nc \
  --backtest-params.n-splits 3 \
  --backtest-params.n-periods 3
```

- `auto_arima_chapkit` is a model **service**: a container with an API.
- chaps starts it for us, and tells chap where it answers.
- `chaps models list` lists the other models.
- The model's image is about 6 GB, if you did not download it before.

**It worked when** chaps writes `chap finished; it wrote auto_arima.nc`.

---

## Step 6: Compare the two

```sh
chaps chap export-metrics minimalist_r.nc auto_arima.nc --output-file comparison.csv
```

| Column | What it measures | Better when |
| --- | --- | --- |
| `mae` | the mean absolute error | lower |
| `rmse` | the root mean square error, which punishes large errors more | lower |
| `crps` | how far the whole forecast distribution is from the truth | lower |
| `coverage_10_90` | how often the truth is inside the 10-90% interval | near 0.8 |

**Exercise 3:** Which model is better on `crps`? Is it also better on
`coverage_10_90`? Why can the two answers differ?

---

## Exercise 4: Change the backtest

Run the second model again with different settings, and compare:

```sh
chaps chap eval --model-name auto_arima_chapkit \
  --dataset-csv laos_subset.csv --output-file arima_long.nc \
  --backtest-params.n-splits 3 --backtest-params.n-periods 6
chaps chap export-metrics auto_arima.nc arima_long.nc --output-file horizon.csv
```

1. What happens to `mae` when the model forecasts 6 months, not 3?
2. Why is a longer forecast more difficult?

---

## Clean up

```sh
chaps stop auto_arima_chapkit --purge   # the model service
rm -rf ~/.local/share/chaps/chap        # the caches
docker image ls 'ghcr.io/dhis2-chap/*'  # the images, to remove with docker image rm
```

Your `.nc`, `.html` and `.csv` files stay in `~/chap-workshop`.

---

## Where to go next

- **Your own data:** the same columns, a GeoJSON of the same name, then
  `chaps chap eval --dataset-csv mydata.csv ...`.
- **Your own model:** see the model template, and `chaps run` for a model
  service of your own.
- **DHIS2 and the Modeling App:** `chaps init mychap --with dhis2`, then
  `chaps dhis2 connect`.

The book: **https://winterop-com.github.io/chaps/**
