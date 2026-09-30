# For AI assistants

This page is written for an AI assistant helping someone set something up with
`chaps`. If a person pointed you here, follow the steps below in order. Keep
your answers short and concrete, and give one command at a time.

## What CHAP is

Use this when the person asks what they are installing, or seems unsure why
they would want it.

[CHAP](https://chap.dhis2.org), the Climate Health Analytics Platform, forecasts
cases of climate-sensitive diseases such as dengue and malaria. It learns from
past case counts together with climate data (rainfall, temperature) and
population, and predicts the next months for each district or province. Before
trusting a model it runs an *evaluation*: it hides the last part of the data,
forecasts it, and scores how close the forecast came. Ministries of health use
it together with [DHIS2](https://dhis2.org), the health information system
where their case data already lives, through the **Modeling App** inside DHIS2.

`chaps` is the installer. It runs CHAP, its forecasting models, a DHIS2 with
demo data and the other pieces on one computer with Docker, so the person can
use them without setting each one up by hand.

A few words the person will see:

| Word | Meaning |
| --- | --- |
| chap-core | CHAP itself: the service that trains and runs the models. |
| model | One forecasting method, for example CHAP-EWARS. Each runs in its own container. |
| DHIS2 | The health information system. `chaps` can run a demo one with data from Laos. |
| Modeling App | CHAP's user interface, installed inside DHIS2. |
| evaluation | Testing a model on past data it did not see, with scores. Also called a backtest. |
| prediction | A forecast for the coming months. |
| OCS | Open Climate Service: downloads and serves climate data. Optional. |

## How to help

1. **Ask what they want to do**, in one question, and match the answer to a
   group in [The options](#the-options). Do not show all eighteen options.
   - *Learn CHAP, try forecasting, see what it does* (a student, an analyst, a
     public health person): **option 1**. It is the only one with a user
     interface they can click through. If their computer cannot give Docker
     8 GB of memory, option 2 instead (6 GB).
   - *Run CHAP for a DHIS2 that already exists*: option 5.
   - *One piece on its own* (climate data, one model, a DHIS2): options 6 to 10.
   - *Develop a model, chap-core or DHIS2*: options 11 to 17.
   - *Two deployments at once*: option 18.

   If they are unsure, recommend option 1.
2. **Check the machine** with [Before any option](#before-any-option).
3. **Give the commands for their option**, in order, one block at a time. Wait
   for them to say it worked before giving the next block. Every option has an
   "It worked when" line: use it to decide.
4. **When it works, say what to do next.** Options 1 and 3 continue in the
   Modeling App: walk them through [Your first forecast in the Modeling
   App](./modeling-app.md). Option 2 continues with
   [What to try next without DHIS2](#what-to-try-next-without-dhis2).
5. **If something fails**, ask them to paste the exact output. Then look for the
   first line that starts with `error:` or `warning:`. chaps puts the fix on the
   same line, usually a command in backticks: tell them to run that command.
   If that does not help, search [Troubleshooting](./troubleshooting.md) for
   the exact message. If chaps says it does not know a command or an option,
   the installed `chaps` is older than this page: `chaps self update` brings
   it up to date, and `chaps <command> --help` is always right for the
   installed one.
6. **Never** tell them to edit the `compose*.yml` files by hand (chaps rewrites
   them), and never ask them to paste the `.env` file (it holds passwords).

When an option has a "More" link, that page has the details. You only need it
if the person asks for something the steps here do not cover.

## If you run the commands yourself

If you have a shell on the person's machine and run the commands rather than
telling the person what to type:

- Run each block as written, top to bottom. Where an option shows
  alternatives, it says so and you pick one. Check its "It worked when" line
  yourself instead of waiting for the person to say it worked.
- If your shell does not keep its folder between commands, put
  `-C mychap` (the folder from `chaps init`) on every command in place of the
  `cd`: `chaps -C mychap up`.
- `chaps status` exits `0` only when everything the deployment declares is up
  and registered, and non-zero otherwise, so poll it by its exit code where
  a step says to run it every minute. A first DHIS2 start can take 20
  minutes.
  `chaps status --json` gives the same report as one JSON document; every
  command takes `--json`.
- The first `chaps up` downloads the images that take most of the disk in
  [Before any option](#before-any-option). Give it a long timeout, or run it
  in the background, and do not start a second one while it runs. Running
  `chaps up` again after it has finished or timed out is safe: it picks up
  where it stopped.
- `chaps dhis2 connect` waits by itself for DHIS2 to start, up to 20 minutes;
  it prints a line on stderr while it waits.
- Nothing prompts you: without a terminal, a question becomes an `error:`
  with the flag that answers it. `chaps up --replace` stops another
  deployment holding the ports, `chaps down --volumes --yes` deletes data;
  ask the person before either.
- Credentials go in the `.env` file in the folder. Ask the person to add them
  there themselves, and do not print or read that file.
- Do not run `chaps ui` (an interactive screen), `chaps up --attach` or
  `chaps logs -f` (they never return). `chaps logs SERVICE` without `-f`
  prints and returns.
- `chaps open` opens a browser on the machine it runs on; when that is not in
  front of the person, give them the address from `chaps status` instead.

## Before any option

The person needs Docker: Docker Desktop on macOS and Windows, Docker Engine on
Linux. Check two numbers before they start:

| | With DHIS2 (options 1, 3, 8, 9, 10, 17) | Without DHIS2 |
| --- | --- | --- |
| Memory for Docker | 8 GB or more | 6 GB or more |
| Free disk | about 30 GB | about 25 GB |

Most of the disk is the model and worker images, which the first `chaps up`
downloads: expect it to take a while on a slow connection. On Docker Desktop
the memory is set under Settings, Resources. Then:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps doctor
```

On Windows, download `chaps-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/winterop-com/chaps/releases/latest), unzip
it, and run `.\chaps.exe doctor` in PowerShell from that folder. Every command
below is the same on Windows with `.\chaps.exe` in place of `chaps`. See
[Install](./install.md).

It worked when `chaps doctor` shows no `fail` lines. On an Apple Silicon Mac a
`warn` line about `linux/amd64` and emulation is normal and can be ignored:
the models run, only slower.

Every option below creates a new folder. Tell the person to run the commands
from the folder where they want it created, and then to `cd` into it as shown.
Several options use the same folder name (`mychap`): if one from an earlier
option is still there, `chaps init` says it `already contains a chaps project`.
Either pick another name in the `chaps init` line (and the `cd`), or remove the
old one first as shown in [Every option: stop, start, remove](#every-option-stop-start-remove).

## The options

| # | Option | Pick this if the person wants to... |
| --- | --- | --- |
| | **Use CHAP** | |
| 1 | [CHAP in DHIS2, with the Modeling App](#1-chap-in-dhis2-with-the-modeling-app) | learn CHAP: evaluate a model and make a forecast by clicking |
| 2 | [CHAP with forecasting models](#2-chap-with-forecasting-models) | run CHAP without DHIS2, from the terminal or its API |
| 3 | [CHAP with climate data](#3-chap-with-climate-data) | use CHAP together with Open Climate Service |
| 4 | [CHAP with every model](#4-chap-with-every-model) | try all the forecasting models, not just one |
| 5 | [CHAP for a DHIS2 that already exists](#5-chap-for-a-dhis2-that-already-exists) | connect CHAP to a DHIS2 someone else runs |
| | **One piece on its own** | |
| 6 | [Climate data only](#6-climate-data-only) | run Open Climate Service, no CHAP |
| 7 | [One forecasting model only](#7-one-forecasting-model-only) | call one model's API directly, no CHAP |
| 8 | [DHIS2 only](#8-dhis2-only) | have a DHIS2 to play with, no CHAP |
| 9 | [A particular DHIS2 version](#9-a-particular-dhis2-version) | run DHIS2 2.41, 2.43 or the next, unreleased one |
| 10 | [Climate data and DHIS2, no CHAP](#10-climate-data-and-dhis2-no-chap) | have OCS and a DHIS2 together, without CHAP |
| | **Develop** | |
| 11 | [My own model](#11-my-own-model) | run a model they wrote themselves |
| 12 | [My own model, while I work on it](#12-my-own-model-while-i-work-on-it) | develop a model in its folder and have CHAP use it |
| 13 | [A model image I built](#13-a-model-image-i-built) | run a model image they built with `docker build`, without publishing it |
| 14 | [My own chap-core, with the models](#14-my-own-chap-core-with-the-models) | develop chap-core itself and have the models register with it |
| 15 | [chap-core built from my checkout](#15-chap-core-built-from-my-checkout) | test their chap-core changes with everything else around them |
| 16 | [My own DHIS2, with CHAP](#16-my-own-dhis2-with-chap) | connect a DHIS2 they run themselves to CHAP from chaps |
| 17 | [A DHIS2, with my own chap-core](#17-a-dhis2-with-my-own-chap-core) | have chaps run DHIS2 in front of the chap-core they are developing |
| | **More than one** | |
| 18 | [Two of these at the same time](#18-two-of-these-at-the-same-time) | keep one option running while starting another |

### 1. CHAP in DHIS2, with the Modeling App

Needs 8 GB of memory for Docker. The DHIS2 comes with demo data from Laos:
weekly dengue cases for 2019 to 2024 in 18 provinces, with rainfall,
temperature and population.

```sh
chaps init mychap --models default --with dhis2
cd mychap
chaps up
chaps status
```

The first time, DHIS2 takes several minutes to start; `chaps status` shows
its progress on the `dhis2` line. Then:

```sh
chaps dhis2 connect
chaps open dhis2
```

`chaps dhis2 connect` waits for DHIS2 to finish starting, then takes about a
minute: it connects DHIS2 to CHAP, installs
the Modeling App and the Climate App, and prepares the data the Modeling App
reads. It worked when DHIS2 opens in the browser, the login `admin` /
`district` works, and typing `Modeling` in the app menu (the grid icon at the
top right) finds **Modeling**.

Next: [Your first forecast in the Modeling App](./modeling-app.md), which takes
them from here to an evaluation and a forecast in about fifteen minutes.

More: [CHAP with a local DHIS2](./use-cases/chap-with-local-dhis2.md).

### 2. CHAP with forecasting models

chap-core and one model, without DHIS2: less memory, but no user interface
beyond chap-core's API page.

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps status
```

It worked when `chaps status` says chap-core is `up` and the last line says the
models are registered. Models can take a minute or two after `chaps up`: if
some say `not registered`, wait a minute and run `chaps status` again. Then
`chaps open chap-core` opens chap-core's API page in the browser.

Next: [What to try next without DHIS2](#what-to-try-next-without-dhis2).

More: [CHAP with forecasting models](./use-cases/chap-with-models.md).

### 3. CHAP with climate data

```sh
chaps init mychap --models default --with ocs,s3
cd mychap
chaps up
chaps status
```

It worked when `chaps status` shows chap-core and `ocs` as `up`. Some climate
datasets need an account with Copernicus or the Earth Data Hub; the keys go in
the `.env` file in the folder, where they are already listed, commented out.
Add `,dhis2` to `--with` for the Modeling App as well (8 GB of memory), then
continue as in option 1 from `chaps dhis2 connect`.

More: [CHAP with climate data from OCS](./use-cases/chap-with-ocs.md).

### 4. CHAP with every model

Like option 2, with every marketplace model. The first start pulls large
images and the checks take several minutes.

```sh
chaps init mychap --models all
cd mychap
chaps up
chaps status
chaps models test --all
```

It worked when `chaps status` ends with every model registered and
`chaps models test --all` says every model passes. Add `--with dhis2` to the
`chaps init` line to compare them in the Modeling App (8 GB of memory, then as
in option 1 from `chaps dhis2 connect`).

More: [CHAP with forecasting models](./use-cases/chap-with-models.md#every-model-in-the-marketplace).

### 5. CHAP for a DHIS2 that already exists

The person needs the DHIS2 address, an admin account on it, and an address at
which that DHIS2 can reach this computer (not `localhost`).

```sh
chaps init mychap --models default --api-token
cd mychap
chaps up
chaps dhis2 use https://THEIR-DHIS2 --chap-url https://ADDRESS-OF-THIS-MACHINE
chaps dhis2 connect
```

It worked when `chaps dhis2 connect` finishes without `error:`. It only sets
the route on their DHIS2: its `skipped:` lines name the apps to install and
the analytics run, which are for whoever runs that DHIS2 to decide (`chaps dhis2
apps`, `chaps dhis2 analytics`). The first time, it usually stops and says it
has no credentials for that DHIS2: the fix is to
put `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` (or `DHIS2_API_TOKEN`) in
the `.env` file in the folder, then run `chaps dhis2 connect` again. If
`--chap-url` is an `http://` address and the DHIS2 is 2.42 or later, it may say
`DHIS2 refused the route`: whoever runs that DHIS2 has to add the address to
`route.remote_servers_allowed` in its `dhis.conf` and restart it.

More: [CHAP for a DHIS2 that runs elsewhere](./use-cases/chap-for-external-dhis2.md).

### 6. Climate data only

```sh
chaps init climate --only ocs,s3
cd climate
chaps up
chaps status
chaps open ocs
```

It worked when `chaps status` shows `ocs` as `up` and the browser opens OCS.

To put some data in (public rainfall data, no account needed; on Windows run
it in Git Bash or WSL, where the quotes work as written):

```sh
curl -X POST http://localhost:8790/ingestions -H 'Content-Type: application/json' -d '{"dataset_id": "chirps3_precipitation_daily", "start": "2024-01-01", "end": "2024-01-03", "publish": true}'
```

It worked when the answer contains `"status":"completed"`.

More: [An OCS server on its own](./use-cases/ocs-alone.md).

### 7. One forecasting model only

```sh
chaps init onemodel --only none --models chapkit_ewars_model
cd onemodel
chaps up
chaps status
```

It worked when `chaps status` shows the model as `up`. Its API is at the
port in the REACH column, usually `http://localhost:5001`, and
`chaps open chapkit_ewars_model` opens its documentation. Other models:
`chaps models list` shows their ids.

More: [A chapkit model service on its own](./use-cases/model-alone.md).

### 8. DHIS2 only

Needs 8 GB of memory for Docker. The first start takes several minutes.

```sh
chaps init dhis --only dhis2
cd dhis
chaps up
chaps status
```

Run `chaps status` every minute until the `dhis2` line says `up`, then
`chaps open dhis2`. It worked when the login `admin` / `district` works. This
is DHIS2 2.42 with the Laos demo data; for another version, see option 9.

More: [A DHIS2 on its own](./use-cases/dhis2-alone.md).

### 9. A particular DHIS2 version

Needs 8 GB of memory for Docker. Each version starts with an empty DHIS2
(only 2.42 has demo data, see option 8). Run one of these, not all three:
2.43,

```sh
chaps init dhis --only dhis2 --dhis2-tag 2.43
```

2.41,

```sh
chaps init dhis --only dhis2 --dhis2-tag 2.41
```

or the next DHIS2, not released yet:

```sh
chaps init dhis --only dhis2 --dhis2-image dhis2/core-dev --dhis2-tag master
```

Then:

```sh
cd dhis
chaps up
chaps status
```

Run `chaps status` every minute until the `dhis2` line says `up`, then
`chaps open dhis2`. It worked when the login `admin` / `district` works. To
have CHAP with it, use option 1 with the same `--dhis2-tag` (and
`--dhis2-image`) added to its `chaps init` line; without demo data there is
nothing to forecast until they import some.

More: [A DHIS2 on its own](./use-cases/dhis2-alone.md#picking-the-version).

### 10. Climate data and DHIS2, no CHAP

Needs 8 GB of memory for Docker.

```sh
chaps init lab --only ocs,s3,dhis2
cd lab
chaps up
chaps status
```

Run `chaps status` every minute until every line says `up`, then
`chaps open ocs` and `chaps open dhis2`. It worked when both open, and the
DHIS2 login `admin` / `district` works.

More: [Combinations](./use-cases/combinations.md#climate-data-and-a-dhis2-no-chap).

### 11. My own model

The model has to be a chapkit model whose GitHub repository publishes a public
image on ghcr.io (most do, through their publish workflow). If `chaps models
add` says the repository `publishes no public image`, use option 13 instead:
build the image from their checkout and add that.

```sh
chaps init mymodel --only none
cd mymodel
chaps models add https://github.com/THEIR-ORG/THEIR-MODEL
chaps up
chaps status
```

It worked when `chaps status` shows the model as `up`, on the port in its
REACH column.

More: [Models outside the marketplace](./models.md#models-outside-the-marketplace).

### 12. My own model, while I work on it

For someone developing a chapkit model in its folder. chaps runs chap-core;
they run the model from the folder.

```sh
chaps init mychap --models none
cd mychap
chaps up
```

Then, in the model's folder (keep the single quotes):

```sh
export SERVICEKIT_ORCHESTRATOR_URL='http://localhost:8700/v2/services/$register'
export SERVICEKIT_HOST=host.docker.internal
export SERVICEKIT_PORT=8001
uv run uvicorn main:app --host 0.0.0.0 --port 8001
```

`main:app` is right for a model with `main.py` at the top of its folder, which
is how chapkit models are laid out. If `main.py` is inside a package folder
instead, use `PACKAGE.main:app`.

It worked when `chaps status`, run in the `mychap` folder, lists the model with
the state `unmanaged` and a LAST PING of a few seconds (unmanaged because chaps
did not start it, which is expected).

More: [Your model from its checkout, with CHAP](./use-cases/model-on-host.md).

### 13. A model image I built

In the model's folder:

```sh
docker build --platform linux/amd64 -t my-model:dev .
```

Then:

```sh
chaps init mychap --models none
cd mychap
chaps models add my-model:dev
chaps up
chaps status
```

It worked when `chaps status` lists the model as `registered`. Usually it does
not the first time: the image registers under its own name, so `chaps status`
shows it `unmanaged` next to `my-model` not registered, and the line under the
table gives the two commands that fix it (`chaps models remove my_model`, then
`chaps models add my-model:dev --service-id` with the name from the
`unmanaged` row). Run them, then `chaps up` again.

More: [A model image you built yourself](./use-cases/local-model-image.md).

### 14. My own chap-core, with the models

For someone developing chap-core itself. They run chap-core from its folder,
listening on `0.0.0.0:8000`; chaps runs the models. Have them start their
chap-core first: `chaps init` then sees whether it runs in Docker and sets the
models up to match.

```sh
chaps init models --chap-core-url http://localhost:8000 --models chapkit_simple_multistep_model
cd models
chaps up
chaps status
```

It worked when `chaps status` shows chap-core `up` and the model `registered`.
If chap-core was started after `chaps up`, wait a few seconds and run
`chaps status` again. `registered, unreachable` means chap-core cannot call the
model back, usually because chap-core was not running during `chaps init`: the
line under the table gives the command that fixes it.

More: [chap-core from its checkout, with the models](./use-cases/chap-core-on-host.md).

### 15. chap-core built from my checkout

For someone changing chap-core who wants to run their version with models.
Replace the path with where they cloned chap-core:

```sh
chaps init mychap --source ~/dev/chap-core --models default
cd mychap
chaps up
chaps status
```

The first `chaps up` builds chap-core and takes several minutes (longer on an
Apple Silicon Mac). It worked when `chaps status` shows chap-core `up` and the
models registered. After changing the code, `chaps up` again rebuilds it.

More: [chap-core built from its checkout](./use-cases/chap-core-from-checkout.md).

### 16. My own DHIS2, with CHAP

For someone who already runs DHIS2 on this machine (for DHIS2 or app work).
Ask whether their DHIS2 runs in Docker. If it does not:

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps dhis2 use http://localhost:8080 --chap-url http://localhost:8700
chaps dhis2 connect
```

If it runs in Docker, it reaches CHAP through `host.docker.internal`:

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps dhis2 use http://localhost:8080 --chap-url http://host.docker.internal:8700
chaps dhis2 connect
```

It worked when `chaps dhis2 connect` finishes without `error:`. If it says it
has no credentials, put `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` (or
`DHIS2_API_TOKEN`) in the `.env` file and run it again. On a DHIS2 chaps did not start it sets only
the route: if its `skipped:` lines say an app is missing, `chaps dhis2 apps`
installs it, and `chaps dhis2 analytics` generates the tables the Modeling App
reads.

On DHIS2 2.42 and later it can then say `DHIS2 refused the route`: their
DHIS2 only proxies to the addresses its own `dhis.conf` allows. Tell them to
add the `--chap-url` address (for example `http://host.docker.internal:8700`)
to the `route.remote_servers_allowed` line in their DHIS2's `dhis.conf`,
restart their DHIS2, and run `chaps dhis2 connect` again.

More: [A DHIS2 you run yourself, with CHAP from chaps](./use-cases/dhis2-dev-with-chap.md).

### 17. A DHIS2, with my own chap-core

For someone developing chap-core who wants DHIS2 and the Modeling App in front
of it. Their chap-core must listen on `0.0.0.0:8000`. Needs 8 GB of
memory for Docker.

```sh
chaps init lab --chap-core-url http://localhost:8000 --with dhis2
cd lab
chaps up
chaps status
```

Then, once `chaps status` shows chap-core `up` (`chaps dhis2 connect` waits
for DHIS2 by itself):

```sh
chaps dhis2 connect
chaps open dhis2
```

It worked when `chaps dhis2 connect` finishes without `error:` and the login
`admin` / `district` works.

More: [A DHIS2 from chaps, with a chap-core elsewhere](./use-cases/dhis2-with-chap-core-elsewhere.md).

### 18. Two of these at the same time

Every option uses the same ports, so a second one needs its own. Create the
second one with these added to its `chaps init` line, one flag per piece it
has (`--ocs-port` only with OCS, `--dhis2-port` only with DHIS2; a flag for a
missing piece is an error):

| Piece | Flag |
| --- | --- |
| chap-core | `--api-port 8701` |
| models | `--port-base 5101` |
| OCS | `--ocs-port 8791` |
| DHIS2 | `--dhis2-port 8781` |

For example, a second option 2: `chaps init second --models default --api-port
8701 --port-base 5101`.

For a third, use 8702, 8792, 8782 and 5201, and so on. It worked when both
folders' `chaps status` say `up`. Running them one at a time needs none of
this; see the note below.

More: [Several deployments on one machine](./use-cases/several-deployments.md).

## What to try next without DHIS2

Option 2 has no user interface, but the same evaluation the Modeling App runs
works from the terminal. In the `mychap` folder:

```sh
chaps models test --all --backtest
chaps jobs
```

The first line makes every enabled model train, forecast and run an evaluation
through chap-core on sample data the model generates itself, and prints the
scores. For CHAP-EWARS it takes under a minute. `chaps jobs` lists what
chap-core did. To see it in the Modeling App instead, add DHIS2 to the same
folder (8 GB of memory for Docker):

```sh
chaps components enable dhis2
chaps up
```

then continue as in option 1 from `chaps dhis2 connect`. See
[Growing a deployment](./use-cases/growing.md).

## Every option: stop, start, remove

```sh
chaps down                    # stop everything; the data is kept
chaps up                      # start it again
chaps down --volumes --yes    # stop and delete all the data
```

After `chaps down --volumes --yes` the folder can be deleted.

Two options use the same ports, so only one runs at a time. When `chaps up`
says a port is used by another deployment and asks whether to stop it, answer
`y`: the other one keeps its data, and `chaps up` in its folder starts it
again; `chaps up --replace` is the same `y` given in advance. While the other one is
up, `chaps status` in this folder says this deployment's chap-core is not
running and names the one that is.
