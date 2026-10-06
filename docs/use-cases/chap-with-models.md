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
registered, and `varde models test --all` says every model passes.

You get chap-core's API on port 8700 and one service per model. The models
register with chap-core over the compose network and publish no host port;
you reach them through chap-core at
`http://localhost:8700/v2/services/<service_id>/run/`. Pick models later with
`varde ui`, or `varde models enable ID` / `disable ID`.

## Every model in the marketplace

`default` is the one model varde starts with; `all` is every model the
marketplace lists (templates aside), and a list of ids picks some.
`varde models list` prints them:

```sh
varde init mychap --models all
cd mychap
varde up
varde models test --all --backtest   # each one through chap-core, with scores
```

Some model images are large, so the first `varde up` pulls for a while. Every
model image is linux/amd64 only, so on an Apple Silicon Mac they run under
emulation and the backtests take minutes each.

Next: [Models and the marketplace](../models.md), [Authentication](../auth.md)
before you put it on a network, [Backup and restore](../backup.md).

All shapes: [Use cases](../use-cases.md).
