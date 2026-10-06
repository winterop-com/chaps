# A chapkit model service on its own

One forecasting model, running and answering on its own port, with no
chap-core in front of it. For developing or checking a model: calling its API
directly, reading its `/docs`, running `chapkit test` against it, or putting it
behind something other than chap-core.

The quickest way needs no folder at all:

```sh
varde run chapkit_ewars_model                               # a marketplace model
varde run https://github.com/chap-models/chapkit_ghr_model  # by repository
varde run ghcr.io/my-org/chapkit_dengue_model:sha-1eb8cf1   # by image
varde ps
varde stop chapkit_ewars_model
```

`varde run` keeps the model in a group under varde' data directory, publishes
it on `127.0.0.1` only, and returns once it answers. `-a` keeps it in the
foreground with its log, until Ctrl-C stops it. See
[Running one model](../run.md). For a deployment directory of your own, with
`varde status`, `varde doctor` and everything else:

```sh
varde init ewars --only none --models chapkit_ewars_model
cd ewars
varde up
varde status                 # the model's own /health: up
curl http://localhost:5001/health
open http://localhost:5001/docs
```

Your own model works the same way, from its repository or a published image:

```sh
varde init mymodel --only none
cd mymodel
varde models add https://github.com/my-org/chapkit_dengue_model   # adds and enables it
varde up
```

Without chap-core each model gets a host port when it is enabled (5001 up;
`--port N` picks one), does not try to register anywhere, and is judged by
`varde status` and `varde doctor` through its own `/health`. Expect one
`registration.missing_orchestrator_url` line at the top of its log; that is
servicekit noting there is nothing to register with, and it carries on.
`varde models test` works here: it runs `chapkit test` inside the model's own
container. `varde models test --backtest` and `varde jobs` go through
chap-core, so they refuse and name `varde components enable chap-core`.

Next: [Model services without chap-core](../components.md#model-services-without-chap-core),
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
