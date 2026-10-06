# Your model from its checkout, with Chap

For developing a chapkit model: varde runs chap-core, and you run the model
from its own checkout on this machine (with `uv run`, a debugger, hot reload),
registered with that chap-core like any deployed model. Nothing about the model
has to be built into an image.

```sh
varde init mychap --models none
cd mychap
varde up
varde status                 # chap-core up, no models yet
```

Then, in the model's checkout, start it on a port of its own and tell it where
chap-core is and where chap-core can call it back:

```sh
export SERVICEKIT_ORCHESTRATOR_URL='http://localhost:8700/v2/services/$register'
export SERVICEKIT_HOST=host.docker.internal
export SERVICEKIT_PORT=8001
uv run uvicorn main:app --host 0.0.0.0 --port 8001
```

`main:app` is the app in a `main.py` at the top of the checkout, which is the
layout chapkit models use; a model whose `main.py` is inside a package runs as
`PACKAGE.main:app` instead.

- `SERVICEKIT_ORCHESTRATOR_URL` is where the model registers. It runs on this
  machine, so `localhost` and the API port reach chap-core. The single quotes
  keep the shell from reading `$register` as a variable.
- `SERVICEKIT_HOST` and `SERVICEKIT_PORT` are the address the model registers
  itself under, which is where chap-core calls it. chap-core runs in a
  container, where `localhost` is the container itself; `host.docker.internal`
  is this machine, and varde maps that name for the chap-core containers on
  Linux too.
- `--host 0.0.0.0` makes the model listen on more than the loopback, which is
  what a call from a container arrives on.

It worked when `varde status` lists the model's service id with a recent LAST
PING. Its state is `unmanaged`, because this deployment did not start it; that
is expected. From there chap-core uses it like any other model, and
`varde models test SERVICE_ID --backtest` runs it through chap-core; the model
level (`chapkit test` in a container) is for models varde runs, so for this one
it says to use `--backtest`.

If the deployment has a registration key (`varde auth show` says so), export
`SERVICEKIT_REGISTRATION_KEY` with the value `varde auth show --reveal` prints.

The model keeps re-registering while it runs, so restarting it (or chap-core)
needs nothing else. When you are done with it, the image route is
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
