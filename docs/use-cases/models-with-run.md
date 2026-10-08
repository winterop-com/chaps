# Any number of model services, no folder

One chapkit model or ten, each answering on its own port, with no `varde init`,
no deployment directory and no chap-core. For calling models' APIs directly,
comparing several side by side, or a tool (a UI, a notebook, a test harness)
that only needs "start this model and tell me where it answers".

From any directory that is not a deployment:

```sh
varde run chapkit_ewars_model
varde run auto_arima_chapkit
varde run https://github.com/chap-models/chapkit_ghr_model
varde ps                     # every model, its state and its URL
```

Each `varde run` returns once the model answers on its `/health` and prints
where it answers. Between the two lines, docker compose shows its progress:

```text
starting chapkit-ewars-model (chapkit_ewars_model in /home/me/.local/share/varde/run/default)
...
running chapkit_ewars_model on http://localhost:5001 (answered in 2s)
```

The first run of a model pulls its image. A marketplace model image is several
gigabytes, so this can take many minutes; a later run starts in seconds.

The GitHub URL above is a model that the marketplace lists. Before the
`starting` line, varde says so, then reads the user of the image (this pulls
the image), and then starts that marketplace model:

```text
https://github.com/chap-models/chapkit_ghr_model is the marketplace model chapkit_ghr_model; enabling that (pass `--id <other>` to add it beside it instead)
asking ghcr.io/chap-models/chapkit_ghr_model:sha-dfb2e3f what uid `app` is (this pulls the image)
```

Each model gets the next free port from 5001 up, its own container and its own
data volume, published on `127.0.0.1` only. The models do not depend on each
other. A marketplace id, a GitHub repository URL, a ghcr image and a local
`name:tag` are all accepted; see
[Running one model](../run.md#what-the-model-can-be).

It worked when `varde ps` lists every model as `up`, each with its own URL:

```text
GROUP    ID                   SERVICE              STATE  URL
default  auto_arima_chapkit   auto-arima-chapkit   up     http://localhost:5002
default  chapkit_ewars_model  chapkit-ewars-model  up     http://localhost:5001
default  chapkit_ghr_model    chapkit-ghr-model    up     http://localhost:5003
```

`varde top` shows the same, live.

## Starting many at once

`--no-wait` returns as soon as the container starts, so a whole set can start
in parallel. Runs into the same group take turns where they have to and never
step on each other; a run that waits for its turn says `another varde command
is changing this deployment; waiting for it to finish`.

```sh
for m in chapkit_ewars_model auto_arima_chapkit chapkit_ghr_model; do
  varde run "$m" --no-wait &
done
wait
varde ps                     # `running, not answering` until each one is ready
```

With `--no-wait`, each run prints `started` in place of `running`. The runs
take their turns in no fixed order, so the ports can go to the models in a
different order each time. Read each URL from `varde ps` or from `--json`.

Running a model that already runs starts nothing new: it reports the same id
and port. A repository or image you ran before is that same model again, not a
copy; `--id auto` adds a second copy beside it when you want one. A run by the
GitHub URL of a marketplace model works as a run by its id: the second run
does not read the image again, recreate the container, or move the port.

## Keeping sets apart: groups

Without a folder, varde keeps the models in a *group* under its data
directory, `default` unless you name one. A group per experiment keeps sets
apart, and every group can be listed and stopped on its own:

```sh
varde run chapkit_ewars_model --group dengue
varde run auto_arima_chapkit --group dengue
varde run chapkit_ewars_model --group malaria   # one more, separate copy
varde ps --group dengue
```

```text
GROUP   ID                   SERVICE              STATE  URL
dengue  auto_arima_chapkit   auto-arima-chapkit   up     http://localhost:5005
dengue  chapkit_ewars_model  chapkit-ewars-model  up     http://localhost:5004
```

The same model in two groups is two containers with two ports and two data
volumes. Group names are lowercase letters, digits, `-` and `_`.

A group is an ordinary varde deployment that varde keeps for you, so the other
commands reach it with `-C`. The `starting` line of `varde run` names the
group directory, and `varde -v run` prints the full log command:

```sh
varde -C ~/.local/share/varde/run/dengue logs chapkit-ewars-model
```

The path is the default data directory; `$VARDE_DATA_DIR` or `$XDG_DATA_HOME`
moves it ([Where it runs: groups](../run.md#where-it-runs-groups)).

For a group that does not exist, `varde ps --group` says
`` there is no `varde run` group X; `varde ps` lists every group ``.

## From a script

Under `--json` each command prints one JSON document on stdout, and docker's
progress goes to stderr:

```sh
varde --json run chapkit_ewars_model --group dengue
```

The fields a tool needs are `url`, `port`, `id`, `group` and `project_dir`; a
failure is `{"ok": false, "error": ..., "hint": ...}` with a non-zero exit. See
[Running one model](../run.md) for every field and
[`--json` for scripts and tools](../commands.md#--json-for-scripts-and-tools).

## Stopping

```sh
varde stop chapkit_ghr_model                     # one model, in whichever group has it
varde stop chapkit_ewars_model --group malaria   # one model in one group
varde stop --group dengue                        # every model in one group
varde stop --all                                 # every model in every group
varde stop --group dengue --purge                # and its data, and the group itself
```

An id alone works only when one group has that model. After the steps above,
`chapkit_ewars_model` runs in three groups, so `varde stop chapkit_ewars_model`
stops nothing and says:

```text
error: chapkit_ewars_model runs in the groups default, dengue, malaria; name one with `--group default`
```

A stop keeps the model's data volume, so the next `varde run` of it continues
with the same data. The port can be a different one. `--purge` deletes the
data too, and removes a group it leaves empty: its network, its directory and
every volume it had, including those of models stopped earlier. To remove
every group and all its data, use `varde stop --all --purge`.

In the order above, `dengue` has no running model when `--purge` comes, so
varde says:

```text
nothing was running to stop
removed group dengue
```

`varde -v stop` also names each volume that it removed.

## When a folder is the better fit

If you often reach a group with `-C`, or want chap-core, OCS or DHIS2 next to
the models, use a folder of your own instead:
[Several model services side by side](./models-alone.md).

To reach the models from other machines, start them with `--bind 0.0.0.0`.

All shapes: [Use cases](../use-cases.md).
