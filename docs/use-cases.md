# Use cases

Helping someone as an AI assistant? Start at [For AI assistants](./ai.md),
which turns these pages into a menu of options with the exact commands.

`chaps` deploys Chap, but Chap is only one of the shapes a deployment can take.
The same directory layout, the same commands and the same `chaps up` /
`chaps status` / `chaps doctor` loop work for any combination of:

| Piece | What it is | How you ask for it |
| --- | --- | --- |
| chap-core | Chap itself: the API, its worker, Valkey and PostgreSQL | on by default; `--without chap-core` or `--only` leaves it out |
| model services | chapkit forecasting models from the marketplace, or your own | `--models ID,...`, later `chaps models enable ID` |
| `ocs` | Open Climate Service: climate data over STAC and openEO | `--with ocs`, or `--only ocs` |
| `s3` | an S3-compatible object store for OCS to keep objects in | `--with s3`, or `--only ocs,s3` |
| `dhis2` | a demo or development DHIS2 with its own database | `--with dhis2`, or `--only dhis2` |

`--with` adds components to chap-core. `--only` names the whole set, and
chap-core is part of it only if you list it. Each page below is one shape
people actually deploy: the command that creates it, what you get, and what to
run next. They are not exclusive: a deployment can move from one shape to
another later with `chaps components enable` / `disable` and `chaps models
enable` / `disable`. See [Growing a deployment](./use-cases/growing.md).

## Only one piece

| I want | Page | First command |
| --- | --- | --- |
| only DHIS2 (2.42, 2.43, 2.41 or the next one) | [A DHIS2 on its own](./use-cases/dhis2-alone.md) | `chaps init dhis --only dhis2` |
| only OCS (with or without S3) | [An OCS server on its own](./use-cases/ocs-alone.md) | `chaps init climate --only ocs,s3` |
| only one model service | [A chapkit model service on its own](./use-cases/model-alone.md) | `chaps run chapkit_ewars_model` |
| only some model services | [Several model services side by side](./use-cases/models-alone.md) | `chaps init models --only none --models ID,ID` |
| any number of model services, no folder | [Any number of model services, no folder](./use-cases/models-with-run.md) | `chaps run ID`, once per model |

## Chap

- [Chap with forecasting models](./use-cases/chap-with-models.md): chap-core and the models it runs; the default, or every model in the marketplace.
- [Chap with climate data from OCS](./use-cases/chap-with-ocs.md): chap-core with Open Climate Service beside it.
- [Chap with a local DHIS2 and the Modeling App](./use-cases/chap-with-local-dhis2.md): chap-core, models and a DHIS2 of the version you pick, connected.
- [Chap for a DHIS2 that runs elsewhere](./use-cases/chap-for-external-dhis2.md): chap-core on a server, used by a DHIS2 someone else runs.
- [Combinations](./use-cases/combinations.md): OCS and DHIS2 without Chap, a model beside OCS, everything at once.

## Developing Chap, a model or DHIS2

- [Your model from its checkout, with Chap](./use-cases/model-on-host.md): chaps runs chap-core; you run your model with `uv run` and it registers.
- [A model image you built yourself](./use-cases/local-model-image.md): `docker build` a model and run it in Chap without publishing it.
- [chap-core from its checkout, with the models](./use-cases/chap-core-on-host.md): you run chap-core; chaps runs the models and registers them with it.
- [chap-core built from its checkout](./use-cases/chap-core-from-checkout.md): chaps builds chap-core from your clone and runs it with everything else.
- [A DHIS2 you run yourself, with Chap from chaps](./use-cases/dhis2-dev-with-chap.md): your DHIS2, chaps' chap-core, connected.
- [A DHIS2 from chaps, with a chap-core elsewhere](./use-cases/dhis2-with-chap-core-elsewhere.md): chaps' DHIS2, your chap-core, connected.

## More than one, and changing your mind

- [Several deployments on one machine](./use-cases/several-deployments.md): taking turns on the default ports, or giving each its own.
- [Growing a deployment](./use-cases/growing.md): adding or removing chap-core, OCS, DHIS2 or models later.

## Which commands work where

| Command | Needs chap-core | Without chap-core |
| --- | --- | --- |
| `up`, `down`, `restart`, `logs`, `status`, `doctor`, `sync` | no | work on whatever the deployment has |
| `components`, `models enable/disable/add/expose` | no | work; models get a host port |
| `update` | no | pulls the images; `--chap-tag` and `--pin-chap-core` refuse |
| `backup create` / `restore` | no | back up files and volumes; no database to dump |
| `open` | no | opens whatever is enabled |
| `models test`, `jobs`, `api` | yes | refuse, naming `chaps components enable chap-core` (`api --url` still works) |
| `dhis2 connect` | yes | refuses: there is no chap-core to connect DHIS2 to |
