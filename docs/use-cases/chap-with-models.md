# CHAP with forecasting models

The default, and what most deployments are: chap-core plus the models it runs.

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps status                 # chap-core up, every model registered
chaps models test --all      # make each model train and predict once
```

You get chap-core's API on port 8000 and one service per model. The models
register with chap-core over the compose network and publish no host port;
you reach them through chap-core at
`http://localhost:8000/v2/services/<service_id>/run/`. Pick models later with
`chaps ui`, or `chaps models enable ID` / `disable ID`.

Next: [Models and the marketplace](../models.md), [Authentication](../auth.md)
before you put it on a network, [Backup and restore](../backup.md).

All shapes: [Use cases](../use-cases.md).
