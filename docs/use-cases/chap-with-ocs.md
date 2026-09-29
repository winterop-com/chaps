# CHAP with climate data from OCS

chap-core with an Open Climate Service beside it, so CHAP has climate data to
fetch from inside the same deployment.

```sh
chaps init mychap --models default --with ocs,s3
cd mychap
chaps up
chaps status                 # chap-core, ocs and s3 lines, then the models
```

chap-core reaches OCS at `http://ocs:9000` on the compose network, and you
reach it at `http://localhost:9000`. `init` writes `ocs/climate-service.yaml`
(yours to edit, never rewritten) and puts the data source credentials into
`.env` commented out. ERA5-Land needs a Copernicus Climate Data Store and/or an
Earth Data Hub account; WorldPop and CHIRPS3 need none.

Next: [OCS](../components.md#ocs), and
[Data source credentials](../components.md#data-source-credentials) for which
dataset needs which account.

All shapes: [Use cases](../use-cases.md).
