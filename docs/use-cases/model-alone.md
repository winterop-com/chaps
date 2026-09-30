# A chapkit model service on its own

One forecasting model, running and answering on its own port, with no
chap-core in front of it. For developing or checking a model: calling its API
directly, reading its `/docs`, running `chapkit test` against it, or putting it
behind something other than chap-core.

```sh
chaps init ewars --only none --models chapkit_ewars_model
cd ewars
chaps up
chaps status                 # the model's own /health: up
curl http://localhost:5001/health
open http://localhost:5001/docs
```

Your own model works the same way, from its repository or a published image:

```sh
chaps init mymodel --only none
cd mymodel
chaps models add https://github.com/my-org/chapkit_dengue_model   # adds and enables it
chaps up
```

Without chap-core each model gets a host port when it is enabled (5001 up;
`--port N` picks one), does not try to register anywhere, and is judged by
`chaps status` and `chaps doctor` through its own `/health`. Expect one
`registration.missing_orchestrator_url` line at the top of its log; that is
servicekit noting there is nothing to register with, and it carries on.
`chaps models test` works here: it runs `chapkit test` inside the model's own
container. `chaps models test --backtest` and `chaps jobs` go through
chap-core, so they refuse and name `chaps components enable chap-core`.

Next: [Model services without chap-core](../components.md#model-services-without-chap-core),
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
