# Your model from its checkout, with CHAP

For developing a chapkit model: chaps runs chap-core, and you run the model
from its own checkout on this machine (with `uv run`, a debugger, hot reload),
registered with that chap-core like any deployed model. Nothing about the model
has to be built into an image.

```sh
chaps init mychap --models none
cd mychap
chaps up
chaps status                 # chap-core up, no models yet
```

Then, in the model's checkout, start it on a port of its own and tell it where
chap-core is and where chap-core can call it back:

```sh
export SERVICEKIT_ORCHESTRATOR_URL='http://localhost:8000/v2/services/$register'
export SERVICEKIT_HOST=host.docker.internal
export SERVICEKIT_PORT=8001
uv run uvicorn my_model.main:app --host 0.0.0.0 --port 8001
```

- `SERVICEKIT_ORCHESTRATOR_URL` is where the model registers. It runs on this
  machine, so `localhost` and the API port reach chap-core. The single quotes
  keep the shell from reading `$register` as a variable.
- `SERVICEKIT_HOST` and `SERVICEKIT_PORT` are the address the model registers
  itself under, which is where chap-core calls it. chap-core runs in a
  container, where `localhost` is the container itself; `host.docker.internal`
  is this machine, and chaps maps that name for the chap-core containers on
  Linux too.
- `--host 0.0.0.0` makes the model listen on more than the loopback, which is
  what a call from a container arrives on.

It worked when `chaps status` lists the model's service id as `registered`.
It shows up as `unmanaged`, because this deployment did not start it; that is
expected. From there chap-core uses it like any other model, and
`chaps models test` can run it.

If the deployment has a registration key (`chaps auth show` says so), export
`SERVICEKIT_REGISTRATION_KEY` with the value `chaps auth show --reveal` prints.

The model keeps re-registering while it runs, so restarting it (or chap-core)
needs nothing else. When you are done with it, the image route is
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
