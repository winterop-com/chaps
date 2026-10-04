# Evaluating a model on your own data

You have a dataset as a CSV, with a GeoJSON beside it, and you want to know
how well a model forecasts it. chap-core's own CLI does this, and
`chaps chap` runs that CLI in a container. You do not install Python, uv or R.

## Steps

1. Go to the directory that has your data:

   ```sh
   cd ~/work
   ls
   # laos_subset.csv  laos_subset.geojson
   ```

2. Evaluate a model from GitHub:

   ```sh
   chaps chap eval --model-name https://github.com/dhis2-chap/minimalist_example_r \
     --dataset-csv laos_subset.csv --output-file r.nc \
     --backtest-params.n-splits 3 --backtest-params.n-periods 3
   ```

   The first run pulls the worker image (about 12 GB) and installs the model.
   The second run of the same model uses the caches.

3. Plot the evaluation:

   ```sh
   chaps chap plot-backtest r.nc --output-file r.html
   ```

4. Open `r.html` in a browser.

It worked when the closing line says `chap finished; it wrote r.nc`, and then
`chap finished; it wrote r.html`.

## A model of a deployment

If a deployment runs the model already, use its service. Run the command in
the deployment's directory, or give the directory with `-C`:

```sh
chaps -C ~/mychap chap eval --model-name http://chapkit-ewars-model:8000 \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

The line before the run lists the URLs of the deployment's models.

## A model that runs in docker

If the model's `MLproject` has `docker_env`, add `--docker`:

```sh
chaps chap --docker eval --model-name https://github.com/dhis2-chap/chap_auto_ewars \
  --dataset-csv laos_subset.csv --output-file ewars.nc
```

See [The chap CLI](../chap-cli.md) for the mounts, the image choice and the
caches.
