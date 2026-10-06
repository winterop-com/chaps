# Chap with climate data from OCS

chap-core with an Open Climate Service beside it, so Chap has climate data to
fetch from inside the same deployment.

```sh
varde init mychap --models default --with ocs,s3
cd mychap
varde up
varde status                 # chap-core, ocs and s3 lines, then the models
```

chap-core reaches OCS at `http://ocs:9000` on the compose network, and you
reach it at `http://localhost:8790`. `init` writes `ocs/climate-service.yaml`
(yours to edit, never rewritten) and puts the data source credentials into
`.env` commented out. ERA5-Land needs a Copernicus Climate Data Store and/or an
Earth Data Hub account; WorldPop and CHIRPS3 need none.

It worked when `varde status` shows chap-core, `ocs` and `s3` as `up` and the
models registered. To put climate data in and get it out per district as
the CSV Chap reads, see
[Ingesting a first dataset](./ocs-alone.md#ingesting-a-first-dataset) and the
sections after it; the same commands work here. The extent is Laos, like the
demo DHIS2, until you change it.

Next: [OCS](../components.md#ocs), and
[Data source credentials](../components.md#data-source-credentials) for which
dataset needs which account.

All shapes: [Use cases](../use-cases.md).
