# Combinations

Components and models combine freely: `--only` names exactly the components
you want (and chap-core only if you list it), `--with` adds components to
chap-core, and `--models` names the model services either way.

Each example below makes a deployment with its own name. Run each `varde init`
from the directory where you keep your deployments, not from inside another
deployment. All three use the default host ports, so only one of them can be
up at a time. `varde init` warns about this, and `varde up` offers to stop the
other deployment. To run them side by side, see
[Several deployments on one machine](./several-deployments.md).

## Climate data and a DHIS2, no Chap

OCS with its object store beside a DHIS2, for work on climate data in DHIS2
without Chap in the picture:

```sh
varde init lab --only ocs,s3,dhis2
cd lab
varde up                     # DHIS2's first start takes several minutes
varde status                 # run it again until every line says up
varde open ocs
varde open dhis2
```

The first `varde up` pulls the images that are not on the machine yet: OCS
(about 3 GB on disk), DHIS2 (about 1.2 GB) and PostGIS (about 0.8 GB). With the
images already present, `varde up` took 14 seconds and DHIS2 answered about 25
seconds later. While DHIS2 starts, `varde status` ends with
`` 1 of 3 components is still starting; run `varde status` again in a moment ``.
Then it shows:

```text
ocs     up   http://localhost:8790   0 datasets   32.0 KB data
s3      up   internal
dhis2   up   http://localhost:8780

all 3 components are up
```

To wait in one command instead, run `varde up --wait --timeout 1200`. It first
says `waiting up to 1200s for ocs and dhis2 to answer`. It returns only when
OCS answers its health check and DHIS2 answers `/api/ping`, and ends with:

```text
started dhis2, dhis2-db, ocs, s3
ready in 17s
  ocs    up  http://localhost:8790
  dhis2  up  http://localhost:8780
```

The 17 seconds count from the end of the compose step. A first start of DHIS2
can take longer than the default 300 seconds, so keep `--timeout 1200`.

It worked when `varde status` shows `ocs`, `s3` and `dhis2` as `up`. OCS is on
`http://localhost:8790` and DHIS2 on `http://localhost:8780` (login `admin` /
`district`). Inside the deployment, DHIS2 reaches OCS at `http://ocs:9000`.
To put climate data into OCS and get it out per DHIS2 org unit, see
[Getting it out](./ocs-alone.md#getting-it-out).

`varde dhis2 connect` refuses, because this deployment has no chap-core:

```text
error: chap-core is not a component of this deployment, so the route would point at a service that is not there; run `varde components enable chap-core` first
```

To add Chap later, do these steps in the deployment directory:

```sh
varde components enable chap-core
varde up
varde status                 # run it again until the chap-core line says up
varde dhis2 connect
```

For about 10 seconds after `varde up`, the chap-core line says `starting`.
`varde dhis2 connect` took about 30 seconds, most of it for analytics.

chap-core comes without models: `varde status` says
`` no models enabled; run `varde models enable ID` to add one ``. Its image tag
is `latest`, not a release. If the chap-core images are not on the machine, the
first `varde up` pulls about 15 GB. See
[Growing a deployment](./growing.md) for the tag and how to pin a release.

## A model beside OCS, no Chap

```sh
varde init ocs-model --only ocs --models chapkit_ewars_model
cd ocs-model
varde up --wait
varde status
```

The first `varde up` pulls the model image (about 7 GB on disk) and the OCS
image (about 3 GB) if they are not on the machine. `varde up --wait` returns
once the model answers on its own host port and OCS answers its health check.
Then
`varde status` shows:

```text
ocs   up   http://localhost:8790   0 datasets   32.0 KB data

MODEL                STATE  REACH      LAST PING
chapkit-ewars-model  up     port 5001  -

1 model up, answering on its own host port
ocs is up
```

If the `ocs` line says `starting`, run `varde status` again in a moment. The
model answers on `http://localhost:5001`, as in
[A chapkit model service on its own](./model-alone.md), and OCS on
`http://localhost:8790`.

## Everything

```sh
varde init full --models default --with ocs,s3,dhis2
cd full
varde up --wait --timeout 1200
varde status
varde dhis2 connect          # the route to chap-core, the apps, analytics
```

chap-core, its models, OCS with the object store and a DHIS2, all in one
deployment. The images take about 28 GB on disk, so a first `varde up` that
pulls them all can take a long time. The running deployment used about 5 GB of
memory; give Docker 8 GB or more.

`varde up --wait` returns only when chap-core, OCS, DHIS2 and the model answer.
DHIS2 answers when `/api/ping` returns 200. A first start of DHIS2 can take
longer than the default 300 seconds, so the command sets `--timeout 1200`.
With the images already present, the command took 32 seconds and ended with:

```text
ready in 16s
  chap-core            up          http://localhost:8700
  chapkit-ewars-model  registered  http://localhost:8700/v2/services/chapkit-ewars-model/run/
  ocs                  up          http://localhost:8790
  dhis2                up          http://localhost:8780
created configured models in chap-core for chapkit_ewars_model (monthly_climate, monthly_population_only, monthly_region_seasonal)
```

Then `varde status` shows:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off
ocs         up   http://localhost:8790   0 datasets   32.0 KB data
s3          up   internal
dhis2       up   http://localhost:8780

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  11s ago

1 model registered
varde has not connected this DHIS2 to Chap; run `varde dhis2 connect`
```

If you use `varde up` without `--wait`, run `varde status` again until every
line says `up`. The model line then says `registered, not configured`, because
only `up --wait` and `varde dhis2 connect` make the configured models.

`varde dhis2 connect` took about 35 seconds, most of it for analytics. It
worked when `varde dhis2 connect` ends with
`` the Modeling App can reach Chap; open DHIS2 with `varde open dhis2` ``.
Follow [Chap with a local DHIS2](./chap-with-local-dhis2.md) for the DHIS2
side, and [Chap with climate data from OCS](./chap-with-ocs.md) for the OCS
side.

## Settings any of these take

`--dhis2-tag` (see [Picking the version](./dhis2-alone.md#picking-the-version))
and `--dhis2-seed` (see [Picking the data](./dhis2-alone.md#picking-the-data))
for DHIS2, `--ocs-read-only`
and `--ocs-base-url` for OCS, and `--api-port`, `--ocs-port`, `--dhis2-port`
for ports other than the defaults (see
[Several deployments on one machine](./several-deployments.md)).

All shapes: [Use cases](../use-cases.md).
