# Introduction

`chaps` runs the CHAP stack. The name is what it does: **CHAP Stack**. It
deploys and manages [CHAP](https://chap.dhis2.org), the Climate Health
Analytics Platform, as a Docker Compose deployment of
[chap-core](https://github.com/dhis2-chap/chap-core) together with the
forecasting model services published in the
[CHAP model marketplace](https://github.com/dhis2-chap/model-marketplace).

A deployment is made of **components**. chap-core is one, and it is on unless
you turn it off; OCS (Open Climate Service) is available as an optional
component beside it, together with the S3-compatible object store OCS will use.
Existing projects keep working unchanged - a deployment that says nothing about
components is chap-core alone, which is what it was. See
[Components](./components.md).

> **chap or chaps?** `chap` is chap-core's own developer CLI, the one that
> serves the API and runs evaluations from a checkout of chap-core. `chaps` is
> this tool: it deploys and manages CHAP on a machine, and never runs a
> forecast itself.
>
> Typing a `chap` command at `chaps` says so rather than printing a bare
> "unrecognized subcommand".

## What it does, in one screen

One command writes a self-contained deployment directory: the base services
(chap-core, its worker, Valkey and PostgreSQL), one Compose overlay per enabled
model, an `.env` file, and a `.chaps/` directory that records exactly what the
deployment is meant to be.

```sh
chaps init mychap --models default   # writes the deployment directory
chaps init mychap --with ocs         # ...with Open Climate Service beside it
cd mychap
chaps up                             # sync the compose files, docker compose up -d
chaps status                         # chap-core health and registered models
chaps ui                             # browse the marketplace, toggle models
chaps up                             # apply what the browser changed
chaps update                         # move the pins to what upstream publishes now
chaps restart                        # apply them to the services that are running
```

After `init`, `chaps` is two things at once: a thin wrapper around
`docker compose` that always passes the explicit `-f` list, and a model manager
that can add or remove models without you hand-editing YAML.

Every command takes `--json` for machine-readable output, and every command
ends with a line saying what it did or found. Empty output is a bug.

## The three words to remember

- **`chaps up`** starts what is on disk. It renders the compose files from
  `.chaps/`, checks the host ports, and runs `docker compose up -d`. It never
  changes which version of anything you run.
- **`chaps update`** fetches newer versions. It asks the marketplace and the
  chap-core release feed what they publish today, moves the pins that follow a
  channel or a release, and pulls the images. It never touches a container: it
  ends by naming the services that are now running something out of date.
- **`chaps restart`** applies them to the services that are running. It
  recreates the containers that no longer match the files and leaves the rest
  alone, and it changes no file and no pin.

Everything else follows from that split. `chaps up`, `chaps restart`,
`chaps models expose` and `chaps models unexpose` are safe on a deployment
running a build you do not want moved; `chaps update` and `chaps models enable`
are the only commands that move a version.

## The pure server promise

A machine that runs CHAP with `chaps` needs Docker and one binary. Nothing
else: no Python, no `uv`, no checkout of chap-core, no build step on the
server. The Linux releases are static musl builds, so the same file runs on any
distribution.

The compose files `chaps` writes are plain Compose files with nothing
`chaps`-specific in them, so

```sh
docker compose -f compose.yml -f compose.chaps.yml -f compose.marketplace.yml up -d
```

works on a machine that has never had `chaps` installed. That is the point of
generating them rather than keeping the deployment inside the tool.

## Where to go next

- [Install](./install.md) puts the binary on the machine.
- [Quickstart](./quickstart.md) walks a first deployment end to end.
- [Concepts](./concepts.md) explains the project directory, intent versus
  artifacts, and the pins.
- [Commands](./commands.md) is the tour; [Command reference](./reference.md) is
  the generated, complete list.

`chaps` is licensed under the AGPL-3.0, like chap-core. The full text is in
`LICENSE` at the repository root.
