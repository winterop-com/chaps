# Chap for a DHIS2 that runs elsewhere

chap-core and its models on a server, used by an existing DHIS2 instance.

```sh
varde init mychap --models default --api-token
cd mychap
varde up                     # returns when the containers start
varde status                 # run it again until the chap-core line says up
varde dhis2 use https://dhis2.example.org --chap-url https://chap.example.org
varde dhis2 connect
```

Replace the two URLs in `varde dhis2 use`:

- `https://dhis2.example.org` is the URL of the DHIS2, as this machine
  reaches it.
- `https://chap.example.org` is the address of chap-core, as the DHIS2 server
  reaches it. The route points there.

`--api-token` protects chap-core's API from the start. `dhis2 use` records the
external DHIS2 and the address it reaches chap-core at. `connect` then creates
the `chap` route on that DHIS2, or points an existing `chap` route at this
chap-core. It puts chap-core's token in the route, so the Modeling App reaches
chap-core and never holds the token itself.

The first `varde up` on a machine pulls the images: about 6 GB to download and
about 20 GB on disk (see [Quickstart](../quickstart.md)). When the images are
on the machine, `up` returns in seconds, and chap-core answers some seconds
later.

## The credentials

varde has no default login for a DHIS2 it did not deploy. So the first
`varde dhis2 use` ends with this warning, and `varde dhis2 connect` stops with
the same text as an `error:`:

```text
warning: varde has no credentials for this DHIS2, and did not deploy it, so there is no default to try; set `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD`, or `DHIS2_API_TOKEN`, in `.env`
```

`.env` has these three lines as comments at its end. To give varde a login:

1. Remove the `#` from `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD`, and
   set the values of a DHIS2 superuser. Or set `DHIS2_API_TOKEN` to a personal
   access token of that user.
2. Run `varde dhis2 use` again. Its `login` line ends with `(accepted)`.
3. Run `varde dhis2 connect`.

See [The credentials](../dhis2.md#the-credentials-and-where-they-come-from).

## What `connect` changes

On a DHIS2 that varde does not run, `connect` changes the route and nothing
else. The admin of that DHIS2 decides about the other two steps:

- `varde dhis2 apps` installs the Modeling App and the Climate App.
- `varde dhis2 analytics` generates the analytics tables. On a large instance
  this can take hours.

For a DHIS2 2.42 that has a `chap` route already and none of the apps, the
first `connect` printed this:

```text
external DHIS2 2.42.6 at https://dhis2.example.org, as `admin`
repointed the `chap` route at https://chap.example.org/**; chap-core answered through it
skipped: installing apps on a DHIS2 varde does not run (the Modeling App is not installed, the Climate App is not installed); `varde dhis2 apps` installs them if its admin agrees
skipped: generating analytics tables on a DHIS2 varde does not run; `varde dhis2 analytics` starts a run if its admin agrees
created configured models in chap-core for chapkit_ewars_model (monthly_climate, monthly_population_only, monthly_region_seasonal)
the Modeling App can reach Chap after the admin of this DHIS2 installs the Modeling App and the Climate App; `varde dhis2 show` says when they are there
```

On a DHIS2 with no `chap` route, the second line starts with `created`. Only
the first `connect` prints the `created configured models` line. It makes the
configured models that the Modeling App lists.

It worked when:

- `varde dhis2 connect` says `chap-core answered through it`.
- `varde dhis2 show` lists the route as `(healthy)`.
- When the two apps are installed, the last line of `varde dhis2 show` and
  `varde dhis2 connect` is
  ``the Modeling App can reach Chap; open DHIS2 with `varde open dhis2` ``.
- The **Models** page of the Modeling App lists the models.

Until the admin installs the apps, `varde dhis2 show` names each missing app
on a `missing:` line.

If `connect` warns `nothing answered through it`, read the rest of the
warning:

- If it says `chap-core answers at`, chap-core is up, but the DHIS2 server may
  not reach the `--chap-url` address. Set the correct address with
  `varde dhis2 use --chap-url URL`. Then run `varde dhis2 connect` again.
- If it says `` run `varde status` ``, chap-core did not answer on this
  machine. Run `varde status`.

See [The route is there but nothing answers through
it](../troubleshooting.md#the-route-is-there-but-nothing-answers-through-it).

Until `connect` gets an answer from chap-core through the route, `varde up`
ends with this line:

```text
varde has not connected this DHIS2 to Chap; run `varde dhis2 connect` once DHIS2 answers
```

On a DHIS2 that varde does not run, the route is all that `connect` changes.
So when chap-core answers through the route, `connect` records the connect in
`.varde/components.yaml`, and the line stops. The apps do not have to be
installed for this.

## The address of chap-core

`--chap-url` is the address at which the DHIS2 server reaches this machine:

- A DHIS2 on another server needs the name or IP address of this machine.
- `localhost` works only for a DHIS2 that runs directly on this machine.
  `varde dhis2 use` warns about it.
- A DHIS2 in Docker on this machine uses `http://host.docker.internal:8700`.

varde serves chap-core over http on port 8700. An `https://` address needs a
reverse proxy in front of it. If you use `http://SERVER:8700`, the DHIS2 admin
must add that origin, with no path, to `route.remote_servers_allowed` in the
`dhis.conf` of the DHIS2 server (DHIS2 2.42 and later).

## Try it on one machine

A second varde deployment can be the external DHIS2. It uses port 8790, so it
does not conflict with chap-core. Start it before `varde dhis2 use`. Run these
commands in the directory that holds `mychap`, not in `mychap`:

```sh
varde init ext-dhis2 --only dhis2 --dhis2-port 8790
cd ext-dhis2
varde up
varde status                 # run it again until the dhis2 line says up
```

DHIS2 answers after about a minute on a fast machine, and after some minutes
on a slow one.

In `mychap`, use `varde dhis2 use http://localhost:8790 --chap-url
http://host.docker.internal:8700`. In [The credentials](#the-credentials), use
the demo login `admin` / `district`. The demo database has a `chap` route
already, so `connect` says `repointed`.

This DHIS2 is yours, so you are its admin. In `mychap`, `varde dhis2 apps`
installs the two apps in about ten seconds.

To remove the two deployments and their data, run `varde down --volumes --yes`
in `mychap` and in `ext-dhis2`.

Next: [A DHIS2 that runs elsewhere](../dhis2.md#a-dhis2-that-runs-elsewhere),
[Authentication](../auth.md).

All shapes: [Use cases](../use-cases.md).
