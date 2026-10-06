# Chap with a local DHIS2 and the Modeling App

A complete local setup: chap-core, its models, and a DHIS2 seeded from the demo
database, connected so the Modeling App in DHIS2 talks to this Chap. For
development, demos and training.

```sh
varde init mychap --models default --with dhis2
cd mychap
varde up                     # DHIS2's first start takes several minutes
varde status                 # wait for the dhis2 line to say up
varde dhis2 connect          # the route to chap-core, the apps, analytics
varde open dhis2
```

`varde up` does not connect DHIS2 to Chap on its own; `varde dhis2 connect` is
the step that does, and `up` and `status` remind you until it has run once.

It worked when `varde dhis2 connect` ends without `error:`, `varde open dhis2`
opens DHIS2 (on `http://localhost:8780`, login `admin` / `district`), and the
Modeling App in its app menu lists the models.

Next, in the Modeling App: [Your first forecast in the Modeling
App](../modeling-app.md) evaluates a model on the demo data and makes a
forecast with it.

## Another DHIS2 version

```sh
varde init mychap --models default --with dhis2 --dhis2-tag 2.43
varde init mychap --models default --with dhis2 --dhis2-tag 2.41
varde init mychap --models default --with dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master
```

The Laos climate demo database exists for 2.42 only, so these start with an
empty DHIS2 (still with `admin` / `district`); `dhis2 connect` works the same.
Do not seed them with DHIS2's Sierra Leone demo when you want Chap connected:
its `admin` is not a superuser, and DHIS2 refuses it the route. See
[A DHIS2 on its own](./dhis2-alone.md#picking-the-version) for the versions and
the data.

Next: [DHIS2](../dhis2.md), especially
[Connecting the Modeling App to Chap](../dhis2.md#connecting-the-modeling-app-to-chap)
and [Memory](../dhis2.md#memory).

All shapes: [Use cases](../use-cases.md).
