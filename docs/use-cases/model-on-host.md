# Your model from its checkout, with Chap

For developing a chapkit model: varde runs chap-core, and you run the model
from its own checkout on this machine (with `uv run`, a debugger, hot reload),
registered with that chap-core like any deployed model. Nothing about the model
has to be built into an image.

```sh
varde init mychap --models none
cd mychap
varde up --wait
varde status
```

`varde up --wait` returns once chap-core answers. Without `--wait`, `varde up`
returns when the containers start, and for some seconds `varde status` says
`starting`. Then `varde status` shows chap-core up and no models yet:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

no models enabled; run `varde models enable ID` to add one
```

## Start the model

Then, in the model's checkout, start it on a port of its own and tell it where
chap-core is and where chap-core can call it back:

```sh
export SERVICEKIT_ORCHESTRATOR_URL='http://localhost:8700/v2/services/$register'
export SERVICEKIT_HOST=host.docker.internal
export SERVICEKIT_PORT=8001
export GIT_REVISION=$(git rev-parse HEAD)
uv run uvicorn main:app --host 0.0.0.0 --port 8001
```

`main:app` is the app in a `main.py` at the top of the checkout, which is the
layout chapkit models use; a model whose `main.py` is inside a package runs as
`PACKAGE.main:app` instead. The first `uv run` in a checkout makes its `.venv`
and installs the dependencies of the model.

- `SERVICEKIT_ORCHESTRATOR_URL` is where the model registers. It runs on this
  machine, so `localhost` and the API port reach chap-core. The single quotes
  keep the shell from reading `$register` as a variable.
- `SERVICEKIT_HOST` and `SERVICEKIT_PORT` are the address the model registers
  itself under, which is where chap-core calls it. chap-core runs in a
  container, where `localhost` is the container itself; `host.docker.internal`
  is this machine. The chap-core compose file maps that name for the chap-core
  containers on Linux too.
- `GIT_REVISION` is the commit the model reports as its `git_revision`.
  chap-core v2.4.0 stores the model template only for a service that reports
  a revision. Without it, the model registers and shows in `varde status` with
  no warning, but chap-core has no template for it. The chap-core log
  (`varde logs chap`) then says `is stored from revision None, but its source
  now reports revision None`. `varde models configs sync` fails with
  `answered HTTP 409 Conflict` and the same reason. Its warning tells you to
  run it again, but that does not help: set `GIT_REVISION`, then start the
  model again.
- `--host 0.0.0.0` makes the model listen on more than the loopback, which is
  what a call from a container arrives on.

It worked when `varde status` lists the model's service id with a recent LAST
PING. For the minimalist example in `~/dev/chap-models`, the status is:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off

MODEL                          STATE      REACH                             LAST PING
chapkit-minimalist-example-py  unmanaged  http://host.docker.internal:8001  7s ago

no models enabled here; the unmanaged one above registered from outside this deployment
```

Its state is `unmanaged`, because this deployment did not start it; that is
expected.

chap-core stores one template for each model version, with the revision it
had at the first registration. Changes you do not commit keep the revision, so
hot reload needs nothing more. If you commit and export `GIT_REVISION` again,
the revision changes. chap-core then refuses the backtest with `is stored from
revision '...', but its source now reports revision '...'`. The backtest row
shows the start of that reason, and `varde jobs logs JOB_ID` shows all of it.
`varde status` warns: `SERVICE_ID: chap-core stores its model template VERSION
from another git revision and refuses to run it; set a new version in the
model, then start it again`. To continue:

1. Set a new `version` in the `MLServiceInfo` in `main.py`.
2. Start the model again.
3. Run `varde models configs sync SERVICE_ID` again.

The configured model from before belongs to the old version. Without step 3,
the `varde status` warning goes away, but the backtest still fails with the
same reason.

## Test it through chap-core

The model level of `varde models test` (`chapkit test` in a container) is for
models that varde runs. For this model, `varde models test SERVICE_ID` skips,
names `--backtest`, and exits with status 1:

```text
testing 1 model (model level; add --backtest to go through chap-core)
chapkit-minimalist-example-py    skip    0s   varde does not run it, so there is no container to test it in
  run `varde models test chapkit-minimalist-example-py --backtest` to test it through chap-core

0 pass, 1 skipped
warning: no model was tested, because every one was skipped; the line under each row says why
```

A backtest uses a configured model: a template with a set of option values.
chap-core v2.4.0 makes no configured model from a registered service, so make
one. Replace `SERVICE_ID` with the id that `varde status` shows:

```sh
varde models configs sync SERVICE_ID
varde models test SERVICE_ID --backtest
```

`varde models configs sync` stores the template of the service and makes one
configured model, `default`, with the defaults of the service. chap-core names
it `SERVICE_ID`:

```text
SERVICE_ID: created configured model default
```

If the model has options, or reads covariates that it does not declare, make
the configured model through the API instead (this example uses `jq`):

```sh
TEMPLATE_ID=$(varde api GET /v1/crud/model-templates | jq '.[] | select(.name == "SERVICE_ID") | .id')
varde api POST /v1/crud/configured-models --data "{\"name\": \"dev\", \"modelTemplateId\": $TEMPLATE_ID, \"additionalContinuousCovariates\": [\"rainfall\", \"mean_temperature\"]}"
```

chap-core names that configured model `SERVICE_ID:dev`. Add the options as
`"userOptionValues"`. If the template id is empty, chap-core has no template
for the model; make sure that `GIT_REVISION` was set when the model started.
The backtest uses the configured model `SERVICE_ID` if there is one, and else
`SERVICE_ID:dev`.

It worked when the backtest passes and prints its scores:

```text
testing 1 model (through chap-core: a dataset, a backtest and its scores)
chapkit-minimalist-example-py    pass      13s   crps 16.7  mae 16.7  rmse 18.0

1 of 1 model passes
```

The scores change from run to run, because the sample data is random. Add
`--seed N` to get the same data each time.

Without the configured model, the backtest skips with `chap-core has no
configured model for SERVICE_ID`, and names `varde models configs sync`.

## A registration key

If the deployment has a registration key (`varde auth show` says
`Registration key    on`), export `SERVICEKIT_REGISTRATION_KEY` before you
start the model. Its value is the line of that name in the deployment's `.env`:

```sh
export $(grep '^SERVICEKIT_REGISTRATION_KEY=' /path/to/mychap/.env)
```

Without the key, chap-core answers `401 Unauthorized`. The model log shows
`registration.attempt_failed` five times, then `registration.failed`, and the
model does not try again until you start it again. `varde status` does not
list the model.

## Restarts

The model keeps its registration alive with a ping every 10 seconds. chap-core
keeps the registrations in `redis`, so a restart of `chap` alone keeps them.
If chap-core loses the registration, for example when `redis` restarts too,
the model log shows `keepalive.service_not_found`. The model registers again
30 seconds after that, so about 40 seconds after the restart, with nothing to
do on your side. When you are done with the model, the image route is
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
