# An OCS server on its own

Open Climate Service and no Chap at all: a climate data server for anything
that speaks STAC or openEO.

```sh
varde init climate --only ocs,s3
cd climate
varde up
varde status                 # ocs up, with its dataset count and data size
varde open ocs
```

You get OCS on `http://localhost:8790`, keeping its objects in the S3-compatible
store beside it. Nothing Chap-specific is created: no `compose.yml`, and no
chap-core release is looked up. It worked when `varde status` shows `ocs` as
`up` and `varde open ocs` opens its web interface.

Without the object store, OCS keeps its data on its own volume:

```sh
varde init climate --only ocs
```

## Ingesting a first dataset

CHIRPS3 rainfall is public and needs no account. Three days of it, over the
instance's extent:

```sh
curl -X POST http://localhost:8790/ingestions \
  -H 'Content-Type: application/json' \
  -d '{"dataset_id": "chirps3_precipitation_daily", "start": "2024-01-01", "end": "2024-01-03", "publish": true}'
curl http://localhost:8790/stac/collections/chirps3_precipitation_daily
```

It worked when the first command answers with `"status":"completed"` and the
second returns the collection; `varde status` then counts one dataset.

The extent is Laos until you change it, the country of the DHIS2 demo database
varde seeds, so data ingested here covers the provinces that DHIS2 holds case
data for. Change it in `ocs/climate-service.yaml`, or at `init` with
`--ocs-name`, `--ocs-country` and `--ocs-bbox`; `curl
http://localhost:8790/extent` shows the one in use.

## Which datasets there are

```sh
curl http://localhost:8790/data-sources
```

lists every data source OCS knows, and `"ingestable": true` marks the ones
`/ingestions` takes. The ones Chap models use:

| Dataset id | Period | Account |
| --- | --- | --- |
| `chirps3_precipitation_monthly`, `chirps3_precipitation_daily` | month, day | none |
| `era5land_temperature_monthly`, `era5land_precipitation_monthly` | month | Copernicus Climate Data Store |
| `era5land_temperature_daily`, `era5land_precipitation_daily` | day | Earth Data Hub |
| `worldpop_population_global2_100m` | year | none |

[Data source credentials](../components.md#data-source-credentials) says where
the two accounts go.

## A range worth forecasting with

`start` and `end` are written in the dataset's own period: `YYYY-MM-DD` for a
daily one, `YYYY-MM` for a monthly one and `YYYY` for a yearly one. Six years
of monthly rainfall, the years the demo DHIS2 has cases for:

```sh
curl -X POST http://localhost:8790/ingestions \
  -H 'Content-Type: application/json' \
  -d '{"dataset_id": "chirps3_precipitation_monthly", "start": "2019-01", "end": "2024-12"}'
```

That took about three minutes for Laos, and the request waits for it. With
`-H 'Prefer: respond-async'` it answers `202` at once instead, with a
`Location` of `/ingestions/jobs/ID` to poll. Leaving out `end` ingests up to
the current period, and sending the same request again answers at once with
the earlier ingestion rather than downloading it twice.
`curl 'http://localhost:8790/datasets?f=json'` lists what has been ingested.

## Getting it out

Values per district or province, as the CSV Chap reads: OCS averages the
dataset over each area in a GeoJSON `FeatureCollection` whose feature ids
become the `location` column. A DHIS2 hands out its org units in that shape,
so with the demo DHIS2 beside OCS (`varde components enable dhis2`, then
`varde up`; or `--only ocs,s3,dhis2` at `init`, option 10 on the
[AI page](../ai.md#10-climate-data-and-dhis2-no-chap)):

```sh
curl -u admin:district -o provinces.geojson \
  'http://localhost:8780/api/organisationUnits.geojson?level=2'
python3 - > request.json <<'PY'
import json
areas = json.load(open("provinces.geojson"))
print(json.dumps({"process": {"process_graph": {"agg": {
    "process_id": "aggregate_to_chap_csv",
    "arguments": {"dataset_id": "chirps3_precipitation_monthly",
                  "temporal_extent": ["2024-01-01", "2024-12-31"],
                  "geometries": areas, "method": "mean", "period_type": "month"},
    "result": True}}}}))
PY
curl -X POST http://localhost:8790/result \
  -H 'Content-Type: application/json' -d @request.json -o rainfall.csv
```

It worked when `rainfall.csv` starts with `time_period,location,precip` and has
one row per province and month (216 for the 18 provinces over 2024). The values
are what the dataset holds: CHIRPS3 is a rate in mm per day.

The whole grid over a bounding box, as NetCDF, for anything that reads it
(`xarray`, QGIS, Panoply):

```sh
curl -X POST http://localhost:8790/result -H 'Content-Type: application/json' -o rainfall.nc -d '{
  "process": {"process_graph": {
    "load": {"process_id": "load_collection", "arguments": {
      "id": "chirps3_precipitation_monthly",
      "temporal_extent": ["2024-01-01", "2024-12-31"],
      "spatial_extent": {"west": 100.0, "south": 13.9, "east": 107.7, "north": 22.5}}},
    "save": {"process_id": "save_result",
      "arguments": {"data": {"from_node": "load"}, "format": "NetCDF"}, "result": true}}}}'
```

`GET /datasets/ID/download` answers `409` for data OCS keeps in its default
storage; `/result` is the way out. Both calls above still work on a read-only
instance, which refuses ingestion but not reading.

## As a server

Two settings matter once other people use it:

```sh
varde components enable ocs --read-only                          # refuse writes over HTTP
varde components enable ocs --base-url https://ocs.example.org   # behind a proxy
```

Both have `init` spellings too, `--ocs-read-only` and `--ocs-base-url`. A
read-only instance refuses every ingestion over HTTP, so ingest first.

Next: [Standalone OCS](../components.md#standalone-ocs),
[Data source credentials](../components.md#data-source-credentials),
[Read-only instances](../components.md#read-only-instances),
[Behind a reverse proxy](../components.md#behind-a-reverse-proxy).

All shapes: [Use cases](../use-cases.md).
