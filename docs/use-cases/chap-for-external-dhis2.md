# CHAP for a DHIS2 that runs elsewhere

chap-core and its models on a server, used by an existing DHIS2 instance.

```sh
chaps init mychap --models default --api-token
cd mychap
chaps up
chaps dhis2 use https://dhis2.example.org --chap-url https://chap.example.org
chaps dhis2 connect
```

`--api-token` protects chap-core's API from the start. `dhis2 use` records the
external DHIS2 and the address it reaches chap-core at; `connect` then creates
the route on that DHIS2 with the credentials you give it, and puts chap-core's
token in the route, so the Modeling App reaches chap-core without ever holding
the token itself. On a DHIS2 chaps does not run, the route is all `connect`
changes: installing the Modeling and Climate apps (`chaps dhis2 apps`) and
generating analytics tables (`chaps dhis2 analytics`, which can take hours on a
large instance) are left to its admin, and `connect` says which apps are
missing.

It worked when `chaps dhis2 connect` ends without `error:` and
`chaps dhis2 show` lists the `chap` route as healthy. The first run usually
stops to say it has no credentials for that DHIS2: put
`DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` (or `DHIS2_API_TOKEN`) in
`.env` and run it again. `--chap-url` has to be an address that DHIS2 server
can reach this machine at, not `localhost`.

Next: [A DHIS2 that runs elsewhere](../dhis2.md#a-dhis2-that-runs-elsewhere),
[Authentication](../auth.md).

All shapes: [Use cases](../use-cases.md).
