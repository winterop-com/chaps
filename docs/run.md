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
stop it with `chaps stop chapkit_ewars_model`; `chaps -C /home/me/.local/share/chaps/run/default logs chapkit-ewars-model` shows its log
```

The command returns once the model answers its own `/health`, so the URL works
when the line is printed. A first run pulls the image, which takes a while for
the R-INLA models; `--timeout SECONDS` (300 by default) is how long it waits
before it fails naming the state the model was in, and `--no-wait` returns as
soon as the container started.

## What the model can be

| `MODEL` | What `run` does |
| --- | --- |
| A marketplace id or service id: `chapkit_ewars_model` | Enables it at the catalogue's stable version. |
| A GitHub repository the marketplace lists | The same: the catalogue's reviewed pin, not the newest build of the branch. |
| Any other GitHub repository URL | Adds it the way [`chaps models add`](./models.md#models-outside-the-marketplace) does: the newest published `sha-` build of the default branch. Needs the network. |
| A ghcr image with a tag or digest | Pins exactly that image. A tag the marketplace publishes for one of its models enables that marketplace version instead. |
| A local image, `name:tag` with no registry | Runs the image from the local store, never pulled. |

A model that is already enabled is started as it is: a second `chaps run` of
the same id does not move its version or its port. `--id` names an added model
(`--id auto` picks a free one), exactly as on `models add`.

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
chaps run chapkit_ewars_model                       # group default
chaps run auto_arima_chapkit --group trial
chaps run chapkit_ghr_model --group trial
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
every model in every group, and `--group NAME --all` every model in one. The
data volume stays, as with `models disable`, and so does the definition of an
added model, so `chaps run ID` starts it again without asking GitHub.
`--purge` takes the volume too.

The deployment behind a group is an ordinary chaps deployment, so every other
command works on it with `-C`: `chaps -C ~/.local/share/chaps/run/default logs
chapkit-ewars-model`, `chaps -C ... status`, `chaps -C ... down --volumes`.
Removing the group's directory after a `down --volumes` removes the group.

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
`state`; `chaps stop --json` lists what it `stopped`. A failure is
`{"ok": false, "error": ..., "hint": ...}` like every other command's; see
[`--json` for scripts and tools](./commands.md#--json-for-scripts-and-tools).
Two `chaps run` started at the same time into one group wait for each other
rather than writing over each other, and land on different ports; see
[One change at a time](./concepts.md#one-change-at-a-time).
