# CHAP with a local DHIS2 and the Modeling App

A complete local setup: chap-core, its models, and a DHIS2 seeded from the demo
database, connected so the Modeling App in DHIS2 talks to this CHAP. For
development, demos and training.

```sh
chaps init mychap --models default --with dhis2
cd mychap
chaps up                     # DHIS2's first start takes several minutes
chaps status                 # wait for the dhis2 line to say up
chaps dhis2 connect          # the route to chap-core, the apps, analytics
chaps open dhis2
```

`chaps up` does not connect DHIS2 to CHAP on its own; `chaps dhis2 connect` is
the step that does, and `up` and `status` remind you until it has run once.

Next: [DHIS2](../dhis2.md), especially
[Connecting the Modeling App to CHAP](../dhis2.md#connecting-the-modeling-app-to-chap)
and [Memory](../dhis2.md#memory).

All shapes: [Use cases](../use-cases.md).
