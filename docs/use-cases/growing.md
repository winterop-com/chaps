# Growing a deployment

Nothing about the first `init` is final. Components go on and off with
`varde components enable NAME` / `disable NAME`, models with `varde models
enable ID` / `disable ID`, and `varde up` applies the change.

The move people make most is from a standalone piece to Chap: a model service
or an OCS that was running alone gets chap-core added beside it.

```sh
varde components enable chap-core
varde up
varde status                 # the models now register with chap-core
```

Model services read the orchestrator URL on that `varde up` and register.
The host ports they were given while standalone stay published until you
`varde models unexpose ID`. The chap-core compose file comes from the copy built
into varde; `varde update --pin-chap-core` moves it to the newest release.

The other direction works too: `varde components disable chap-core` keeps the
models, publishes a host port for each one that had none, and says where each
one now answers. chap-core's own volumes are kept either way; `varde down
--volumes` is what removes them.

It worked when `varde status` shows the new pieces: chap-core `up` and the
models `registered` after adding chap-core, or each model `up` on its own port
after taking it away.

The same goes for any other piece: `varde components enable ocs`,
`varde components enable dhis2` or `varde models enable ID`, then `varde up`. A
DHIS2 added this way runs the default version unless you name one:
`varde components enable dhis2 --tag 2.43` (or `--image dhis2/core-dev --tag
master` for the unreleased one), or `v` on its row in `varde ui` (see [Changing the DHIS2 version](../dhis2.md#changing-the-dhis2-version)).

All shapes: [Use cases](../use-cases.md).
