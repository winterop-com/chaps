# A DHIS2 on its own

A DHIS2 instance and its database, nothing else: for trying a DHIS2 version,
developing an app, or restoring a database dump to look at it.

```sh
chaps init dhis --only dhis2                              # the version-matched demo database
chaps init dhis --only dhis2 --dhis2-seed none            # an empty instance
chaps init dhis --only dhis2 --dhis2-seed dumps/mine.sql.gz   # your own dump
cd dhis
chaps up
chaps open dhis2
```

The seed is applied once, when the database is first created. `chaps dhis2
connect` refuses here, since there is no chap-core to connect to;
`chaps components enable chap-core` adds one.

Next: [The seed](../dhis2.md#the-seed),
[Changing the DHIS2 version](../dhis2.md#changing-the-dhis2-version).

All shapes: [Use cases](../use-cases.md).
