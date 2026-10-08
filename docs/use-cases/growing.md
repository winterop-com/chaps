# Growing a deployment

Nothing about the first `init` is final. Components go on and off with
`varde components enable NAME` / `disable NAME`, models with `varde models
enable ID` / `disable ID`, and `varde up` applies the change.

The move people make most is from a standalone piece to Chap: a model service
or an OCS that was running alone gets chap-core added beside it. The examples
here start from a model on its own (see [A chapkit model service on its
own](./model-alone.md)):

```sh
varde init ewars --only none --models chapkit_ewars_model
cd ewars
varde up
```

Add chap-core:

```sh
varde components enable chap-core
varde up
varde status                 # the models now register with chap-core
```

`varde components enable chap-core` prints `enabled chap-core on
http://localhost:8700` and `run `varde up` to apply`. Model services read the
orchestrator URL on that `varde up` and register.
Registration takes up to a minute after the start, so the first `varde status`
can show `running, not registered` and the line
`started under two minutes ago and registers once it is ready`.
If it does, run `varde status` again in a minute:

```text
chap-core   up   http://localhost:8700   2.4.0   latest: running f80b66f755ef, revision 7c95b2a7df62   auth: off

MODEL                STATE       REACH      LAST PING
chapkit-ewars-model  registered  port 5001  3s ago

1 model registered
```

The host ports they were given while standalone stay published until you
`varde models unexpose ID`. The chap-core compose file comes from the copy built
into varde, and the image tag is `latest`, a moving tag: the `latest: running
...` cell in `varde status` names the build that runs. `varde update
--pin-chap-core` moves it to the newest release, which is what `varde init`
with chap-core records.

The other direction works too: `varde components disable chap-core` keeps the
models, publishes a host port for each one that had none, and says where each
one now answers:

```text
$ varde components disable chap-core
disabled chap-core
stopped and removed 6 containers (chap, chapkit-ewars-model, chapkit-ewars-model-init, postgres, redis, worker); the host ports they published are free again
run `varde up` to apply
```

A model that had no host port (after `varde models unexpose`) gets one more
line, for example
`chapkit_ewars_model now registers nowhere and is published on http://localhost:5001`.
The disable stops the model containers too, so run `varde up` to start the
models again without chap-core.

chap-core's own volumes are kept either way. While chap-core is enabled,
`varde down --volumes` removes them. After the disable it does not, because
the deployment no longer has the compose file that names them. To remove them,
run `varde components enable chap-core` first, then `varde down --volumes`.

It worked when `varde status` shows the new pieces: chap-core `up` and the
models `registered` after adding chap-core, or each model `up` on its own port
after taking it away.

The same goes for any other piece: `varde components enable ocs`,
`varde components enable dhis2` or `varde models enable ID`, then `varde up`. A
DHIS2 added this way runs the default version unless you name one:
`varde components enable dhis2 --tag 2.43` (or `--image dhis2/core-dev --tag
master` for the unreleased one), or `v` on its row in `varde ui` (see [Changing the DHIS2 version](../dhis2.md#changing-the-dhis2-version)).
Only the default version has a demo database. Another version starts empty, and
the enable says so in a `warning:` line. If the deployment has chap-core, run
`varde dhis2 connect` once DHIS2 answers (see
[Connecting the Modeling App to Chap](../dhis2.md#connecting-the-modeling-app-to-chap)).

All shapes: [Use cases](../use-cases.md).
