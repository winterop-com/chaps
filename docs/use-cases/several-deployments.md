# Several deployments on one machine

Each `varde init` directory is a deployment of its own, with its own
containers, data and ports. Any number of them can live on one machine. They
share only the host ports, and by default every deployment asks for the same
ones: chap-core on 8700, DHIS2 on 8780 and OCS on 8790 (models from 5001 up).
There are two ways to work with that.

## One deployment at a time

Keep the default ports and run one deployment at a time. For example, compare a
DHIS2 2.42 with a 2.43:

```sh
varde init d242 --only dhis2
varde init d243 --only dhis2 --dhis2-tag 2.43   # warns: 8780 is also used by d242
cd d242 && varde up
cd ../d243 && varde up
```

The second `init` gives two warnings:

```text
warning: port 8780 is also used by d242 (/home/me/d242), which is not running; run only one of them at a time, or run `varde components enable dhis2 --port 8781`
warning: varde knows no DHIS2 demo dump for 2.43, so `dhis2_db` starts empty; name one with `seed:` in `.varde/components.yaml` (a URL or a path) and run `varde sync`
```

`init` warns when another deployment that it can see uses the same port. It
sees a deployment in a directory beside the new one, and a deployment that
Docker started before. The warning names a free port. `init` refuses nothing:
two deployments on one port can take turns. The second warning is about the
data: varde has a demo database for 2.42 only, so d243 starts with an empty
DHIS2.

`varde up` returns when the containers are started. DHIS2 needs more time to
answer: `varde status` shows `starting` until it does, then `dhis2 is up`.

`varde up` checks every port before it starts anything. When the port belongs
to another varde deployment, `varde up` names it. At a terminal, it then asks:

```text
d242 is using these ports. Stop it and start this deployment instead? Its data is kept. [y/N]
```

If you answer `y`, varde runs `varde down` on the other deployment, then starts
this one:

```text
stopping d242 (/home/me/d242) to free its ports; `varde -C /home/me/d242 up` starts it again
...
started dhis2, dhis2-db
```

varde deletes nothing. `varde up` in the other directory starts it again, with
its database. In a script, there is no one to answer. `varde up` then refuses
and names each way out:

```text
error: 1 host port this deployment needs is already in use; nothing was started
  port 8780 (needed by dhis2) is in use, and d242 (/home/me/d242) publishes it too; stop it with `varde -C /home/me/d242 down`, or move this one: run `varde components enable dhis2 --port 8781`
  or run `varde up --no-preflight` to hand the conflict to Docker
  or run `varde up --replace` to stop d242 first
```

`varde up --replace` gives the answer `y` before the question:

```sh
varde up --replace
```

Only a running deployment is named, and only a running one is stopped by
`--replace`. A stopped deployment that publishes the same port holds nothing,
so `varde up` starts this one. If the port is in use for another reason, the
line is the plain `port 8780 (needed by dhis2) is already in use on this
machine` line.

When a program that is not a varde deployment holds the port, varde only
reports it and does not stop that program. `varde up` names the port and the
setting that moves this deployment to a different port.

## All deployments at the same time

Give each deployment its own ports. At `init`:

```sh
varde init a --with ocs,dhis2
varde init b --with ocs,dhis2 --api-port 8701 --ocs-port 8791 --dhis2-port 8781 --port-base 5101
```

Both deployments also get chap-core and the default model,
`chapkit_ewars_model`. `init b` prints the ports it uses:

```text
chap-core: v2.4.0, API on http://localhost:8701
OCS: http://localhost:8791
DHIS2: http://localhost:8781
```

`--port-base` moves the range that model services get host ports from. It has
an effect only on models with host ports: models in a deployment without
chap-core, and models that `varde models expose` publishes. In `b` above, the
model is reached through chap-core, so it has no host port.

On a deployment that already exists:

| What | How |
| --- | --- |
| chap-core's API | `CHAP_API_PORT=8701` in `.env`, then `varde up` |
| OCS | `varde components enable ocs --port 8791`, then `varde up` |
| DHIS2 | `varde components enable dhis2 --port 8781`, then `varde up` |
| a model | `varde models expose ID --port 5101`, or `varde models unexpose ID` |

You do not have to find a free port. When a port is in use, the warnings of
`varde init` and the refusal of `varde up` name the next free port in the line
to copy.

Inside each deployment, the services keep their container ports
(`http://chap:8000`, `http://ocs:9000`). Thus a change to a host port does not
change how the services reach each other. Memory is the other limit: each
DHIS2 can use 4 to 5 GB.

## Which deployment a command uses

Each command works on the deployment in the current directory, or on the one
that `-C DIR` names:

```sh
varde -C d242 status
varde -C d243 down
```

`varde status` in each directory shows what that deployment runs and where.

More: [Ports](../ports.md), [The preflight](../ports.md#the-preflight),
[A host port is already in use](../troubleshooting.md#a-host-port-is-already-in-use).

All shapes: [Use cases](../use-cases.md).
