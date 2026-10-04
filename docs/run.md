# Running one model

`chaps run` starts one model service and prints where it answers. There is no
`chaps init` first and no directory to keep: name the model, by marketplace id,
GitHub repository or image, and use the URL.

```sh
chaps run chapkit_ewars_model
chaps run https://github.com/chap-models/chapkit_ghr_model
chaps run ghcr.io/my-org/chapkit_dengue_model:sha-1eb8cf1
chaps run my-dengue-model:dev                 # built on this machine
```

```text
starting chapkit-ewars-model (chapkit_ewars_model in /home/me/.local/share/chaps/run/default)
running chapkit_ewars_model on http://localhost:5001 (answered in 41s)
following the log of chapkit-ewars-model; Ctrl-C stops it
```

chaps prints the URL once the model answers its own `/health`, so the URL works
when the line is printed. A first run pulls the image, which takes a while for
the R-INLA models; `--timeout SECONDS` (300 by default) is how long it waits
before it fails naming the state the model was in.

## In the foreground, and Ctrl-C

In a terminal, `chaps run` stays in the foreground, as `docker run` does. After
the URL, it shows the model's log, and Ctrl-C stops the model:

```text
stopped chapkit_ewars_model; its data stays, and `chaps run chapkit_ewars_model` starts it again with it
```

Ctrl-C takes the model out of its group, as `chaps stop` does. The model's data
volume stays, so the next run starts with the model's trained state. `--rm`
removes the data volume too.

`chaps run` returns at once, and leaves the model running, in these cases:

| When | Why |
| --- | --- |
| `--detach` | You asked for it: start several models from one terminal, for example. |
| stdin or stdout is not a terminal | A script, CI or an AI assistant must not wait for a Ctrl-C. |
| `--json` | A tool reads one document and goes on. |
| in a deployment of your own | The deployment lives on its own; `chaps down` stops it. |
| `--no-wait` | It returns as soon as the container started. |

Then the last line names the `chaps stop` that stops the model, and its log
command. `--attach` keeps the foreground also where it would return, for example
when you pipe the log; with `--no-wait`, you watch the model start in its log.

A second Ctrl-C exits at once. A model that ran before the command is not
stopped: Ctrl-C then stops only the log.

**Ctrl-C during the start**, in either mode, takes back out what the start put
in: a model that this run enabled goes back out of the group, as after a failed
start. The command then exits with 130:

```text
error: stopped by Ctrl-C; chapkit_ewars_model is taken back out of /home/me/.local/share/chaps/run/default
```

## What the model can be

| `MODEL` | What `run` does |
| --- | --- |
| A marketplace id or service id: `chapkit_ewars_model` | Enables it at the catalogue's stable version. |
| A GitHub repository the marketplace lists | The same: the catalogue's reviewed pin, not the newest build of the branch. |
| Any other GitHub repository URL | Adds it the way [`chaps models add`](./models.md#models-outside-the-marketplace) does: the newest published `sha-` build of the default branch. Needs the network. |
| A ghcr image with a tag or digest | Pins exactly that image. A tag the marketplace publishes for one of its models enables that marketplace version instead. |
| A local image, `name:tag` with no registry | Runs the image from the local store, never pulled. |

A model that is already enabled is started as it is: a second `chaps run` of
the same id does not move its version or its port. The same holds for a
repository or an image: one added before, by an earlier `chaps run` or
`models add`, is that entry again, enabled or not, so running the same URL twice
starts one model, not two. An image counts as the same only at the same tag.
`--id` names an added model (`--id auto` picks a free one), exactly as on
`models add`; an `--id` other than the one the source was added under adds a
second copy beside it.

A word with no `:`, `/` or `@` is a marketplace id, never a local image (a local
image always carries a tag), so `chaps run does_not_exist` says there is no such
model and names `chaps models search`. A marketplace template is refused like it
is on `models enable`; `--allow-template` starts it anyway.

When the container cannot be started - the image does not exist, the pull is
denied, the port is taken - `run` says what compose said and takes the model it
enabled back out, so `chaps ps` does not list it and the next `chaps run` of the
same source starts afresh. The definition of an added model stays.

## Where it runs: groups

Outside a deployment, `chaps run` keeps its models in a *group*: a deployment
chaps owns under its data directory, `run/<group>/`. The data directory is
`$CHAPS_DATA_DIR`, else `$XDG_DATA_HOME/chaps`, else `~/.local/share/chaps`.
`--group NAME` picks one; without it the group is `default`. The first
`chaps run` into a group creates it, as `chaps init --only none --models none`
would, so there is no chap-core and no component in it: only model services,
each on its own host port.

Groups keep models apart. Each one is a compose project of its own, so its
containers, volumes and network carry its name, and stopping a group touches
nothing else. `.chaps/project.yaml` records the group as `group: <name>`, and
every container in it carries the labels `com.winterop.chaps.kind: run` and
`com.winterop.chaps.group: <name>`, so
`docker ps --filter label=com.winterop.chaps.group=trial` lists one group's
containers ([Container labels](./concepts.md#container-labels)):

```sh
chaps run chapkit_ewars_model --detach                      # group default
chaps run auto_arima_chapkit --group trial --detach
chaps run chapkit_ghr_model --group trial --detach
chaps ps
```

```text
GROUP    ID                  SERVICE             STATE        URL
default  chapkit_ewars_model chapkit-ewars-model up           http://localhost:5001
trial    auto_arima_chapkit  auto-arima-chapkit  up           http://localhost:5002
trial    chapkit_ghr_model   chapkit-ghr-model   not running  http://localhost:5003
groups live in /home/me/.local/share/chaps/run
```

`--port auto` takes the lowest port in 5001-5999 that nothing on the machine
holds and no group has given to a model, running or stopped, so two groups
never collide.

Inside a deployment of your own - the working directory, or `-C DIR` - `run`,
`ps` and `stop` work on that deployment instead, and `--group` is refused,
because the deployment is the group. So a tool that only wants "start this
model and give me its URL" has one code path whether or not someone ran
`chaps init`.

## Only this machine

A group publishes every model port on `127.0.0.1`, so a model `chaps run`
started is reachable from this machine and not from the network: models have no
login. `--bind 0.0.0.0` publishes one on every address instead. Inside a
deployment, `run` publishes as that deployment does. See
[Ports](./ports.md#which-address-a-model-port-is-published-on).

## Listing and stopping

`chaps ps` lists every model in every group, or in one with `--group`, with the
state `chaps status` would give it: `up` when its `/health` answers,
`running, not answering` while it starts or when it failed to,
`not running` when its container is gone.

`chaps stop ID` stops a model and takes its overlay away, in whichever group
has it; when two groups do, it asks for `--group`. `chaps stop --all` stops
every model in every group, and `chaps stop --group NAME` every model in one.
The data volume stays, as with `models disable`, and so does the definition of
an added model, so `chaps run ID` starts it again without asking GitHub.
`--purge` takes the volume too, and the group it leaves with no model in it
goes as well: its network, its directory and every volume compose made for
it, including the volume of a model stopped earlier without `--purge`. Only
that group: another group with no model left, emptied by an earlier plain
`stop`, keeps its data. `chaps stop --group NAME --purge` empties and removes
that one group, and `chaps stop --all --purge` every group. A volume docker
will not remove keeps its group, so the same `chaps stop --group NAME --purge`
can finish the job once the volume is free.

The deployment behind a group is an ordinary chaps deployment, so every other
command works on it with `-C`: `chaps -C ~/.local/share/chaps/run/default logs
chapkit-ewars-model`, `chaps -C ... status`. Messages from inside a group name
their commands that way.

## For tools

Every one of the three takes `--json` and prints one document on stdout:

```json
{
  "ok": true,
  "id": "chapkit_ewars_model",
  "service_id": "chapkit-ewars-model",
  "port": 5001,
  "bind": "127.0.0.1",
  "url": "http://localhost:5001",
  "group": "default",
  "project_dir": "/home/me/.local/share/chaps/run/default",
  "enabled": true,
  "wait": {"ready": true, "waited_s": 41, "models": [...]}
}
```

`chaps ps --json` is `{"models": [...]}` with the same fields per model plus
`state`; `chaps stop --json` lists what it `stopped`, the groups it
`removed` and the volumes that went with them, `removed_volumes`. A failure is
`{"ok": false, "error": ..., "hint": ...}` like every other command's, a usage error included; see
[`--json` for scripts and tools](./commands.md#--json-for-scripts-and-tools).
docker's own progress, such as a pull, goes to stderr, so stdout is the one
document.

Two `chaps run` started at the same time into one group wait for each other
rather than writing over each other, and land on different ports; see
[One change at a time](./concepts.md#one-change-at-a-time). That holds for a
group neither of them has created yet, and for two runs of the same model,
which start it once and both report it.
