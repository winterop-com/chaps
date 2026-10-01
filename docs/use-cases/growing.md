# Growing a deployment

Nothing about the first `init` is final. Components go on and off with
`chaps components enable NAME` / `disable NAME`, models with `chaps models
enable ID` / `disable ID`, and `chaps up` applies the change.

The move people make most is from a standalone piece to Chap: a model service
or an OCS that was running alone gets chap-core added beside it.

```sh
chaps components enable chap-core
chaps up
chaps status                 # the models now register with chap-core
```

Model services pick up the orchestrator URL on that `chaps up` and register.
The host ports they were given while standalone stay published until you
`chaps models unexpose ID`. The chap-core compose file comes from the copy built
into chaps; `chaps update --pin-chap-core` moves it to the newest release.

The other direction works too: `chaps components disable chap-core` keeps the
models, publishes a host port for each one that had none, and says where each
one now answers. chap-core's own volumes are kept either way; `chaps down
--volumes` is what removes them.

It worked when `chaps status` shows the new pieces: chap-core `up` and the
models `registered` after adding chap-core, or each model `up` on its own port
after taking it away.

The same goes for any other piece: `chaps components enable ocs`,
`chaps components enable dhis2` or `chaps models enable ID`, then `chaps up`. A
DHIS2 added this way runs the default version unless you name one:
`chaps components enable dhis2 --tag 2.43` (or `--image dhis2/core-dev --tag
master` for the unreleased one), or `v` on its row in `chaps ui` (see [Changing the DHIS2 version](../dhis2.md#changing-the-dhis2-version)).

All shapes: [Use cases](../use-cases.md).
