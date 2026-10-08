# Chap with forecasting models

The default, and what most deployments are: chap-core plus the models it runs.

```sh
varde init mychap --models default
cd mychap
varde up
varde status                 # chap-core up, every model registered
varde models test --all      # make each model train and predict once
```

It worked when `varde status` shows chap-core `up` and every model
registered:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                STATE                       REACH          LAST PING
chapkit-ewars-model  registered, not configured  via chap-core  1s ago

1 model registered
chapkit-ewars-model: chap-core has no configured model for it, so nothing can run it; run `varde models configs sync`
```

and `varde models test --all` says every model passes:

```text
testing 1 model (model level; add --backtest to go through chap-core)
chapkit-ewars-model    pass   16s   1 training, 1 prediction

1 of 1 model passes
```

`registered, not configured` is the normal state after a plain `varde up`.
chap-core v2.4.0 makes no configured model when a model registers, and the
model level of `varde models test` does not need one. A backtest and the
Modeling App need one. The [backtest](#test-through-chap-core) below creates
them, and so do `varde models configs sync` and `varde up --wait`. After that,
`varde status` shows the model as `registered`. See
[Configured models](../models.md#configured-models).

You get chap-core's API on port 8700 and one service per model. The models
register with chap-core over the compose network and publish no host port.
You reach them through chap-core at
`http://localhost:8700/v2/services/<service_id>/run/`.

To change the models later, use `varde ui`, or `varde models enable ID` and
`varde models disable ID`. `disable` stops the model at once. After `enable`,
run `varde up` to start the model.

## Every model in the marketplace

`default` is the one model varde starts with, `chapkit_ewars_model`. `all` is
every model the marketplace lists (templates aside), and a list of ids picks
some. `varde models list` prints them.

This is a new deployment. It needs port 8700 too. If the deployment from the
first section is still up, go to `mychap` and run `varde down --volumes --yes`
first, then `cd ..`. `varde init` then warns that `mychap` also uses port
8700. While `mychap` is down, you can ignore that warning.

```sh
varde init allmodels --models all
cd allmodels
varde up
varde status                 # again, until every model is registered
varde models test --all      # make each model train and predict once
```

The model images are large: five images of 1 to 7 GB each, all linux/amd64
only. `varde init` already pulls one of them to read its user, and says
``asking ghcr.io/chap-models/chapkit_ghr_model:sha-dfb2e3f what uid `app` is (this pulls the image)``.
The first `varde up` pulls the others, which can take many minutes.

`varde up` returns before the models register. Right after it, `varde status`
can show some or all models as not registered yet, and exits with 1:

```text
MODEL                             STATE                    REACH          LAST PING
auto-arima-chapkit                running, not registered  via chap-core  -
...
chapkit-simple-multistep-model    running, not registered  via chap-core  -

5 of 5 models are not registered.
auto-arima-chapkit: started under two minutes ago and registers once it is ready; run `varde status` again in a minute
...
```

If you see this, wait a minute and run `varde status` again. On an M2 Max,
all five models registered in less than a minute. It worked when `varde status`
ends with `all 5 models registered`, followed by one
`chap-core has no configured model for it` line for each model, as in the
first section. `varde models test --all` then says:

```text
testing 5 models (model level; add --backtest to go through chap-core)
auto-arima-chapkit                  pass   12s   1 training, 1 prediction
chapkit-ewars-model                 pass   16s   1 training, 1 prediction
chapkit-ghr-model                   pass   22s   1 training, 1 prediction
chapkit-rwanda-malaria-bym-model    pass   20s   1 training, 1 prediction
chapkit-simple-multistep-model      pass    5s   1 training, 1 prediction

5 of 5 models pass
```

On an Apple Silicon Mac the models run under emulation; the five tests above
took about 80 seconds on an M2 Max.

## Test through chap-core

`varde models test --all --backtest` tests each model through chap-core, the
way the Modeling App does. It makes a dataset from the sample data of the
model, runs a backtest on it and reads the scores. A backtest needs a
configured model, so the command first creates the configured models from the
marketplace entry of each model, as `chap-admin install` does. You make no
API call yourself.

```sh
varde models test --all --backtest
```

It worked when every model passes with its scores, and the closing lines name
the configured models it created:

```text
testing 5 models (through chap-core: a dataset, a backtest and its scores)
auto-arima-chapkit                  pass      18s   crps 3.5  mae 4.8  rmse 6.0
chapkit-ewars-model                 pass      33s   crps 3.5  mae 4.5  rmse 5.3
chapkit-ghr-model                   pass   1m 28s   crps 12.9  mae 19.6  rmse 21.4
chapkit-rwanda-malaria-bym-model    pass      38s   crps 18.8  mae 28.3  rmse 32.9
chapkit-simple-multistep-model      pass      13s   crps 9.6  mae 12.5  rmse 14.9

created configured models in chap-core for auto_arima_chapkit (monthly), chapkit_ewars_model (monthly_climate, monthly_population_only, monthly_region_seasonal), chapkit_ghr_model (monthly_climate), chapkit_rwanda_malaria_bym_model (monthly), chapkit_simple_multistep_model (monthly_climate, monthly_selfhistory)
5 of 5 models pass
```

The five backtests took about three minutes on an M2 Max. The scores are not
the same from one run to the next. A second run creates no configured models,
so it has no `created configured models` line. After the first run,
`varde status` shows each model as `registered`.

Next: [Models and the marketplace](../models.md), [Authentication](../auth.md)
before you put it on a network, [Backup and restore](../backup.md).

All shapes: [Use cases](../use-cases.md).
