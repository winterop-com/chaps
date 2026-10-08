# Chap with climate data from OCS

chap-core with an Open Climate Service beside it, so Chap has climate data to
fetch from inside the same deployment.

```sh
varde init mychap --models default --with ocs,s3
cd mychap
varde up
varde status                 # chap-core, ocs and s3 lines, then the models
```

The first `varde up` pulls the images that are not on the machine yet: the
chap-core images, the model image and the OCS image. The OCS image alone is
about 3 GB on disk. With the chap-core and model images already present, the
first `varde up` took six minutes. It ends with one line:

```text
started chap, chapkit-ewars-model, ocs, postgres, redis, s3, worker
```

`varde status` then shows this:

```text
chap-core   up   http://localhost:8700   2.4.0   auth: off
ocs         up   http://localhost:8790   0 datasets   32.0 KB data
s3          up   internal

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  3s ago

1 model registered
```

chap-core reaches OCS at `http://ocs:9000` on the compose network, and you
reach it at `http://localhost:8790`. varde does not configure chap-core to
fetch from OCS. chap-core has no setting for it: the two run side by side, and
you connect them. See [OCS](../components.md#ocs).

`init` writes `ocs/climate-service.yaml`, which is yours to edit and is never
rewritten. It also puts the data source credentials into `.env`, commented
out. ERA5-Land needs one or both of a Copernicus Climate Data Store account
and an Earth Data Hub account. WorldPop and CHIRPS3 need no account.

It worked when `varde status` shows chap-core, `ocs` and `s3` as `up` and the
models registered.

## Climate data in and out

To put climate data in and get it out, see
[Ingesting a first dataset](./ocs-alone.md#ingesting-a-first-dataset) and the
sections after it. The same commands work here. Three days of CHIRPS3 took
about 40 seconds, and one year of monthly CHIRPS3 about one minute.

The per-district CSV in
[Getting it out](./ocs-alone.md#getting-it-out) reads the districts from a
DHIS2 on port 8780. This deployment has no DHIS2. Do one of these:

- Give `geometries` a GeoJSON `FeatureCollection` of your own districts. The
  feature ids become the `location` column.
- Add the demo DHIS2 with `varde components enable dhis2`, then `varde up`.
  See [Growing a deployment](./growing.md).

The extent is Laos, like the demo DHIS2, until you change it.
`varde doctor` warns about this on its `components` line, and tells you how
to change it. See
[`ocs/climate-service.yaml`](../components.md#ocsclimate-serviceyaml).

Next: [OCS](../components.md#ocs), and
[Data source credentials](../components.md#data-source-credentials) for which
dataset needs which account.

All shapes: [Use cases](../use-cases.md).
