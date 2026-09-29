# Combinations

Components and models combine freely: `--only` names exactly the components
you want (and chap-core only if you list it), `--with` adds components to
chap-core, and `--models` names the model services either way.

## Climate data and a DHIS2, no CHAP

OCS with its object store beside a DHIS2, for work on climate data in DHIS2
without CHAP in the picture:

```sh
chaps init lab --only ocs,s3,dhis2
cd lab
chaps up                     # DHIS2's first start takes several minutes
chaps status                 # run it again until every line says up
chaps open ocs
chaps open dhis2
```

It worked when `chaps status` shows `ocs`, `s3` and `dhis2` as `up`. OCS is on
`http://localhost:8790` and DHIS2 on `http://localhost:8780`; inside the
deployment DHIS2 can reach OCS at `http://ocs:9000`. `chaps dhis2 connect`
refuses, since there is no chap-core; `chaps components enable chap-core` adds
it.

## A model beside OCS, no CHAP

```sh
chaps init lab --only ocs --models chapkit_ewars_model
```

The model answers on its own host port (5001), as in
[A chapkit model service on its own](./model-alone.md), and OCS on 8790.

## Everything

```sh
chaps init full --models default --with ocs,s3,dhis2
```

chap-core, its models, OCS with the object store and a DHIS2, all in one
deployment. Follow [CHAP with a local DHIS2](./chap-with-local-dhis2.md) for
connecting DHIS2 to CHAP, and [CHAP with climate data from
OCS](./chap-with-ocs.md) for the OCS side.

## Settings any of these take

`--dhis2-tag` and `--dhis2-seed` for DHIS2 (see
[A DHIS2 on its own](./dhis2-alone.md#picking-the-version)), `--ocs-read-only`
and `--ocs-base-url` for OCS, and `--api-port`, `--ocs-port`, `--dhis2-port`
for ports other than the defaults (see
[Several deployments on one machine](./several-deployments.md)).

All shapes: [Use cases](../use-cases.md).
