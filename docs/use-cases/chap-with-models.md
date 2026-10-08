# Chap with forecasting models

The default, and what most deployments are: chap-core plus the models it runs.

```sh
varde init mychap --models default
cd mychap
varde up
varde status                 # chap-core up, every model registered
varde models test --all      # make each model train and predict once
```

It worked when `varde status` shows chap-core `up` and ends with every model
registered:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  2s ago

1 model registered
```

and `varde models test --all` says every model passes:

```text
testing 1 model (model level; add --backtest to go through chap-core)
chapkit-ewars-model    pass   18s   1 training, 1 prediction

1 of 1 model passes
```

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
chapkit-simple-multistep-model    running, not registered  via chap-core  -

1 of 5 models is not registered.
chapkit-simple-multistep-model: started under two minutes ago and registers once it is ready; run `varde status` again in a minute
```

If you see this, wait a minute and run `varde status` again. It worked when
`varde status` ends with `all 5 models registered`, and `varde models test
--all` says:

```text
testing 5 models (model level; add --backtest to go through chap-core)
auto-arima-chapkit                  pass   12s   1 training, 1 prediction
chapkit-ewars-model                 pass   18s   1 training, 1 prediction
chapkit-ghr-model                   pass   24s   1 training, 1 prediction
chapkit-rwanda-malaria-bym-model    pass   22s   1 training, 1 prediction
chapkit-simple-multistep-model      pass    6s   1 training, 1 prediction

5 of 5 models pass
```

On an Apple Silicon Mac the models run under emulation; the five tests above
took about 90 seconds on an M2 Max.

`varde models test --all --backtest` tests each model through chap-core, with
scores. chap-core v2.4.0 makes no configured model from a registered service,
and a backtest needs one. So the command first creates the configured models
from the marketplace entry of each model, as `chap-admin install` does. Its
closing lines name them:

```text
created configured models in chap-core for auto_arima_chapkit (monthly), ...
```

`varde up --wait` and `varde models configure` do the same step. See
[Configured models](../models.md#configured-models). The model level above
does not need a configured model.

Next: [Models and the marketplace](../models.md), [Authentication](../auth.md)
before you put it on a network, [Backup and restore](../backup.md).

All shapes: [Use cases](../use-cases.md).
