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

It worked when `chaps dhis2 connect` ends without `error:`, `chaps open dhis2`
opens DHIS2 (on `http://localhost:8780`, login `admin` / `district`), and the
Modeling App in its app menu lists the models.

## Another DHIS2 version

```sh
chaps init mychap --models default --with dhis2 --dhis2-tag 2.43
chaps init mychap --models default --with dhis2 --dhis2-tag 2.41
chaps init mychap --models default --with dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master
```

The Laos climate demo database exists for 2.42 only, so these start with an
empty DHIS2 (still with `admin` / `district`); `dhis2 connect` works the same.
Do not seed them with DHIS2's Sierra Leone demo when you want CHAP connected:
its `admin` is not a superuser, and DHIS2 refuses it the route. See
[A DHIS2 on its own](./dhis2-alone.md#picking-the-version) for the versions and
the data.

Next: [DHIS2](../dhis2.md), especially
[Connecting the Modeling App to CHAP](../dhis2.md#connecting-the-modeling-app-to-chap)
and [Memory](../dhis2.md#memory).

All shapes: [Use cases](../use-cases.md).
