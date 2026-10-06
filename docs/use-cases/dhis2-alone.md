# A DHIS2 on its own

A DHIS2 instance and its database, nothing else: for trying a DHIS2 version,
developing an app, or restoring a database dump to look at it.

```sh
varde init dhis --only dhis2
cd dhis
varde up                     # the first start takes several minutes
varde status                 # run it again until the dhis2 line says up
varde open dhis2
```

You get DHIS2 on `http://localhost:8780`, with its own PostgreSQL, and no
chap-core, OCS or models. It worked when `varde status` shows `dhis2` as `up`
and the login `admin` / `district` works in the browser.

DHIS2 needs about 4 to 5 GB of memory; give Docker 8 GB or more.

## Picking the version

`--dhis2-tag` picks the DHIS2 version, and `--dhis2-seed` what its database
starts from:

| Version | Command | Database it starts with |
| --- | --- | --- |
| 2.42 (the default) | `varde init dhis --only dhis2` | the Laos climate demo |
| 2.43 | `varde init dhis --only dhis2 --dhis2-tag 2.43` | empty |
| 2.41 | `varde init dhis --only dhis2 --dhis2-tag 2.41` | empty |
| the next, unreleased | `varde init dhis --only dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master` | empty |

A full version such as `--dhis2-tag 2.42.6` works too. varde has a demo
database for 2.42 only, so every other version starts empty and `init` says
so. An empty DHIS2 still has the `admin` / `district` login, as a superuser.

## Picking the data

```sh
varde init dhis --only dhis2 --dhis2-seed none                  # empty, whatever the version
varde init dhis --only dhis2 --dhis2-seed dumps/mine.sql.gz     # your own dump, a path
varde init dhis --only dhis2 --dhis2-seed dumps/mine.sql.gz \
  --dhis2-seed-password                                         # every user logs in with district
varde init dhis --only dhis2 --dhis2-tag 2.43 \
  --dhis2-seed https://databases.dhis2.org/sierra-leone/2.43/dhis2-db-sierra-leone.sql.gz
```

The seed is applied once, when the database is first created; after that the
database is DHIS2's. DHIS2's Sierra Leone demo is published for 2.41, 2.42 and
2.43 at that address pattern. It has no climate data, and its `admin` is not a
superuser: it cannot create the route the Modeling App uses, so pick it for a
DHIS2 on its own, not for one you will connect to Chap.

A dump of a real DHIS2 keeps the passwords of its users.
`--dhis2-seed-password` gives every user the password `district`, turns on
every account and turns off two-factor login, so you can log in as any of
them. See
[One password for every user](../dhis2.md#one-password-for-every-user).

## Next

`varde dhis2 connect` refuses here, since there is no chap-core to connect to;
`varde components enable chap-core` adds one, and
[Chap with a local DHIS2](./chap-with-local-dhis2.md) is that shape from the
start. More: [The seed](../dhis2.md#the-seed),
[Changing the DHIS2 version](../dhis2.md#changing-the-dhis2-version),
[Memory](../dhis2.md#memory).

All shapes: [Use cases](../use-cases.md).
