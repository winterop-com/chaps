# A chapkit model service on its own

One forecasting model, running and answering on its own port, with no
chap-core in front of it. For developing or checking a model: calling its API
directly, reading its `/docs`, running `chapkit test` against it, or putting it
behind something other than chap-core.

The quickest way needs no folder at all:

```sh
varde run chapkit_ewars_model                               # a marketplace model
varde run https://github.com/chap-models/chapkit_ghr_model  # by repository
varde run ghcr.io/my-org/chapkit_dengue_model:sha-1eb8cf1   # by image: use your own
varde ps
varde stop chapkit_ewars_model
varde stop --all
```

The image line is an example: `my-org` does not publish it, so that `varde run`
stops with `` chapkit-dengue-model did not start: the registry answered `denied` ``.
Put an image of your own there. See
[`did not start: the registry answered denied`](../troubleshooting.md#did-not-start-the-registry-answered-denied-for-image-so-the-image-does-not-exist-or-is-private).

Each of the other two commands prints a `starting` line, and one more line
when the model answers:

```text
starting chapkit-ewars-model (chapkit_ewars_model in ~/.local/share/varde/run/default)
running chapkit_ewars_model on http://localhost:5001 (answered in 2s)
```

varde prints the full path of the group. The docker compose progress goes to
stderr between the two lines. The repository line also says on stderr that the
URL is the marketplace model `chapkit_ghr_model`, and it answers on
`http://localhost:5002`.

`varde run` keeps the model in a group under varde's data directory, publishes
it on `127.0.0.1` only, and returns once it answers. A first run pulls the
model image, about 2 GB for each of these two, and the pull can take some
minutes. `-a` keeps it in the foreground with its log, until Ctrl-C stops it.
`varde stop ID` stops one model, and `varde stop --all` stops every model that
`varde run` started. Both keep the data volume of each model. To remove the
volumes and the groups too, add `--purge`. See [Running one model](../run.md).

For a deployment directory of your own, with `varde status`, `varde doctor` and
everything else:

```sh
varde init ewars --only none --models chapkit_ewars_model
cd ewars
varde up --wait
varde status
curl http://localhost:5001/health
varde open chapkit_ewars_model
```

`varde up --wait` returns once the model answers its own `/health`:

```text
waiting up to 300s for the model to answer
started chapkit-ewars-model
ready in 2s
  chapkit-ewars-model  up  http://localhost:5001
```

Without `--wait`, `varde up` returns when the container starts, and for some
seconds `varde status` says `running, not answering`. Then `varde status`
shows:

```text
MODEL                STATE  REACH      LAST PING
chapkit-ewars-model  up     port 5001  -

1 model up, answering on its own host port
```

The `curl` command gets the same `/health` that `varde status` asks:

```text
{"status":"healthy","checks":{"database":{"state":"healthy"},"registration":{"state":"healthy"}}}
```

`varde open chapkit_ewars_model` opens the model's API documentation, at
`http://localhost:5001/docs`, in a browser. With `--no-browser`, it prints the
address instead:

```text
the model's API documentation is at http://localhost:5001/docs
```

`varde open` with no name opens nothing. It lists the components and each
model with a host port, with the address of its `/docs`.

Your own model works the same way, from its repository or a published image:

```sh
varde init mymodel --only none
cd mymodel
varde models add https://github.com/my-org/chapkit_dengue_model   # adds and enables it
varde up --wait
```

Replace the URL with your model's repository. The repository must publish a
`sha-<commit>` image on ghcr, as the model repositories' publish workflow does.
If the URL names a repository that does not exist, `varde models add` stops
with `GitHub has no repository my-org/chapkit_dengue_model, or it is private`.
The `varde up --wait` after it then says `nothing to start`. A repository that
the marketplace lists is enabled as that marketplace model; add `--id NAME` to
keep an entry of your own beside it. See
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

Without chap-core each model gets a host port when it is enabled (the first
free one from 5001; `--port N` picks one), does not try to register anywhere,
and is judged by `varde status` and `varde doctor` through its own `/health`.
Near the start of its log, after `Application startup complete`, expect one
`registration.missing_orchestrator_url` line at error level and a
`registration.skipped` line. servicekit has nothing to register with, and the
model continues to serve. See
[`registration.missing_orchestrator_url` in a model's log](../troubleshooting.md#registrationmissing_orchestrator_url-in-a-models-log).

`varde models test chapkit_ewars_model` (or `--all`) works here: it runs
`chapkit test` inside the model's own container:

```text
testing 1 model (model level; add --backtest to go through chap-core)
chapkit-ewars-model    pass   18s   1 training, 1 prediction

1 of 1 model passes
```

`varde models test --backtest` and `varde jobs` go through chap-core, so they
refuse and name `varde components enable chap-core`.

Next: [Model services without chap-core](../components.md#model-services-without-chap-core),
[Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
