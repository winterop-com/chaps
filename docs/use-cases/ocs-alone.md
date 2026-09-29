# An OCS server on its own

Open Climate Service with its object store and no CHAP at all: a climate data
server for anything that speaks STAC or openEO.

```sh
chaps init climate --only ocs,s3
cd climate
chaps up
chaps status                 # ocs up, with its dataset count and data size
chaps open ocs
```

Nothing CHAP-specific is created: no `compose.yml`, no chap-core release is
looked up. Set the credentials your datasets need in `.env`. Two settings
matter for a server:

```sh
chaps components enable ocs --read-only                    # refuse writes over HTTP
chaps components enable ocs --base-url https://ocs.example.org   # behind a proxy
```

Next: [Standalone OCS](../components.md#standalone-ocs),
[Read-only instances](../components.md#read-only-instances),
[Behind a reverse proxy](../components.md#behind-a-reverse-proxy).

All shapes: [Use cases](../use-cases.md).
