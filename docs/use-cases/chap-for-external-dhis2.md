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
the route on that DHIS2 with the credentials you give it.

Next: [A DHIS2 that runs elsewhere](../dhis2.md#a-dhis2-that-runs-elsewhere),
[Authentication](../auth.md).

All shapes: [Use cases](../use-cases.md).
