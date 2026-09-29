# Use cases

Helping someone as an AI assistant? Start at [For AI assistants](./ai.md),
which turns these pages into a menu of options with the exact commands.

`chaps` deploys CHAP, but CHAP is only one of the shapes a deployment can take.
The same directory layout, the same commands and the same `chaps up` /
`chaps status` / `chaps doctor` loop work for any combination of:

| Piece | What it is | How you ask for it |
| --- | --- | --- |
| chap-core | CHAP itself: the API, its worker, Valkey and PostgreSQL | on by default; `--without chap-core` or `--only` leaves it out |
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

- [CHAP with forecasting models](./use-cases/chap-with-models.md): chap-core and the forecasting models it runs. The default.
- [CHAP with climate data from OCS](./use-cases/chap-with-ocs.md): chap-core with Open Climate Service beside it for climate data.
- [CHAP with a local DHIS2 and the Modeling App](./use-cases/chap-with-local-dhis2.md): chap-core, models and a demo DHIS2, connected for the Modeling App.
- [CHAP for a DHIS2 that runs elsewhere](./use-cases/chap-for-external-dhis2.md): chap-core on a server, used by a DHIS2 that runs elsewhere.
- [An OCS server on its own](./use-cases/ocs-alone.md): Open Climate Service and its object store, no CHAP.
- [A DHIS2 on its own](./use-cases/dhis2-alone.md): A DHIS2 and its database, nothing else.
- [A chapkit model service on its own](./use-cases/model-alone.md): One chapkit model service answering on its own port, no chap-core.
- [Several model services side by side](./use-cases/models-alone.md): Several model services, each on its own port, no chap-core.
- [Your model from its checkout, with CHAP](./use-cases/model-on-host.md): chaps runs chap-core; you run your model with `uv run` and it registers.
- [chap-core from its checkout, with the models](./use-cases/chap-core-on-host.md): you run chap-core; chaps runs the models and registers them with it.
- [chap-core built from its checkout](./use-cases/chap-core-from-checkout.md): chaps builds chap-core from your clone and runs it with everything else.
- [A model image you built yourself](./use-cases/local-model-image.md): `docker build` a model and run it in CHAP without publishing it.
- [Combinations](./use-cases/combinations.md): Other mixes of components and models.
- [Growing a deployment](./use-cases/growing.md): Moving a deployment from one shape to another.

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
