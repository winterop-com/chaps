# Introduction

[Chap](https://chap.dhis2.org) forecasts
cases of climate-sensitive diseases such as dengue and malaria. It learns from
past case counts together with climate data and population, scores how well
each forecasting model would have done on the past, and predicts the coming
months per district or province. It is often used together with DHIS2,
through the Modeling App. [Your first forecast in the Modeling App](./modeling-app.md)
shows it end to end on demo data.

`varde` deploys and manages [Chap](https://chap.dhis2.org) and the services
around it, as Docker Compose deployments:
[chap-core](https://github.com/dhis2-chap/chap-core), the forecasting model
services published in the
[Chap model marketplace](https://github.com/dhis2-chap/model-marketplace),
[Open Climate Service](https://github.com/dhis2/open-climate-service) (OCS) with
an S3-compatible object store, and [DHIS2](https://dhis2.org).

They deploy together or any of them on its own. A deployment is made of
**components** (chap-core, OCS, the object store, DHIS2) plus the model
services you enable. chap-core is on unless you leave it out, so the default is
a Chap deployment. `varde init --only ocs,s3` is an OCS server,
`varde init --only dhis2` a DHIS2, and
`varde init --only none --models ID` one model service answering on its own
port. [Use cases](./use-cases.md) walks through each shape, and
[Components](./components.md) is the reference.

## The name

A *varde* (say "VAR-deh") is a Norwegian word with two meanings.

- **A beacon.** From the Middle Ages, Norway had a chain of beacons on the
  hills along its coast, and the law said how to build and keep them. When a
  guard saw danger, he lit his varde. The next guard saw the fire and lit his,
  and the warning went along the coast faster than any messenger. It was an
  early warning system for the whole country.
- **A cairn.** Today a varde is a pile of stones on a mountain path. It shows
  hikers that they are on the right way.

Both meanings fit the tool:

- **Early warning is the purpose of Chap.** Chap forecasts outbreaks of
  diseases such as malaria and dengue from climate data, so that a health
  service can act before the cases come. A varde did the same for an attack:
  it saw the danger early and passed the warning on.
- **One varde alone does little; the chain does the work.** `varde` puts a
  chain together: DHIS2, chap-core, the models and OCS, where each part passes
  its work to the next.
- **A cairn shows the way.** `varde status`, `varde doctor` and the hints show
  the next step to the person who runs Chap.

The word is Norwegian because DHIS2 is Norwegian too: it started at a
university in Norway.

The command has a short form: `vg` is the same command as `varde`. The install
script puts it next to `varde`.

## What it does, in one screen

One command writes a self-contained deployment directory: the base services
(chap-core, its worker, Valkey and PostgreSQL), one Compose overlay per enabled
model, an `.env` file, and a `.varde/` directory that records exactly what the
deployment is meant to be.

```sh
varde init mychap --models default   # writes the deployment directory
cd mychap
varde up                             # sync the compose files, docker compose up -d
varde status                         # chap-core health and registered models
varde ui                             # browse the marketplace, toggle models
varde up                             # apply what the browser changed
varde update                         # move the pins to what upstream publishes now
varde restart                        # apply them to the services that are running
```

Other shapes:

```sh
varde init mychap --with ocs         # Chap with Open Climate Service beside it
varde init climate --only ocs,s3     # OCS alone, with no Chap
```

A new model:

```sh
varde models new my-model            # a model project, from a template
```

After `init`, `varde` is two things at once: a thin wrapper around
`docker compose` that always passes the explicit `-f` list, and a model manager
that can add or remove models without you hand-editing YAML.

Every command takes `--json` for machine-readable output, and every command
ends with a line saying what it did or found. Empty output is a bug.

## The three words to remember

- **`varde up`** starts what is on disk. It renders the compose files from
  `.varde/`, checks the host ports, and runs `docker compose up -d`. It never
  moves a pin. Only `varde up --pull` downloads a newer build, and only for an
  image on a moving tag such as the `main` tag of OCS.
- **`varde update`** fetches newer versions. It asks the marketplace and the
  chap-core release feed what they publish today, moves the pins that follow a
  channel or a release, and pulls the images. It never touches a container: it
  ends by naming the services that are now running something out of date.
- **`varde restart`** applies them to the services that are running. It
  recreates the containers that no longer match the files and leaves the rest
  alone, and it changes no file and no pin.

Everything else follows from that split. `varde up`, `varde restart`,
`varde models expose` and `varde models unexpose` are safe on a deployment
running a build you do not want moved. `varde update`, `varde models enable`,
`varde models add` and `varde components enable --tag` are the commands that
move a version; see [What moves a pin](./updating.md#what-moves-a-pin).

## The pure server promise

A machine that runs Chap with `varde` needs Docker and one binary. Nothing
else: no Python, no `uv`, no checkout of chap-core, no build step on the
server. The Linux releases are static musl builds, so the same file runs on any
distribution.

The compose files `varde` writes are plain Compose files with nothing
`varde`-specific in them, so

```sh
docker compose -f compose.yml -f compose.varde.yml -f compose.marketplace.yml up -d
```

works on a machine that has never had `varde` installed. That is the point of
generating them rather than keeping the deployment inside the tool.

That is the list for chap-core and models. A component adds its own file
before `compose.marketplace.yml`, for example `-f compose.ocs.yml`. A
deployment without chap-core has no `compose.yml` or `compose.varde.yml`.

## Where to go next

- [Install](./install.md) puts the binary on the machine.
- [Quickstart](./quickstart.md) walks a first deployment end to end.
- [Your first forecast in the Modeling App](./modeling-app.md) uses one.
- [Concepts](./concepts.md) explains the deployment directory, intent versus
  artifacts, and the pins.
- [Commands](./commands.md) is the tour; [Command reference](./reference.md) is
  the generated, complete list.

`varde` is licensed under the AGPL-3.0, like chap-core. The full text is in
`LICENSE` at the repository root.
