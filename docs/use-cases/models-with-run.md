# Any number of model services, no folder

One chapkit model or ten, each answering on its own port, with no `chaps init`,
no deployment directory and no chap-core. For calling models' APIs directly,
comparing several side by side, or a tool (a UI, a notebook, a test harness)
that only needs "start this model and tell me where it answers".

From any directory that is not a deployment:

```sh
chaps run chapkit_ewars_model --detach
chaps run auto_arima_chapkit --detach
chaps run https://github.com/chap-models/chapkit_ghr_model --detach
chaps ps                     # every model, its state and its URL
```

With `--detach`, each `chaps run` returns once the model answers on its
`/health` and prints where it answers: `running chapkit_ewars_model on
http://localhost:5001`. Without it, a terminal keeps the first one in the
foreground with its log, until Ctrl-C stops it. Each
model gets the next free port from 5001 up, its own container and its own data
volume, published on `127.0.0.1` only. The models do not depend on each other.
A marketplace id, a GitHub repository URL, a ghcr image and a local `name:tag`
are all accepted; see [Running one model](../run.md#what-the-model-can-be).

It worked when `chaps ps` lists every model as `up`, each with its own URL.
`chaps top` shows the same, live.

## Starting many at once

`--no-wait` returns as soon as the container starts, so a whole set can start
in parallel. Runs into the same group take turns where they have to and never
step on each other:

```sh
for m in chapkit_ewars_model auto_arima_chapkit chapkit_ghr_model; do
  chaps run "$m" --no-wait &
done
wait
chaps ps                     # `running, not answering` until each one is ready
```

Running a model that already runs starts nothing new: it reports the same id
and port. A repository or image you ran before is that same model again, not a
copy; `--id auto` adds a second copy beside it when you want one.

## Keeping sets apart: groups

Without a folder, chaps keeps the models in a *group* under its data
directory, `default` unless you name one. A group per experiment keeps sets
apart, and every group can be listed and stopped on its own:

```sh
chaps run chapkit_ewars_model --group dengue
chaps run auto_arima_chapkit --group dengue
chaps run chapkit_ewars_model --group malaria   # a second, separate copy
chaps ps --group dengue
```

The same model in two groups is two containers with two ports and two data
volumes. Group names are lowercase letters, digits, `-` and `_`.

## From a script

Under `--json` each command prints one JSON document on stdout, and docker's
progress goes to stderr:

```sh
chaps --json run chapkit_ewars_model --group dengue
```

The fields a tool needs are `url`, `port`, `id`, `group` and `project_dir`; a
failure is `{"ok": false, "error": ..., "hint": ...}` with a non-zero exit. See
[Running one model](../run.md) for every field and
[`--json` for scripts and tools](../commands.md#--json-for-scripts-and-tools).

## Stopping

```sh
chaps stop chapkit_ewars_model            # one model, in whichever group has it
chaps stop --group dengue                 # every model in one group
chaps stop --all                          # every model in every group
chaps stop --group dengue --purge         # and its data, and the group itself
```

A stop keeps the model's data volume, so the next `chaps run` of it picks up
where it left off. `--purge` deletes the data too, and removes a group it
leaves empty: its network, its directory and every volume it had, including
those of models stopped earlier.

## When a folder is the better fit

A group is an ordinary chaps deployment that chaps keeps for you, so the other
commands reach it with `-C`: `chaps -C ~/.local/share/chaps/run/dengue logs
chapkit-ewars-model`. If you find yourself doing that often, or want
chap-core, OCS or DHIS2 next to the models, use a folder of your own instead:
[Several model services side by side](./models-alone.md).

To reach the models from other machines, start them with `--bind 0.0.0.0`.

All shapes: [Use cases](../use-cases.md).
