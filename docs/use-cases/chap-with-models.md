# Chap with forecasting models

The default, and what most deployments are: chap-core plus the models it runs.

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps status                 # chap-core up, every model registered
chaps models test --all      # make each model train and predict once
```

It worked when `chaps status` shows chap-core `up` and ends with every model
registered, and `chaps models test --all` says every model passes.

You get chap-core's API on port 8700 and one service per model. The models
register with chap-core over the compose network and publish no host port;
you reach them through chap-core at
`http://localhost:8700/v2/services/<service_id>/run/`. Pick models later with
`chaps ui`, or `chaps models enable ID` / `disable ID`.

## Every model in the marketplace

`default` is the one model chaps starts with; `all` is every model the
marketplace lists (templates aside), and a list of ids picks some.
`chaps models list` prints them:

```sh
chaps init mychap --models all
cd mychap
chaps up
chaps models test --all --backtest   # each one through chap-core, with scores
```

Some model images are large, so the first `chaps up` pulls for a while. Every
model image is linux/amd64 only, so on an Apple Silicon Mac they run under
emulation and the backtests take minutes each.

Next: [Models and the marketplace](../models.md), [Authentication](../auth.md)
before you put it on a network, [Backup and restore](../backup.md).

All shapes: [Use cases](../use-cases.md).
