# Several deployments on one machine

Each `chaps init` directory is a deployment of its own, with its own
containers, data and ports. Any number of them can live on one machine; the
only thing they share is the host ports, and by default every deployment asks
for the same ones: chap-core on 8700, DHIS2 on 8780 and OCS on 8790 (models
from 5001 up). Two ways to live with that.

## Taking turns

Keep the defaults and run one deployment at a time: a DHIS2 2.42 and a 2.43
side by side, say, to compare them.

```sh
chaps init d242 --only dhis2
chaps init d243 --only dhis2 --dhis2-tag 2.43   # warns: 8780 is also used by d242
cd d242 && chaps up
cd ../d243 && chaps up
```

`init` warns when a port is already used by another deployment it can see (one
in a directory beside the new one, or one Docker has started before), and names
a free port. Nothing is refused: two deployments on one port is a fine way to
take turns.

`chaps up` checks every port before it starts anything. When the port belongs
to another chaps deployment that is up, it names it, and at a terminal asks:

```text
d242 is using these ports. Stop it and start this deployment instead? Its data is kept. [y/N]
```

`y` runs `chaps down` on the other deployment and starts this one. Nothing is
deleted: `chaps up` in the other directory brings it back, database and all. In
a script there is no one to ask, so `chaps up` refuses and names both ways out;
`chaps up --replace` gives the answer in advance:

```sh
chaps up --replace
```

A port some other program holds (not a chaps deployment) is only reported,
never taken: `chaps up` names the port and the setting that moves this
deployment off it.

## Running them at the same time

Give each deployment its own ports. At `init`:

```sh
chaps init a --with ocs,dhis2
chaps init b --with ocs,dhis2 --api-port 8701 --ocs-port 8791 --dhis2-port 8781 --port-base 5101
```

`--port-base` moves the range model services are published from, for
deployments whose models have host ports (those without chap-core, or exposed
with `chaps models expose`).

On a deployment that already exists:

| What | How |
| --- | --- |
| chap-core's API | `CHAP_API_PORT=8701` in `.env`, then `chaps up` |
| OCS | `chaps components enable ocs --port 8791`, then `chaps up` |
| DHIS2 | `chaps components enable dhis2 --port 8781`, then `chaps up` |
| a model | `chaps models expose ID --port 5101`, or `chaps models unexpose ID` |

Inside each deployment the services keep their container ports
(`http://chap:8000`, `http://ocs:9000`), so moving a host port changes nothing
between them. Memory is the other limit: every DHIS2 wants 4 to 5 GB.

## Knowing which is which

Every command works on the deployment in the current directory, or the one
`-C DIR` names:

```sh
chaps -C d242 status
chaps -C d243 down
```

`chaps status` in each directory says what that one is running and where.

More: [Ports](../ports.md), [The preflight](../ports.md#the-preflight),
[A host port is already in use](../troubleshooting.md#a-host-port-is-already-in-use).

All shapes: [Use cases](../use-cases.md).
