# Combinations

Components and models combine freely. A few that come up:

```sh
chaps init lab --only ocs,s3,dhis2             # climate data and a DHIS2, no CHAP
chaps init lab --only ocs --models chapkit_ewars_model   # a model beside OCS, no CHAP
chaps init full --models default --with ocs,s3,dhis2     # everything
```

All shapes: [Use cases](../use-cases.md).
