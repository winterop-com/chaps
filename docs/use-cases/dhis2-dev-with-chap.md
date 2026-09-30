# A DHIS2 you run yourself, with CHAP from chaps

For DHIS2 or Modeling App development: you run DHIS2 your own way on this
machine (from its source, or its own compose project), and chaps runs chap-core
and the models. `chaps dhis2 connect` wires the two together as it would for any
DHIS2.

```sh
chaps init mychap --models default
cd mychap
chaps up
```

Then record your DHIS2 and how it reaches chap-core, and connect:

```sh
# DHIS2 runs as a process on this machine (a dev server, Jetty, Tomcat):
chaps dhis2 use http://localhost:8080 --chap-url http://localhost:8700

# DHIS2 runs in a container of its own compose project:
chaps dhis2 use http://localhost:8080 --chap-url http://host.docker.internal:8700

chaps dhis2 connect
```

The first URL is where chaps reaches DHIS2; `--chap-url` is where DHIS2 reaches
chap-core, which is what the `chap` route points at:

- A DHIS2 **process** on this machine reaches chap-core at `localhost` and the
  API port. `dhis2 use` notes that `localhost` only works for a DHIS2 on this
  machine, which is the case here.
- A DHIS2 **container** has its own `localhost`; `host.docker.internal` is this
  machine. On Linux, give that container
  `extra_hosts: ["host.docker.internal:host-gateway"]` in its own compose file.

DHIS2 42 and later only proxy to origins its `dhis.conf` allows: add the
`--chap-url` origin to `route.remote_servers_allowed` there, and restart it.
`chaps dhis2 connect` names the setting if DHIS2 refuses the route.

It worked when `chaps dhis2 connect` finishes without `error:` and
`chaps dhis2 show` lists the route as healthy. `connect` sets only the route on
a DHIS2 chaps did not start; `chaps dhis2 apps` installs the Modeling and
Climate apps, and `chaps dhis2 analytics` generates the analytics tables. `chaps dhis2 use --clear` forgets
your DHIS2 again.

More: [A DHIS2 that runs elsewhere](../dhis2.md#a-dhis2-that-runs-elsewhere).

All shapes: [Use cases](../use-cases.md).
