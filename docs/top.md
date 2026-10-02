# Watching everything: chaps top

`chaps top` shows every chaps deployment on this machine as one tree that keeps
itself up to date: the deployments made with `chaps init` and the
[`chaps run` groups](./run.md) alike, each with its services under it -
chap-core and its worker and databases, DHIS2, OCS, the object store and the
models - and what each container is doing.

```text
 chaps top   3 deployments  7/9 running  at 14:02:41 (every 2s)
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│DEPLOYMENT / SERVICE               STATE               CPU     MEMORY              URL      │
│▾ mychap  init                     5/6 running                                  /home/me/mychap
│  ├─ chap                          running (healthy)   1.20%   612MiB / 15.6GiB   http://localhost:8700
│  ├─ postgres                      running (healthy)   0.40%   88MiB / 15.6GiB
│  ├─ redis                         running (healthy)   0.30%   9MiB / 15.6GiB
│  ├─ worker                        running (healthy)   0.10%   410MiB / 15.6GiB
│  ├─ dhis2                         running (starting)  96.0%   2.1GiB / 15.6GiB   http://localhost:8780
│  └─ chapkit-ewars-model           exited                                         http://localhost:5001
│▾ run/default  run                 1/1 running
│  └─ chapkit-ghr-model             running             0.50%   230MiB / 15.6GiB   http://localhost:5002
│▸ run/trial  run                   1/2 running
└────────────────────────────────────────────────────────────────────────────────────────────┘
 j/k move  enter fold  l logs  s stop model  r refresh  q quit
```

| Key | What it does |
| --- | --- |
| `j`/`k`, the arrows | Move. |
| `enter`, space | Fold or unfold the deployment under the cursor. |
| `l` | Show the last 200 lines of the service's log in a pane underneath, colour codes removed; `l` or `esc` closes it. The log is asked for on a thread of its own, so the pane says `asking docker ...` until it arrives and a slow daemon never holds up a key. |
| `s` | Stop the model under the cursor, after a `y`: the same as `chaps stop`, which takes the model out of its deployment - its overlay goes and `chaps up` no longer starts it; its data stays. The prompt says so. `chaps run` or `chaps models enable` brings it back. Only a model stops here; a whole deployment stops with `chaps -C DIR down`, which the footer names. |
| `r` | Look again now rather than at the next tick. |
| `q`, `esc` | Quit. |

`--interval SECONDS` (2 by default, at most 3600) sets how often docker is asked. Docker is
asked from a thread of its own, so a slow daemon never holds up a key.

## How it finds them

Every container chaps starts carries labels that say what it is (see
[Container labels](./concepts.md#container-labels)), so one
`docker ps -a --filter label=com.winterop.chaps.role` finds them all, on any
deployment, wherever its directory is. Compose's own labels say which
deployment and which service each one is, and the directory compose ran in;
from that directory `chaps top` reads the deployment's state for the URLs. A
deployment whose directory is gone still shows, without them.

The deployment the command runs in, and every `chaps run` group with a model
enabled, show even with nothing running: their enabled models are listed as
`not running`. A group with no model is left out. Containers
from before chaps labelled them are not found until the next `chaps up`
recreates them with their labels.

## In a script

Off a terminal - piped, or under `--json` - `chaps top` prints one snapshot of
the same tree and exits:

```sh
chaps top --json | jq '.deployments[] | {name, kind, services: [.services[] | {service, state, cpu}]}'
```

Each deployment has `project` (the compose project name), `name`, `kind`
(`init` or `run`), `group`, `dir` and `services`; each service has `service`,
`role`, `model`, `state`, `health`, `cpu`, `memory` and `url`. For the models
of one deployment with their registration state, [`chaps ps`](./run.md) and
`chaps status` say more.
