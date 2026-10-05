# chap-core from its checkout, with the models

For developing chap-core: you run chap-core from its own checkout on this
machine, and chaps runs the model services around it, registered with your
chap-core instead of one of its own.

```sh
chaps init models --chap-core-url http://localhost:8000 --models chapkit_simple_multistep_model
cd models
chaps up
```

Start chap-core from its checkout as you normally do, listening on
`0.0.0.0:8000` so the model containers can reach it. `--chap-core-url` leaves
chaps' own chap-core out and records yours, so:

- **Each model registers with your chap-core.** Its overlay sets
  `SERVICEKIT_ORCHESTRATOR_URL` to your URL, with `localhost` turned into
  `host.docker.internal`, which is this machine from inside a container.
- **Your chap-core calls each model back on its host port.** Every model gets
  one (5001 up), and registers itself as `localhost:<port>`, which is right for
  a chap-core running on this machine. chaps reads each image's command, so
  the host port reaches the app: an image that starts on `--port 8000` gets
  its host port mapped to 8000, and one that reads `PORT` is told the host
  port. Every marketplace model registers in this shape.
- **The chap-core commands talk to yours.** `chaps status` asks it for its
  health and the registered models, and `chaps jobs`, `chaps api` and
  `chaps models test` send their requests there.

```sh
chaps status                 # your chap-core, and the models registered with it
chaps models test --all
```

It worked when `chaps status` shows chap-core `up` at your URL and the models
`registered`.

**Without a deployment directory**, `chaps run --chap-core` does the same for
one model at a time:

```sh
chaps run -a --rm --chap-core http://localhost:8000 https://github.com/chap-models/chapkit_ghr_model
```

It worked when the lines after the start say `registered with
http://localhost:8000`. See [Running one model](../run.md#your-own-chap-core). The models register again every few seconds, so starting your
chap-core after `chaps up`, or restarting it, needs nothing else.

To point an existing deployment at your chap-core, or back:

```sh
chaps components disable chap-core                            # stop chaps' own, if it has one
chaps components enable chap-core --url http://localhost:8000
chaps components list                                         # chap-core: external
chaps up

chaps components disable chap-core                            # forget it again
chaps components enable chap-core                             # or run chaps' own again
```

If your chap-core is itself a container (another compose project, which is
what `make restart` in chap-core's checkout starts) rather than a process on
this machine, it cannot call the models at `localhost`; they have to register
as `host.docker.internal`. `chaps init --chap-core-url` and `components enable
chap-core --url` see that for themselves when a container publishes the URL's
port at the time, and say so; otherwise add `--models-host host.docker.internal`
to `components enable chap-core --url`. Without it the models still register,
and `chaps status` shows them `registered, unreachable`.
For a chap-core on another machine, pass the name that machine reaches this one
by.

If your chap-core requires a registration key, run `chaps auth enable` (which
makes every overlay pass `SERVICEKIT_REGISTRATION_KEY` from `.env`), then set
that line in `.env` to your chap-core's key and run `chaps up`. See
[Authentication](../auth.md).

More: [A chap-core elsewhere](../components.md#a-chap-core-elsewhere).

All shapes: [Use cases](../use-cases.md).
