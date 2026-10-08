# Several model services side by side

Two or more chapkit model services in one deployment directory, with no
chap-core. Each model gets its own host port and its own data volume, and the
models do not depend on each other. For one model, see
[A chapkit model service on its own](./model-alone.md).

```sh
varde init models --only none --models chapkit_ewars_model,auto_arima_chapkit
cd models
varde up --wait
varde status
varde models list --enabled  # the PORT column says where each one answers
```

`varde init` gives each model the next free port from 5001 up, and prints it:

```text
created a deployment in /path/to/models
components: none
enabled chapkit_ewars_model v1.0.4 on http://localhost:5001
enabled auto_arima_chapkit v1.0.2 on http://localhost:5002
run `cd /path/to/models && varde up` to start the deployment
```

A first `varde up` pulls the model images, about 2 GB to download for each of
these two (6 to 7 GB each on disk). The pull can take some minutes. The images
are amd64 only, so on an arm64 machine they run under emulation.

`varde up --wait` returns once each model answers its own `/health`. Without
`--wait`, `varde up` returns when the containers start, and for some seconds
`varde status` can say `running, not answering`. After compose's own lines,
`varde up --wait` ends with:

```text
waiting up to 300s for the 2 models to answer
started auto-arima-chapkit, chapkit-ewars-model
ready in 2s
  auto-arima-chapkit   up  http://localhost:5002
  chapkit-ewars-model  up  http://localhost:5001
```

Then `varde status` shows:

```text
MODEL                STATE  REACH      LAST PING
auto-arima-chapkit   up     port 5002  -
chapkit-ewars-model  up     port 5001  -

all 2 models up, each answering on its own host port
```

It worked when `varde status` shows every model as `up`, each on its own port.
Each model answers on `http://localhost:<port>`, with its API documentation at
`/docs`. The ids for `--models` are in the ID column of `varde models list`.
That command also works outside a deployment directory.

Compose publishes these ports on every address of the machine, and the models
have no login. To keep a model on this machine only, see
[Which address a model port is published on](../ports.md#which-address-a-model-port-is-published-on).

Without a folder, `varde run` does the same one model at a time:
[Any number of model services, no folder](./models-with-run.md).

All shapes: [Use cases](../use-cases.md).
