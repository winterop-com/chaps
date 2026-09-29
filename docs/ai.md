# For AI assistants

This page is written for an AI assistant helping someone set something up with
`chaps`. If a person pointed you here, follow the steps below in order. Keep
your answers short and concrete, and give one command at a time.

## How to help

1. **Offer the menu.** Show the person the numbered options in
   [The options](#the-options), each with its one-line description, and ask
   which one they want. Do not explain Docker, Compose or chap-core unless they
   ask. If they are unsure, recommend option 1.
2. **Check the machine** with [Before any option](#before-any-option).
3. **Give the commands for their option**, in order, one block at a time. Wait
   for them to say it worked before giving the next block. Every option has an
   "It worked when" line: use it to decide.
4. **If something fails**, ask them to paste the exact output. Then look for the
   first line that starts with `error:` or `warning:`. chaps puts the fix on the
   same line, usually a command in backticks: tell them to run that command.
   If that does not help, search [Troubleshooting](./troubleshooting.md) for
   the exact message.
5. **Never** tell them to edit the `compose*.yml` files by hand (chaps rewrites
   them), and never ask them to paste the `.env` file (it holds passwords).

When an option has a "More" link, that page has the details. You only need it
if the person asks for something the steps here do not cover.

## Before any option

The person needs Docker: Docker Desktop on macOS and Windows, Docker Engine on
Linux. The options with DHIS2 need more memory than the others; DHIS2 alone
uses 4 to 5 GB, so give Docker about 8 GB or more for those. Then:

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps doctor
```

On Windows, download `chaps-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/winterop-com/chaps/releases/latest), unzip
it, and run `chaps.exe doctor` from that folder. See [Install](./install.md).

It worked when `chaps doctor` shows no `fail` lines. On an Apple Silicon Mac a
`warn` line about `linux/amd64` and emulation is normal and can be ignored.

Every option below creates a new folder. Tell the person to run the commands
from the folder where they want it created, and then to `cd` into it as shown.

## The options

| # | Option | Pick this if the person wants to... |
| --- | --- | --- |
| 1 | [CHAP with forecasting models](#1-chap-with-forecasting-models) | try CHAP on their own computer |
| 2 | [CHAP with a DHIS2 to click around in](#2-chap-with-a-dhis2-to-click-around-in) | see CHAP inside DHIS2 (the Modeling App) |
| 3 | [CHAP with climate data](#3-chap-with-climate-data) | use CHAP together with Open Climate Service |
| 4 | [Climate data only](#4-climate-data-only) | run Open Climate Service, no CHAP |
| 5 | [One forecasting model only](#5-one-forecasting-model-only) | call one model's API directly, no CHAP |
| 6 | [DHIS2 only](#6-dhis2-only) | have a DHIS2 to play with, no CHAP |
| 7 | [CHAP for a DHIS2 that already exists](#7-chap-for-a-dhis2-that-already-exists) | connect CHAP to a DHIS2 someone else runs |
| 8 | [My own model](#8-my-own-model) | run a model they wrote themselves |

### 1. CHAP with forecasting models

```sh
chaps init mychap --models default
cd mychap
chaps up
chaps status
```

It worked when `chaps status` says chap-core is `up` and the last line says the
models are registered. Models can take a minute or two after `chaps up`: if
some say `not registered`, wait a minute and run `chaps status` again. Then
`chaps open chap-core` opens chap-core in the browser.

More: [CHAP with forecasting models](./use-cases/chap-with-models.md).

### 2. CHAP with a DHIS2 to click around in

Needs about 8 GB of memory for Docker. The first start takes several minutes.

```sh
chaps init mychap --models default --with dhis2
cd mychap
chaps up
chaps status
```

Run `chaps status` every minute until the `dhis2` line says `up`. Then:

```sh
chaps dhis2 connect
chaps open dhis2
```

It worked when DHIS2 opens in the browser and the login `admin` / `district`
works. The Modeling App is in the DHIS2 app menu.

More: [CHAP with a local DHIS2](./use-cases/chap-with-local-dhis2.md).

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

More: [CHAP with climate data from OCS](./use-cases/chap-with-ocs.md).

### 4. Climate data only

```sh
chaps init climate --only ocs,s3
cd climate
chaps up
chaps status
chaps open ocs
```

It worked when `chaps status` shows `ocs` as `up` and the browser opens OCS.

More: [An OCS server on its own](./use-cases/ocs-alone.md).

### 5. One forecasting model only

```sh
chaps init onemodel --only none --models chapkit_ewars_model
cd onemodel
chaps up
chaps status
```

It worked when `chaps status` shows the model as `up`. Its API is at the
address in the REACH column, usually `http://localhost:5001`, and its
documentation at `http://localhost:5001/docs`. Other models:
`chaps models list` shows their ids.

More: [A chapkit model service on its own](./use-cases/model-alone.md).

### 6. DHIS2 only

Needs about 8 GB of memory for Docker. The first start takes several minutes.

```sh
chaps init dhis --only dhis2
cd dhis
chaps up
chaps status
```

Run `chaps status` every minute until the `dhis2` line says `up`, then
`chaps open dhis2`. It worked when the login `admin` / `district` works.

More: [A DHIS2 on its own](./use-cases/dhis2-alone.md).

### 7. CHAP for a DHIS2 that already exists

The person needs the DHIS2 address, an admin account on it, and an address at
which that DHIS2 can reach this computer (not `localhost`).

```sh
chaps init mychap --models default --api-token
cd mychap
chaps up
chaps dhis2 use https://THEIR-DHIS2 --chap-url https://ADDRESS-OF-THIS-MACHINE
chaps dhis2 connect
```

It worked when `chaps dhis2 connect` finishes without `error:`. The first time,
it usually stops and says it has no credentials for that DHIS2: the fix is to
put `DHIS2_ADMIN_USERNAME` and `DHIS2_ADMIN_PASSWORD` (or `DHIS2_API_TOKEN`) in
the `.env` file in the folder, then run `chaps dhis2 connect` again.

More: [CHAP for a DHIS2 that runs elsewhere](./use-cases/chap-for-external-dhis2.md).

### 8. My own model

The model has to be a chapkit model, either in a GitHub repository or published
as an image on ghcr.io.

```sh
chaps init mymodel --only none
cd mymodel
chaps models add https://github.com/THEIR-ORG/THEIR-MODEL
chaps up
chaps status
```

It worked when `chaps status` shows the model as `up`, at the address in its
REACH column.

More: [Models outside the marketplace](./models.md#models-outside-the-marketplace).

## Every option: stop, start, remove

```sh
chaps down                    # stop everything; the data is kept
chaps up                      # start it again
chaps down --volumes --yes    # stop and delete all the data
```

After `chaps down --volumes --yes` the folder can be deleted.
