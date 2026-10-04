---
marp: true
theme: chaps
paginate: true
footer: chaps - DHIS2, the Modeling App and Chap
title: Chap behind the Modeling App
description: For people who use DHIS2 and the Modeling App: what runs behind it, and how chaps sets it up
---

<!-- _class: title -->
<!-- _paginate: false -->
<!-- _footer: '' -->

# Chap behind the Modeling App

## A DHIS2, the apps and Chap, connected with one command

For people who use DHIS2 and the Modeling App

---

## What you see, and what runs behind it

<div class="columns">
<div>

**In DHIS2:**

- the Modeling App: evaluate a model, make a forecast
- the Climate App: climate data for your org units
- analytics: the data the apps read

</div>
<div>

**Behind it:**

- **chap-core**: runs the evaluations and forecasts
- **models**: one container each (EWARS, ARIMA, ...)
- **OCS**: climate data, if you want it

</div>
</div>

The Modeling App reaches chap-core through a **route** in DHIS2. chap-core
never calls DHIS2, and DHIS2 never calls chap-core.

---

## Three things must be correct

| Without it | What the Modeling App does |
| --- | --- |
| the `chap` route | sends you to "get started"; it cannot see Chap |
| the analytics tables | has no data to send, and says nothing about why |
| the apps | there is no Chap screen in DHIS2 |

Before chaps, you did each of these by hand: a route JSON, the App Hub, an
analytics run.

---

## A DHIS2 with Chap, from nothing

```sh
chaps init mychap --with dhis2 --models default
cd mychap
chaps up
chaps dhis2 connect
```

- `--with dhis2` adds a DHIS2, with the Laos climate demo (DHIS2 2.42).
- `chaps dhis2 connect` makes the route, installs the apps and generates
  analytics.

Open **http://localhost:8780** and log in as `admin` / `district`.

> **The first `chaps up` downloads about 25 GB** of images: chap-core, the
> model and DHIS2. Do it on a fast network.

---

## The first start takes minutes

DHIS2 migrates its schema before it answers. The demo database is restored
before that.

| Shape | About |
| --- | --- |
| A native image, empty database | 40 seconds |
| Under emulation (an amd64 image on arm64) | 8 to 15 minutes |
| With the demo database | the above, plus the restore |

- `chaps status` shows each service. `chaps logs dhis2` shows the migration.
- DHIS2 needs **4 to 5 GB of memory** for analytics. `chaps doctor` warns
  below 6 GB for docker. On Docker Desktop, raise the VM's memory.

---

## `chaps dhis2 connect`

Three steps, in this order:

1. **The route.** It creates the `chap` route, or repoints one that points
   elsewhere. Then it sends a request through it, to prove that chap-core
   answers.
2. **The apps.** It installs the Modeling App and the Climate App from the
   App Hub, at the newest version that this DHIS2 can run.
3. **Analytics.** It generates the analytics tables, and waits for the run.

- Each step checks first, and does nothing when it is correct already.
- A second run repoints nothing and reinstalls nothing, and says so.
- `chaps dhis2 show` asks the same questions and changes nothing.

---

## A DHIS2 that runs elsewhere

Your DHIS2 runs on a server already. Chap runs on another machine:

```sh
chaps init chap-server --models default --api-token
cd chap-server && chaps up
chaps dhis2 use https://dhis2.example.org --chap-url https://chap.example.org
chaps dhis2 connect
```

- `use` records the DHIS2, and checks that it answers and accepts the login.
- On a DHIS2 that runs elsewhere, `connect` makes only the route. The apps
  and analytics stay with its admin.
- `--api-token` protects chap-core. The route carries the token.

---

## Which DHIS2 version

```sh
chaps init mychap --with dhis2 --dhis2-tag 2.41
chaps init mychap --with dhis2 --dhis2-tag 2.43 --dhis2-seed none
```

- `--dhis2-seed none` starts with an empty database.
- `--dhis2-seed FILE` or `URL` restores your own dump.
- DHIS2 migrates **forward only**. Before a move to a newer version, run
  `chaps backup create`.

---

## More models, more data

```sh
chaps models list                       # the marketplace
chaps models enable auto_arima_chapkit  # one more model
chaps components enable ocs             # Open Climate Service
chaps up
```

A new model registers with chap-core, and the Modeling App lists it at once.

---

## When something does not work

```sh
chaps doctor        # docker, ports, memory, pins, and whether Chap is up
chaps status        # chap-core, the models, DHIS2
chaps dhis2 show    # the route, the apps, analytics
```

Each line that is wrong names the fix. For example:

```text
warn  memory   5.0 GB available to docker
      keep at least 6.0 GB for docker: DHIS2 alone wants 4.0 GB for the
      analytics populate phase, ...
```

---

## Without DHIS2: the same evaluation from a terminal

The Modeling App runs a backtest through chap-core. You can run one without
DHIS2:

```sh
chaps models test --all --backtest   # every model, through chap-core
chaps jobs                           # what chap-core ran, and why one failed
```

Or evaluate a model on your own CSV:

```sh
chaps chap eval --model-name chapkit_ewars_model \
  --dataset-csv mydata.csv --output-file ewars.nc
```

---

## Start now

```sh
curl -fsSL https://raw.githubusercontent.com/winterop-com/chaps/main/install.sh | sh
chaps init mychap --with dhis2 --models default
cd mychap && chaps up && chaps dhis2 connect
```

The DHIS2 chapter: **https://winterop-com.github.io/chaps/dhis2.html**
