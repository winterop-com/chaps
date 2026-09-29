# An OCS server on its own

Open Climate Service and no CHAP at all: a climate data server for anything
that speaks STAC or openEO.

```sh
chaps init climate --only ocs,s3
cd climate
chaps up
chaps status                 # ocs up, with its dataset count and data size
chaps open ocs
```

You get OCS on `http://localhost:8790`, keeping its objects in the S3-compatible
store beside it. Nothing CHAP-specific is created: no `compose.yml`, and no
chap-core release is looked up. It worked when `chaps status` shows `ocs` as
`up` and `chaps open ocs` opens its web interface.

Without the object store, OCS keeps its data on its own volume:

```sh
chaps init climate --only ocs
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
second returns the collection; `chaps status` then counts one dataset. The
extent is the example one in `ocs/climate-service.yaml` until you change it,
either in that file or at `init` with `--ocs-name`, `--ocs-country` and
`--ocs-bbox`. ERA5-Land needs a Copernicus Climate Data Store and/or an Earth
Data Hub account, set in `.env`; WorldPop and CHIRPS3 need none.

## As a server

Two settings matter once other people use it:

```sh
chaps components enable ocs --read-only                          # refuse writes over HTTP
chaps components enable ocs --base-url https://ocs.example.org   # behind a proxy
```

Both have `init` spellings too, `--ocs-read-only` and `--ocs-base-url`. A
read-only instance refuses every ingestion over HTTP, so ingest first.

Next: [Standalone OCS](../components.md#standalone-ocs),
[Data source credentials](../components.md#data-source-credentials),
[Read-only instances](../components.md#read-only-instances),
[Behind a reverse proxy](../components.md#behind-a-reverse-proxy).

All shapes: [Use cases](../use-cases.md).
