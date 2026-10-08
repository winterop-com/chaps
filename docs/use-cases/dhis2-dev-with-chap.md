# A DHIS2 you run yourself, with Chap from varde

For DHIS2 or Modeling App development: you run DHIS2 your own way on this
machine (from its source, or its own compose project), and varde runs chap-core
and the models. `varde dhis2 connect` points the `chap` route of your DHIS2 at
this chap-core.

```sh
varde init mychap --models default
cd mychap
varde up                     # returns when the containers start
varde status                 # run it again until the chap-core line says up
```

The first `varde up` on a machine pulls the images: about 6 GB to download and
about 20 GB on disk (see [Quickstart](../quickstart.md)). When the images are
on the machine, `up` returns in seconds, and chap-core answers some seconds
later.

## The login

varde has no default login for a DHIS2 it did not start. `.env` has the three
lines as comments at its end. Before you record your DHIS2:

1. Remove the `#` from `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD`.
2. Set the login of a DHIS2 superuser. For a local dev DHIS2, this is often
   `admin` and `district`.

Or set `DHIS2_API_TOKEN` to a personal access token of that user. Without a
login, `varde dhis2 use` ends with this warning, and `varde dhis2 connect`
stops with the same text as an `error:`:

```text
warning: varde has no credentials for this DHIS2, and did not deploy it, so there is no default to try; set `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD`, or `DHIS2_API_TOKEN`, in `.env`
```

## Record your DHIS2 and connect

Run one of the two `use` lines, then `connect`. If your DHIS2 does not use
port 8080, change `8080` to its port.

```sh
# DHIS2 runs as a process on this machine (a dev server, Jetty, Tomcat):
varde dhis2 use http://localhost:8080 --chap-url http://localhost:8700

# DHIS2 runs in a container of its own compose project:
varde dhis2 use http://localhost:8080 --chap-url http://host.docker.internal:8700

varde dhis2 connect
```

The first URL is where varde reaches DHIS2; `--chap-url` is where DHIS2 reaches
chap-core, which is what the `chap` route points at:

- A DHIS2 **process** on this machine reaches chap-core at `localhost` and the
  API port. `dhis2 use` gives a warning about `localhost`. Here DHIS2 runs
  directly on this machine, so the address is correct:

  ```text
  warning: http://localhost:8700 is right for a DHIS2 running directly on this machine; one running in Docker reaches chap-core at `http://host.docker.internal:8700` instead
  ```

- A DHIS2 **container** has its own `localhost`; `host.docker.internal` is this
  machine. On Linux, give that container
  `extra_hosts: ["host.docker.internal:host-gateway"]` in its own compose file.

For a DHIS2 2.42 in a container, with the `chap` route of the demo database and
no apps, `connect` printed this:

```text
external DHIS2 2.42.6 at http://localhost:8080, as `admin`
repointed the `chap` route at http://host.docker.internal:8700/**; chap-core answered through it
skipped: installing apps on a DHIS2 varde does not run (the Modeling App is not installed, the Climate App is not installed); `varde dhis2 apps` installs them if its admin agrees
skipped: generating analytics tables on a DHIS2 varde does not run; `varde dhis2 analytics` starts a run if its admin agrees
the Modeling App can reach Chap once it is installed; open DHIS2 with `varde open dhis2`
```

On a DHIS2 with no `chap` route, the second line starts with `created`.

## The allowlist in `dhis.conf`

DHIS2 2.42 and later only proxy to origins that its `dhis.conf` allows. The
DHIS2 default is `https://*`, which refuses both `http://` addresses above. Add
the `--chap-url` origin, with no path, to `route.remote_servers_allowed`:

```ini
route.remote_servers_allowed = https://*,http://host.docker.internal:8700
```

Then restart DHIS2. If DHIS2 refuses the route, `varde dhis2 connect` names
the setting. For the process address, it printed:

```text
error: DHIS2 refused the route: version 42 and later only allow the origins `route.remote_servers_allowed` lists, and http://localhost:8700 has to be one of them; add it, with no path (DHIS2 will not start with one), to that line in the `dhis.conf` of the DHIS2 server and restart DHIS2 there
```

DHIS2 checks the allowlist when varde writes the route. If the route already
points at this chap-core, varde writes nothing. Then a refusal shows only as
a warning, and chap-core can be up:

```text
warning: the `chap` route is in place but nothing answered through it: HTTP 503 Service Unavailable; run `varde status` to see whether chap-core is up
```

If `varde status` says chap-core is up, check the allowlist line.

## The apps and the analytics tables

`connect` sets only the route on a DHIS2 varde did not start. On your own dev
DHIS2, install the Modeling and Climate apps, and generate the analytics
tables:

```sh
varde dhis2 apps
varde dhis2 analytics
varde dhis2 connect
```

`apps` needs the network: DHIS2 downloads the apps from the App Hub. On the
demo database, `analytics` took 30 seconds; a larger database can take hours.
The second `connect` records the connect in `.varde/components.yaml`. Until
then, `varde up` ends with this line:

```text
varde has not connected this DHIS2 to Chap; run `varde dhis2 connect` once DHIS2 answers
```

It worked when `varde dhis2 connect` says `chap-core answered through it`, and
`varde dhis2 show` lists the route as `(healthy)` and both apps:

```text
external DHIS2 2.42.6 at http://localhost:8080, as `admin`
route      http://host.docker.internal:8700/** (healthy)
target     http://host.docker.internal:8700/**
analytics  2026-10-07T23:14:20.016 (a run finished on this deployment)
apps       Modeling App 7.2.0, DHIS2 Climate App 1.16.2
the Modeling App can reach Chap; open DHIS2 with `varde open dhis2`
```

`varde dhis2 use --clear` forgets your DHIS2 again. The `chap` route stays in
DHIS2.

To try this with no DHIS2 of your own, start one as a second varde deployment:
see [Try it on one machine](chap-for-external-dhis2.md#try-it-on-one-machine).

More: [A DHIS2 that runs elsewhere](../dhis2.md#a-dhis2-that-runs-elsewhere).

All shapes: [Use cases](../use-cases.md).
