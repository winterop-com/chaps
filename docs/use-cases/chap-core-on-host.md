# chap-core from its checkout, with the models

For developing chap-core: you run chap-core from its own checkout on this
machine, and varde runs the model services around it, registered with your
chap-core instead of one of its own.

1. Start chap-core from its checkout as you normally do. Make sure that it
   listens on `0.0.0.0:8000`, so that the model containers can reach it.
2. Make the deployment and start the models:

   ```sh
   varde init models --chap-core-url http://localhost:8000 --models chapkit_simple_multistep_model
   cd models
   varde up
   ```

`--chap-core-url` leaves out varde's own chap-core, with its worker, Valkey
and PostgreSQL. Your chap-core uses the database and the Valkey that you give
it. varde records the URL of your chap-core, so:

- **Each model registers with your chap-core.** Its overlay sets
  `SERVICEKIT_ORCHESTRATOR_URL` to your URL, with `localhost` turned into
  `host.docker.internal`, which is this machine from inside a container.
- **Your chap-core calls each model back on its host port.** Every model gets
  one (5001 up), and registers itself as `localhost:<port>`, which is right for
  a chap-core running on this machine. varde reads each image's command, so
  the host port reaches the app: an image that starts on `--port 8000` gets
  its host port mapped to 8000, and one that reads `PORT` is told the host
  port. Every marketplace model registers in this shape.
- **The chap-core commands talk to yours.** `varde status` asks it for its
  health and the registered models, and `varde jobs`, `varde api` and
  `varde models test --backtest` send their requests there. `varde models
  test` without `--backtest` tests each model inside its container, and does
  not need chap-core.

```sh
varde status                 # your chap-core, and the models registered with it
```

```text
chap-core   up   http://localhost:8000   2.5.0.dev0   auth: off

MODEL                           STATE                       REACH      LAST PING
chapkit-simple-multistep-model  registered, not configured  port 5001  6s ago

1 model registered
chapkit-simple-multistep-model: chap-core has no configured model for it, so nothing can run it; run `varde models configure`
```

It worked when `varde status` shows chap-core `up` at your URL and the models
registered. The version is the one that your checkout reports. A model
registers some seconds after `varde up`. Until then, `varde status` shows it
`running, not registered` and says `started under two minutes ago`; run
`varde status` again in a moment.

`registered, not configured` is the normal state after a plain `varde up`.
chap-core makes no configured model when a model registers. A backtest and the
Modeling App need one. To make them in your chap-core, run `varde models
configure`:

```sh
varde models configure
varde models test --all
```

```text
chapkit_simple_multistep_model: created configured model monthly_climate
chapkit_simple_multistep_model: created configured model monthly_selfhistory
```

```text
testing 1 model (model level; add --backtest to go through chap-core)
chapkit-simple-multistep-model    pass    6s   1 training, 1 prediction

1 of 1 model passes
```

After that, `varde status` shows the model as `registered`, and
`1 model registered`. The configured models are in the database of your
chap-core, so they stay when you restart it. See
[Configured models](../models.md#configured-models).

Start chap-core before `varde up`. A model tries to register five times in
its first seconds, and then it stops. If chap-core starts after that, the
model stays `running, not registered`. After two minutes, the first line
under the summary of `varde status` gives the way out:

```text
chapkit-simple-multistep-model: restart it with `varde restart --all chapkit-simple-multistep-model`
```

To make every model of the deployment register again, run `varde restart
--all`. A restart of chap-core needs nothing more. Each registered model pings
chap-core about every 15 seconds. If chap-core does not know the model, the
model registers again, in about a minute. See
[A model is running but not registered](../troubleshooting.md#a-model-is-running-but-not-registered).

**Without a deployment directory**, `varde run --chap-core` does the same for
one model at a time:

```sh
varde run -a --rm --chap-core http://localhost:8000 https://github.com/chap-models/chapkit_ghr_model
```

If the image is not on this machine, the first run pulls it, and that can take
some minutes: this image is about 7 GB.

```text
running chapkit_ghr_model on http://localhost:5002 (answered in 2s)
registered with http://localhost:8000
following the log of chapkit-ghr-model; Ctrl-C stops it
```

It worked when the lines after the start say `registered with
http://localhost:8000`. Ctrl-C stops the model, and `--rm` also removes its
data: varde prints `stopped chapkit_ghr_model, and removed its data`, then
`removed group default` when the group has no other model. See
[Running one model](../run.md#your-own-chap-core).

To point an existing deployment at your chap-core, or back:

```sh
varde components disable chap-core                            # stop varde's own, if it has one
varde components enable chap-core --url http://localhost:8000
varde components list                                         # chap-core: external
varde up

varde components disable chap-core                            # forget it again
varde components enable chap-core                             # or run varde's own again
```

`varde components disable chap-core` stops and removes the containers of
chap-core and of the models. The next `varde up` starts the models again.
It also gives each model a host port, because your chap-core calls the models
there. When varde runs its own chap-core again, the models keep that port. The
`REACH` column of `varde status` then shows the port, not `via chap-core`. To
remove the port, run `varde models unexpose ID`.

If your chap-core is itself a container (another compose project, which is
what `make restart` in chap-core's checkout starts), it cannot call the models
at `localhost`. The models must register as `host.docker.internal`.
`varde init --chap-core-url` and `varde components enable chap-core --url` find
this themselves if a container publishes the port of the URL at that time.
`varde init` and `varde components enable` tell it in a warning, for example:

```text
warning: chap-core at http://localhost:8700 is the container `own-dd1b2e-chap-1`, so it calls the models back at host.docker.internal; if it is a process on this machine instead, run `varde components enable chap-core --url http://localhost:8700 --models-host localhost`
```

If varde did not find it, do these steps:

1. Run `varde components enable chap-core --url URL --models-host host.docker.internal`.
2. Run `varde up`.

Without `--models-host`, the models still register, and `varde status` shows
them `registered, unreachable`, with this command on the line under the table.
If chap-core runs on another machine, run `varde components enable chap-core
--url URL --models-host HOST`, with HOST the name or address that machine
reaches this one at.

If your chap-core requires a registration key or an API token, do these steps:

1. If your chap-core checks an API token, run `varde auth enable --token
   TOKEN` with its token. If not, run `varde auth enable`.
2. In `.env`, set the line `SERVICEKIT_REGISTRATION_KEY` to the key of your
   chap-core.
3. Run `varde up`.

For a chap-core elsewhere, `varde auth enable` says what it does not do:

```text
API authentication is on
run `varde up` to restart the models with authentication
varde does not change the chap-core at http://localhost:8000; set `SERVICEKIT_REGISTRATION_KEY` in `.env` to its registration key
```

After that, `varde status` shows `auth: on`. This means that varde sends a
token. It does not mean that your chap-core checks it.

`varde auth enable` writes an API token and a registration key to `.env`, and
every overlay then passes `SERVICEKIT_REGISTRATION_KEY` from `.env`. varde
sends the API token to your chap-core in `varde status`, `api`, `jobs` and
`models test --backtest`. If authentication is already on, `varde auth enable
--token` with another token stops with `--token was not used: API
authentication is already on with another token`, and changes nothing. To give
a different token, run `varde auth disable`, then `varde auth enable --token
TOKEN`. The registration key in `.env` stays. See
[Authentication](../auth.md).

More: [A chap-core elsewhere](../components.md#a-chap-core-elsewhere).

All shapes: [Use cases](../use-cases.md).
