# Models and the marketplace

## Where the catalogue comes from

The catalogue is the YAML in the model-marketplace repository, read from

```text
https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml
```

`--registry-url` points somewhere else, for a fork or a mirror. `varde init
--registry-url URL` records it as `registry_url` in `.varde/project.yaml`, and
every later command in that deployment uses the recorded one; typing the flag
again overrides it for that one run. The
marketplace has no JSON API, so the index is one request and each model file it
lists is one more; the requests are sequential, which is fast enough for a
handful of small files and makes a failure easy to attribute.

Resolution order for every command that needs the catalogue:

1. a cached copy younger than 24 hours,
2. the network,
3. a stale cached copy, when the network fails,
4. the snapshot compiled into the binary.

`varde registry show` prints which of those was used:

```text
  url     https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml
  source  cache (56 minutes old)
  models  7
  cache   /home/me/.cache/varde/registries/https_raw_githubusercontent_com_dhis2_chap_model_marketplace_mai-f6e5270ab795f1bd

  chapkit_ewars_model
  chapkit_rwanda_malaria_bym_model
  chapkit_simple_multistep_model
  auto_arima_chapkit
  chapkit_ghr_model
  chapkit_minimalist_example_py
  chapkit_minimalist_example_r

7 models in the catalogue (cache (56 minutes old))
```

With `-v`, a hint names the next command:

```text
hint: `varde models info ID` describes one
```

`varde registry update` forces a fetch and refreshes the cache, as does
`varde update`. `--offline` never touches the network, so a laptop on a plane
still resolves the catalogue from the cache or the embedded snapshot;
`varde update` and `varde registry update` refuse to run that way, because a
refresh needs the network.

The cache lives in `$VARDE_CACHE_DIR`, else `$XDG_CACHE_HOME/varde`, else
`~/.cache/varde`; `--cache-dir` overrides it for one invocation. A private
marketplace and the public one never share a cache entry, because the entry is
keyed by the registry URL. Every cache read failure is a warning, never an
error: a missing, truncated or hand-edited cache just means the fetch or the
embedded snapshot answers instead.

The embedded snapshot is the last resort, so `varde` works on a machine that
has never had network access. It is refreshed with `make vendor`.

## Channels and versions

Each marketplace entry lists versions, and the channels `stable` and `latest`
each point at one of them.

```sh
varde models enable chapkit_ewars_model                       # stable, the default
varde models enable chapkit_ewars_model --channel latest
varde models enable chapkit_ewars_model --version 1.0.3       # an exact pin
```

A model that follows a channel records both the channel and the version it
resolved to, and `varde update` re-resolves the channel. A model enabled with
`--version` is listed as pinned by `update` and never moved. `--channel` and
`--version` cannot be combined.

Every version pins one commit of the model's own repository, and the image
built from it; whether that commit is still the newest build the repository
has published is what [`varde doctor`](./doctor.md#project-checks) reports as
one `registry pin <id>` line per enabled model, because a pin that lags is for
the marketplace to move and not something a deployment can fix.

## Templates

Entries with `kind: template` are scaffolding for writing your own model, not
forecasting models. They are hidden from `varde models list` and from the
browser by default, `--templates` and `--all` show them, and enabling one needs
`--allow-template`.

These are chapkit templates: the same word as in `chapkit init --template`,
which starts a new model from one of them. They are not the "model templates"
of chap-core, which is what a model service registers as.

## Looking at the catalogue

```sh
varde models list                 # models enabled in this deployment are marked
varde models list --enabled       # only this deployment's
varde models list --all           # templates too
varde models search malaria
varde models info chapkit_ewars_model
```

```text
ID                                SERVICE                           NAME                STATUS        STABLE  LATEST  PORT
chapkit_ewars_model               chapkit-ewars-model               CHAP-EWARS          limited data  1.0.4   1.0.4   via chap-core
chapkit_simple_multistep_model    chapkit-simple-multistep-model    Simple Multistep    limited data  0.1.2   0.1.2   -
auto_arima_chapkit                auto-arima-chapkit                Auto-ARIMA          experimental  1.0.2   1.0.2   -
chapkit_ghr_model                 chapkit-ghr-model                 GHRmodel            experimental  0.1.3   0.1.3   -
chapkit_rwanda_malaria_bym_model  chapkit-rwanda-malaria-bym-model  Rwanda Malaria BYM  not for use   0.1.3   0.1.3   -

5 listed, 1 enabled in this deployment
```

Every listing is in the same order: by `STATUS`, from the models that have
been validated down to the ones nobody should run, and by name inside each
step of that scale. Templates come after models when `--all` or `--templates`
asks for them, and a search narrows the catalogue without reordering it.

The `PORT` column is the host port a model publishes, `via chap-core` for one
this project enables without a host port of its own - chap-core's proxy is
then the only way in - or `-` for one this project does not enable.

## What the STATUS column means

`STATUS` is the model author's own
[chapkit](https://github.com/dhis2-chap/chapkit) assessment of the model, not
a marketplace verdict and not a measure of whether it runs. It is the one
column to read before enabling anything:

| Status | Colour | What the author is saying |
| --- | --- | --- |
| `production` | green | Validated and ready for production use. |
| `testing` | yellow | Ready for more rigorous testing on diverse data. |
| `limited data` | orange | Shows promise on limited data; needs manual configuration and careful evaluation. |
| `experimental` | red | A highly experimental prototype, not validated, only for early experimentation. |
| `not for use` | gray | Not intended for use: deprecated, or kept for backwards compatibility. |

`--json` keeps emitting the colour as `assessed_status`, so a script that
matches on `orange` goes on working, and adds the words above as
`assessed_status_label`. `varde models info ID` prints the colour and the
whole sentence:

```text
  status        orange, shows promise on limited data, needs manual
                configuration and careful evaluation
```

`varde models info ID` prints everything the marketplace says about one entry:
its image and runtime, the period types and forecast horizon it supports, the
covariates it requires and defaults to, the channels and the version table,
the maintainers and the attribution, and any named configurations. For a model
this project has enabled with no host port, it also prints the chap-core proxy
URL that reaches it.

Both `ID` forms work everywhere an id is taken: the marketplace id
(`chapkit_ewars_model`, with underscores) and the service id
(`chapkit-ewars-model`, with hyphens, which is also the Compose service name
and the DNS name).

## Enabling and disabling

```sh
varde models enable ID [--channel stable|latest | --version X]
                       [--port N|auto] [--bind ADDR] [--data-dir PATH]
                       [--user USER:GROUP] [--allow-template]
varde models disable ID [--purge]
varde models expose ID [--port N|auto] [--bind ADDR]
varde models unexpose ID
varde up                            # apply any of them
```

`--bind ADDR` sets the host address of the model port; see
[Which address a model port is published on](./ports.md#which-address-a-model-port-is-published-on).

`enable` and `disable` edit `.varde/models.yaml` and then re-render the compose
files; `expose` and `unexpose` only rewrite the overlay's `ports:`, so they
never re-resolve the version and are safe on a deployment running a build you
do not want moved. The browser is the same: moving a port there keeps the
version the model is pinned to, and only a toggle or a new channel resolves the
marketplace again.

`disable` also stops and removes the model's container when one is running,
before the definition goes away, and says so:

```text
disabled chapkit_ewars_model
stopped and removed 2 containers (chapkit-ewars-model, chapkit-ewars-model-init); host port 5001 is free again
kept volume mychap-1ab2c3_ck_chapkit_ewars_model_data; remove it with `varde models disable chapkit_ewars_model --purge` or `docker volume rm mychap-1ab2c3_ck_chapkit_ewars_model_data`
```

The port is therefore free straight away, rather than at the next `varde up`,
which would have removed the container as an orphan but only once it was run.
There is nothing left for `varde up` to apply. With `-v`, two hints follow:
the overlay that was removed (`removed compose.chapkit-ewars-model.yml`) and
`` `varde status` shows what runs now ``.

### The data volume

What `disable` does not take is the model's data: the named volume
`<compose project>_ck_<id>_data` stays exactly as it was, so enabling the model
again finds everything it had. That is why the line above names it - when
docker has it: a model that was never started has no volume, and then there
is no line, because nothing was kept. Nothing
else does any more - the overlay that declared the volume has just been
removed, so `varde down --volumes` no longer knows about it, and a volume
nobody names again is kept for as long as the machine lasts.

`--purge` is how the data goes with the model:

```sh
varde models disable chapkit_ewars_model --purge
```

```text
disabled chapkit_ewars_model
stopped and removed 2 containers (chapkit-ewars-model, chapkit-ewars-model-init)
removed volume mychap-1ab2c3_ck_chapkit_ewars_model_data
```

The volume is removed after the container, which is the only order docker
allows: a volume a container still has mounted cannot be removed. Everything
about it is best-effort and reported rather than fatal - `volume <name> not
found` for a deployment that was never started, and `volume <name> could not
be removed: <docker's own words>` for anything else. `--json` carries the same
answer as two lists, `purged` and `kept_volumes`.

`--purge` also works on a model that is not enabled any more, which is the
state the kept-volume line leaves you in:

```text
chapkit_ewars_model is not enabled here, so only its data volume was looked for
removed volume mychap-1ab2c3_ck_chapkit_ewars_model_data
```

`varde doctor`'s `volumes` line finds the ones that were forgotten: a volume
under this deployment's name that belongs to no enabled model and no enabled
component is reported as a leftover, with the same two ways to remove it. See
[Doctor](./doctor.md).

`expose` and `unexpose` say what changed:

```text
exposed chapkit-ewars-model on http://localhost:5001
run `varde up` to apply
```

```text
unexposed chapkit-ewars-model; it stays registered with chap-core and reachable at http://localhost:8700/v2/services/chapkit-ewars-model/run/
run `varde up` to apply
```

If the model is already where the command puts it, nothing changes, and there
is no `varde up` line:

```text
chapkit-ewars-model is already exposed on http://localhost:5001; nothing changed
```

```text
chapkit-ewars-model was already unexposed; nothing changed, and it is reachable at http://localhost:8700/v2/services/chapkit-ewars-model/run/
```

An `enable` of a model that is already enabled at the same version and port
also changes nothing:

```text
chapkit_ewars_model v1.0.4 is already enabled at http://localhost:8700/v2/services/chapkit-ewars-model/run/; nothing changed
```

With `--json`, such a model is in the `unchanged` list, not in `enabled`.

## Configured models

chap-core does not run a model service directly. It runs a **configured
model**: a model template together with a set of option values and
covariates. A backtest, a prediction and the model picker of the Modeling App
all name a configured model.

Each configured model has a name, the variant name. The Modeling App shows it
in brackets after the model name: `<model name> [<variant name>]`. chap-core
stores it after the template: `<service id>:<variant name>`, and the bare
service id for the variant name `default`.

`varde models configs` is the command group for the configured models:

| Command | What it does |
| --- | --- |
| `varde models configs [list] [ID]` | Lists the configured models that chap-core has. |
| `varde models configs add ID` | Adds a configured model. |
| `varde models configs update ID NAME` | Gives a configured model new values. |
| `varde models configs archive ID NAME` | Archives a configured model. |
| `varde models configs export [ID]` | Writes the configured models in the marketplace format. |
| `varde models configs sync [ID..]` | Creates the configured models of the marketplace entry. |

In each command, `ID` is the marketplace id or the service id of an enabled
model, and `NAME` is the variant name. Each command asks chap-core, so
chap-core must run. If chap-core does not answer, the command stops and names
`varde status`.

### Listing

```sh
varde models configs                           # every enabled model
varde models configs list chapkit_ewars_model  # one model
varde models configs --all                     # with the archived ones
```

```text
chapkit_ewars_model
NAME                     COVARIATES                 OPTIONS                                          SOURCE
monthly_climate          rainfall,mean_temperature  n_lags=3,3 precision=0.01 region_seasonal=false  marketplace
monthly_population_only  -                          n_lags=3 precision=0.01 region_seasonal=false    marketplace
monthly_region_seasonal  rainfall,mean_temperature  n_lags=3,3 precision=0.01 region_seasonal=true   marketplace
short_lags               rainfall                   n_lags=2,2 precision=0.05                        by hand

4 configured models of 1 model
```

Each model gets a table. NAME is the variant name. OPTIONS shows the option
values that chap-core stores; an option without a value uses the model
default. SOURCE says where the configured model comes from:

- `marketplace`: a configuration of the marketplace entry.
- `service default`: the configuration `default` of a model without a
  marketplace entry.
- `by hand`: a configured model that you added.
- `models test`: a configured model that a test left.

`--all` adds the archived configured models and the column ARCHIVED. `--json`
gives every field, with the option values whole. If chap-core has no
configured model of a model, the line names the way out:

```text
chapkit_ewars_model: chap-core has no configured model of it; `varde models configs sync chapkit_ewars_model` creates them
```

### Adding

```sh
varde models configs add chapkit_ewars_model --name short_lags \
  --set n_lags=2,2 --set precision=0.05 --covariates rainfall
```

```text
created configured model short_lags of chapkit_ewars_model
```

- `--name` is the variant name.
- `--set KEY=VALUE` gives the value of one option. Repeat it for more than
  one. Write a list with commas, or as a JSON list. The booleans are `true`
  and `false`. `null` is a value only where the option allows it.
- `--covariates` gives the additional covariates, separated by commas.

varde checks each key against the options of the template, and each value
against the kind of the option, before it asks chap-core. chap-core does not
check the values of a chapkit model, and chapkit accepts keys that it does
not know. So a mistake would otherwise show only when a run fails. A bad key
or value stops the command with exit code 2:

```text
error: `lag` is not an option of chapkit_ewars_model; its options are label (text or null), max_lag (integer), method (one of fast, exact), n_lags (list of integers), precision (number), region_seasonal (true or false), seasonal (true or false)
error: `two` is not a value of `max_lag`, which takes an integer, such as `3`
```

varde also checks the covariates. A required covariate is refused, because
every run gets it. A model that does not allow free covariates takes only the
defaults of its marketplace entry.

varde refuses a variant name that a configured model has and that is not
archived. chap-core would show the new configured model and hide the old one,
and give no message.

With no options at a terminal, `add` opens the form of `varde ui`: one field
for the variant name, one field for each option with its kind, default and
description, and one field for the covariates. An empty field uses the
default. Esc closes the form and writes nothing:

```text
left the form; nothing was changed
```

Without a terminal (in a pipe or with `--json`), `add` with no options stops
with exit code 2 and names `--name` and `--set`.

`--from FILE` adds every configuration in a file of the marketplace format:
an [export](#exporting), or a complete marketplace entry. varde checks all of
them before it creates the first. chap-core sets `prediction_periods` for
each run, so varde does not store the value in the file.

### Changing

```sh
varde models configs update chapkit_ewars_model short_lags --set precision=0.1
varde models configs update chapkit_ewars_model short_lags --unset precision
varde models configs update chapkit_ewars_model short_lags   # the form
```

```text
updated configured model short_lags of chapkit_ewars_model; chap-core keeps the old values as an archived configured model
```

chap-core has no request that changes a configured model. varde creates a
configured model with the same variant name and the new values, then archives
the old one. chap-core keeps the old row for the backtests and predictions
that use it, but it does not list it again, so `list --all` does not show the
old values.

- `--set` gives a new value. The other values stay.
- `--unset KEY` removes a value, so the model default applies.
- `--covariates` replaces the old list.

With no options at a terminal, `update` opens the form with the current
values. If the new values are the same as the current values, varde writes
nothing:

```text
configured model short_lags of chapkit_ewars_model has these values already; nothing changed
```

If the archive step fails, the new values are live, and a warning gives the
`varde api DELETE` command that archives the old row.

### Archiving

```sh
varde models configs archive chapkit_ewars_model short_lags
```

```text
archived configured model short_lags of chapkit_ewars_model
```

chap-core does not delete a configured model. It archives it: chap-core
keeps the row for the backtests and predictions that use it, `list --all`
shows it, and the Modeling App shows it as Archived. If you add the same
variant name with the same values again, chap-core makes the archived row
live again.

You can also archive a configuration of the marketplace entry. `varde models
configs sync` creates the marketplace configurations of a model only when
chap-core has no live configured model of the version the model runs. While
another one is live, `sync` does not change the model. When you archive the
last one, varde says so:

```text
chapkit_ewars_model has no other configured model now, so `varde models configs sync` and `varde up --wait` create its configured models again
```

### Exporting

```sh
varde models configs export chapkit_ewars_model > configurations.yaml
varde models configs export chapkit_ewars_model --out configurations.yaml
```

The output is the `configurations:` block of a marketplace entry. For each
configured model it has the variant name, a `description` and a `config`.
`config` is the flat object that chapkit takes: the option values,
`additional_continuous_covariates` and `prediction_periods`.

```yaml
configurations:
  short_lags:
    description: The configured model short_lags of chapkit_ewars_model.
    config:
      additional_continuous_covariates:
      - rainfall
      n_lags:
      - 2
      - 2
      precision: 0.05
      prediction_periods: 3
```

chap-core does not store `prediction_periods`. varde takes it from the
marketplace configuration with the same name. If there is none, varde takes
the default from the config schema of the service, and else the chapkit
default of 3. `-v` says which source each value comes from. The keys of
`config` are in alphabetical order.

An export is for one model, because the block goes under one marketplace
entry. If the deployment runs more than one model, give the ID. With `--out`,
varde reports the file:

```text
wrote 4 configurations of chapkit_ewars_model to configurations.yaml
```

To use the configurations in another deployment, run `varde models configs
add ID --from configurations.yaml` there. To publish them, put the block in
the marketplace entry of the model.

### Sync

Up to chap-core 2.3, a registration made a configured model. chap-core 2.4
and later makes none. A model that varde starts registers, and without a
configured model nothing can run it.

`varde models configs sync` does what `chap-admin install` does after a model
service has registered:

1. It stores the model template from the service
   (`POST /v1/crud/model-templates/from-service`).
2. It creates one configured model for each entry under `configurations` in
   the marketplace entry of the model. The option values come from the
   `config` of that entry. The covariates come from the config, or from
   `covariates.defaults` when the config has none. `prediction_periods` is
   removed, because chap-core sets it for each run.
3. A marketplace entry without configurations gets one configured model,
   `default`. A model added with `varde models add`, or a model that runs an
   image other than its marketplace entry, also gets one `default` with the
   defaults of the service.

```sh
varde models configs sync                      # every model this deployment enables
varde models configs sync chapkit_ewars_model  # one model, by marketplace id or service id
```

```text
chapkit_ewars_model: created configured model monthly_climate
chapkit_ewars_model: created configured model monthly_population_only
chapkit_ewars_model: created configured model monthly_region_seasonal
chapkit_ghr_model: chap-core has a configured model of it already
```

The command is idempotent for each registered version. A model that has a
configured model of the version it registered with is left alone, so a second
run adds nothing. An older chap-core that still makes configured models itself
gets no second set. A model that chap-core has not registered yet gets a line
that says so. Directly after `varde up`, expect this line for a few seconds:

```text
chapkit_ewars_model: not registered with chap-core; run `varde models configs sync` again once `varde status` shows it registered
```

A model that could not be configured gets a warning with the reason, and the
command exits non-zero.

You do not usually run the command yourself. `varde up --wait`, `varde dhis2
connect` and `varde models test --backtest` do the same step, and say which
configured models they created:

```text
created configured models in chap-core for chapkit_simple_multistep_model (monthly_climate, monthly_selfhistory)
```

`varde status` shows a model without a configured model as `registered, not
configured`, and adds a line with the way out:

```text
auto-arima-chapkit: chap-core has no configured model for it, so nothing can run it; run `varde models configs sync`
```

### Testing a configuration

`varde models test --backtest` runs a backtest of one configured model of each
model. To test a configured model that you added, give its variant name with
`--config`:

```sh
varde models configs add chapkit_ewars_model --name short_lags --set n_lags=2,2
varde models test chapkit_ewars_model --backtest --config short_lags
```

```text
testing 1 model (through chap-core: a dataset, a backtest and its scores)
chapkit-ewars-model    pass   31s   crps 4.9  mae 7.0  rmse 10.2
  configured model: short_lags (id 9)

1 of 1 model passes
```

Without `--config`, the backtest takes the configured model `default`, or else
the lowest-numbered one. See [The backtest level](#the-backtest-level). The
line under the row always names the configured model that it used.

`--config` counts only a configured model of the version that the model
registered with, and not an archived one. If the model has no configured model
of that name, the test skips it and names the ones that it has:

```text
chapkit-ewars-model    skip    0s   chap-core has no configured model weekly for chapkit-ewars-model 1.0.1; it has monthly_climate, monthly_population_only, monthly_region_seasonal, short_lags
  run `varde models configs list chapkit_ewars_model` to see its configured models
```

With `--all` or more than one id, `--config` applies to each model. A model
that has a configured model of that name is tested with it. Each other model
is a skip with the names that it has. `--config` without `--backtest` stops
with exit code 2.

## Testing a model

`varde status` and `varde doctor` can both be entirely green while a model
cannot produce a single prediction. Registration is a heartbeat: it says the
service is alive and talking to chap-core, not that its runtime works, that the
account it runs as can write, or that the covariates it declares are the ones
it reads. `varde models test` is the check neither of them can make, because it
is the only one that makes the model do the work.

```sh
varde models test ID..                 # one or more, by marketplace id or service id
varde models test --all                # every model this deployment enables
varde models test --all --backtest     # the whole way round, through chap-core
varde models test SERVICE_ID --backtest  # a model registered from outside, such as one run from its checkout
```

The model level needs no chap-core, so it also tests a model in a deployment
without one; `--backtest` needs one.

Models are tested one after another. A model that could not be tested at all
is a skip, and a skip is counted rather than treated as a bad answer. The exit
code is non-zero when one of the models **failed**, or when every model was
skipped: a run that tested no model proved nothing, and it says so in a
warning.

### The model level

The default. For each model, `varde` runs chapkit's own end-to-end test inside
that model's container:

```text
docker compose exec -T <service> chapkit test --url http://127.0.0.1:8000 --timeout 300
```

chapkit reads the service's own configuration schema, creates a config from it,
generates data matching the covariates, period type and geometry the service
declares, and then validates, trains and predicts against it.

```text
testing 5 models (model level; add --backtest to go through chap-core)
auto-arima-chapkit                  pass   12s   1 training, 1 prediction
chapkit-ewars-model                 pass   18s   1 training, 1 prediction
chapkit-ghr-model                   pass   24s   1 training, 1 prediction
chapkit-rwanda-malaria-bym-model    pass   22s   1 training, 1 prediction
chapkit-simple-multistep-model      pass    5s   1 training, 1 prediction

5 of 5 models pass
```

**What it proves:** the image runs, the service answers, the account the
container runs as can write to its data directory and its workspace, the
runtime has the libraries the model needs, and the model can turn a config and
a frame into a prediction. **What it does not touch:** chap-core. This is
therefore the level to reach for when chap-core is the thing that is broken,
and the level that answers in seconds rather than minutes.

A failure names the phase and the first sentence of the model's own error, out
of the stderr chapkit quotes back:

```text
chapkit-rwanda-malaria-bym-model    FAIL   14s   predict: Error in file(file, "rb") : cannot open file 'model.rds': Permission denied
  run `varde models test chapkit_rwanda_malaria_bym_model -vv` for the full output, and `varde logs chapkit-rwanda-malaria-bym-model` for the service's own
```

`-vv` prints the whole output of `chapkit test`. `-v` shows only the hints.

A model that could not be tested is skipped. The line under the row gives the
way out:

```text
testing 1 model (model level; add --backtest to go through chap-core)
auto-arima-chapkit    skip    0s   its container is not running
  run `varde up`

0 pass, 1 skipped
warning: no model was tested, because every one was skipped; the line under each row says why
```

| Skip | What to do |
| --- | --- |
| `its container is not running` | `varde up`. |
| ``the image has no `chapkit test`; the service reports chapkit 1.0.0`` | The image predates the command. `varde update` moves the pin; `--backtest` tests it through chap-core instead. |
| `chap-core has no configured model for <service> <version>` | `--backtest` only. Run `varde models configs sync ID`, then the test again. |
| `chap-core has no configured model NAME for <service> <version>; it has ...` | `--config` only. Use one of the names, or add the configured model with `varde models configs add ID`. |
| `chap-core did not list its configured models, so varde cannot find --config NAME` | `--config` only. Run `varde status`. |
| `no answer in 5m` | The model is wedged or genuinely slow. The line under the row names `--timeout` with twice the limit. The same number is handed to chapkit as its per-job deadline. |

### The backtest level

`--backtest` goes the whole way round, the path the Modeling App takes. For
each model, `varde` fetches sample data from the model itself through
chap-core's proxy (`GET .../api/v1/ml/$generate-sample-data?kind=train`,
36 periods over 5 org units, in the period type the service declares),
transposes that frame into chap-core observations, posts it as a dataset,
waits for the dataset job, runs a small rolling backtest over it (3 periods, 2
splits, stride 1) and reads the scores off the result.

Before the first model, `--backtest` creates the configured models that the
models of this deployment do not have yet, as [`varde models
configs sync`](#configured-models) does. A model registered from outside the
deployment is not configured by this step.

The backtest names one of chap-core's configured models by its id, chosen from
`GET /v1/crud/configured-models`. Only a configured model of the version that
the service registered with counts, and only one that chap-core has not
archived. A service that registers with a new version keeps the configured
models of the old version, but chap-core does not run them. Without
`--config`, the backtest takes the first of these that exists:

1. The configured model named after the service: the variant name `default`.
2. The lowest-numbered `<service>:<variant>`.
3. A `<service>:test_config_...` that an earlier test left behind.

`--config NAME` names the configured model to backtest. NAME is the variant
name, as `varde models configs` shows it, or the full name `<service>:<variant>`.
See [Testing a configuration](#testing-a-configuration).

A service that has no configured model of its version is skipped, not
backtested. The skip names the version, and the line under it names `varde
models configs sync`:

```text
chapkit-ewars-model    skip    0s   chap-core has no configured model for chapkit-ewars-model 1.0.1
  it is registered, and chap-core has nothing to run it with; run `varde models configs sync chapkit_ewars_model`, then `varde models test chapkit_ewars_model --backtest`
```

That configured model's `additionalContinuousCovariates` are the columns the
backtest hands the model, and the sample data does not always carry them:
`$generate-sample-data` makes the service's required covariates and a pair of
climate ones, but not the ones a configuration adds - the simple multistep
model's defaults include `mean_relative_humidity`, and without it the backtest
fails inside the model with `KeyError: "['mean_relative_humidity'] not in
index"`. So at least as many `feature_N` columns are asked for as the
configuration names, and each covariate the frame lacks takes over a spare one:
the same kind of synthetic seasonal series under the name the model reads.
`-vv` says which column stood in for which.

Geometry is asked for whatever the service declares (`include_geo=true`).
chap-core's dataset always carries a GeoJSON collection, so a model that says
it needs no geometry would otherwise be handed features with none in them - and
a model that builds a neighbour graph fails on an empty polygon where it would
have been perfectly happy with no geometry at all. Real polygons are never
worse: a model that ignores geometry ignores these too. chapkit puts each
location's id in `properties.id` and nothing at the top level, and chap-core
matches org units on the feature's top-level `id` and silently drops the ones
it cannot match, so `varde` sets it before posting.

```text
testing 2 models (through chap-core: a dataset, a backtest and its scores)
chapkit-rwanda-malaria-bym-model    pass   38s   crps 15.5  mae 23.8  rmse 26.9
  configured model: monthly (id 7)
chapkit-ewars-model                 pass   33s   crps 4.6  mae 6.6  rmse 9.7
  configured model: monthly_climate (id 4)

2 of 2 models pass
```

The line under each row names the configured model that the backtest used:
its variant name and its id in chap-core. So the scores are always of a
configured model that you can see. `--json` has the same as `configured_model`,
with `id`, `name` and `variant`. If chap-core did not list its configured
models, the backtest sends the service id, and the line says `(by name, as
chap-core could not list them)`. `id` is then `null`.

**What it proves:** everything the model level does, plus that chap-core can
reach the model, that the covariates and period type the model declares are the
ones chap-core builds a dataset from, that the org units survive the round trip,
and that a real forecast comes back with scores on it. The three printed are
chap-core's `crps`, `mae` and `rmse`; `--json` hands back all fourteen.

A job that fails prints chap-core's own reason - the same line
[`varde jobs logs`](./jobs.md) digs out of the `--- stderr ---` section - and
the command that shows the whole log:

```text
chapkit-ghr-model    FAIL   1m 12s   Error in predict_chap() : the inla program crashed
  run `varde jobs logs 9a84e02f-6a0e-4a3b-9a63-1f2b5a1f3f21`
```

Scores are not a verdict on the model: a two-split backtest over generated data
says the pipeline works, not that the model is any good.

### What it leaves behind, and what it cleans up

Both levels put something in a database, and both delete it again unless
`--keep` says not to.

| Level | What it creates | What is deleted |
| --- | --- | --- |
| model | one config named `test_config_<ulid>` and a few artifacts, in the **model service's own** database | The service's configs and artifacts are listed before the run and again after it, and exactly what appeared is deleted. Deleting a config cascades to the artifacts linked to it. |
| `--backtest` | one dataset and one backtest row in **chap-core's** database, plus the jobs that made them | The backtest first, then the dataset, and nothing else. The jobs stay, because that is the record of what was run; they show up in [`varde jobs`](./jobs.md). |

chap-core's model proxy is read-only by design, so a delete cannot go through
it: the model level lists through the proxy and deletes with the `curl` every
chapkit image carries for its healthcheck (`docker compose exec -T <service>
curl -X DELETE ...`). If a delete does not work, the command says so and names
what was left, so it can be removed by hand.

`--keep` keeps it, and says what was kept. At the model level the line names
the config; it goes away with the volume of the model
(`varde models disable ID --purge`):

```text
chapkit-simple-multistep-model: kept test_config_01M4CVD3GTBZGX2GGQS7131PXQ and 3 artifacts in its database; remove it with `varde docker exec chapkit-simple-multistep-model curl -fsS -X DELETE http://127.0.0.1:8000/api/v1/configs/01M4CVD3GTBZGX2GGQS7131PXQ`
```

With `--backtest`, the line names the two deletes:

```text
kept backtest 8 and dataset 7; remove them with `varde api DELETE /v1/crud/backtests/8` then `varde api DELETE /v1/crud/datasets/7`
```

Both lines are closing lines of the command. They go to stdout, and `--json`
carries them in `messages`.

A backtest chap-core ran also leaves a configuration in the model service's own
database, the way any backtest started from the Modeling App does. That one is
chap-core's, not the test's, so it is left alone.

### What each level needs

| Level | Requirement |
| --- | --- |
| model | The model's container running, and an image with the `chapkit` CLI in it. Every chapkit-built image has one; the marketplace images ship chapkit 2.0 or 2.1. chap-core need not be up at all - it is only asked for the model's declared period type, and for which chapkit the service reports when there is none to run. |
| `--backtest` | chap-core up, the model registered with it, and chapkit **1.1.0 or newer** in the image, which is where `$generate-sample-data` arrived. An older one is skipped with the version it reports. |

### Flags

| Flag | What it does |
| --- | --- |
| `--all` | Test every model in `.varde/models.yaml`. Cannot be combined with an id. |
| `--backtest` | Run the backtest level instead of the model level. |
| `--config NAME` | Backtest the configured model with this variant name. Needs `--backtest`. With more than one model, it applies to each model. |
| `--seed N` | Seed the generated data, so two runs compare. Without it every run is fresh data. |
| `--timeout SECONDS` | How long one model gets: 300 at the model level, 900 with `--backtest`. At the model level the same number is chapkit's per-job deadline. |
| `--keep` | Do not delete what the run created. |
| `-vv` | Stream `chapkit test`'s whole output as it runs, and narrate every request. This is what to add to a failure. |
| `--json` | One object per model: `id`, `service_id`, `level`, `result`, `seconds`, `summary`, `detail`, and - for a backtest - `job_id`, `backtest_id`, `configured_model` and the whole `metrics` object. |

## Models outside the marketplace

A model the catalogue does not list - a new one, a private one, a fork of your
own - is added to one deployment with `varde models add`:

```sh
varde models add https://github.com/my-org/chapkit_dengue_model
varde models add ghcr.io/my-org/chapkit_dengue_model:sha-1eb8cf1
varde models add ghcr.io/my-org/chapkit_dengue_model@sha256:31163f6a...
varde models add my-dengue-model:dev        # built on this machine, never pulled
```

A reference with no registry host is an image in the local image store, such as
one just built with `docker build --platform linux/amd64 -t my-dengue-model:dev .`.
It is read from the local store rather than a registry, rendered with
`pull_policy: never`, and may share a marketplace model's service id while that
model is disabled. See
[A model image you built yourself](./use-cases/local-model-image.md).

The two forms differ in one thing, and it is the important one:

- A **repository URL** follows the default branch. `varde models add` asks
  GitHub for that branch and its newest commits, and ghcr for the newest of
  those commits that has a published `sha-<short commit>` build - which is the
  tag the model repositories' publish workflow writes. `varde update` moves the
  pin to whatever the newest published build is then.
- An **image reference** pins exactly what it names, tag or `@sha256:` digest,
  and `varde update` reports it as pinned and leaves it alone.

Either way the entry is a full definition, so everything else treats it like a
marketplace model: it is enabled by the same write path, rendered into the same
kind of overlay, listed by `models list`, described by `models info`, moved (or
not) by `varde update`, seen by `varde doctor` and shown in the browser.

```text
$ varde -v models add https://github.com/my-org/chapkit_dengue_model --port 5001
added chapkit_dengue_model (chapkit-dengue-model)
hint: source: https://github.com/my-org/chapkit_dengue_model
hint: image: ghcr.io/my-org/chapkit_dengue_model:sha-b1d6c31
hint: pin: sha-b1d6c31 (commit b1d6c31)
hint: follows: main (`varde update` moves the pin)
hint: data dir: /work/data (from the image config)
hint: user: 10001:10001 (from a docker probe)
enabled chapkit_dengue_model sha-b1d6c31 on http://localhost:5001
hint: chapkit_dengue_model is in `compose.chapkit-dengue-model.yml`
the service must register with chap-core as `chapkit-dengue-model`; if its own MLServiceInfo.id differs, `varde status` shows it as unmanaged - run `varde models remove chapkit_dengue_model`, then add it again with `--service-id <that id>`
run `varde up` to apply
```

Every line says where its answer came from, because none of them came from a
reviewed catalogue entry:

| Value | Where it comes from |
| --- | --- |
| id | `--id`, else the repository or image name in snake_case. |
| service | `--service-id`, else the id with hyphens. Not one of the names varde uses itself - `chap`, `worker`, `redis`, `postgres`, `ocs`, `s3`, `s3-init`, `dhis2`, `dhis2-db`, `dhis2-prep`, `varde`, `marketplace` - which a model would be merged into or written over; `models add` and `sync` refuse them. |
| name | `--name`, else the repository or image name. |
| pin | The newest published `sha-` build of the branch, or the tag or digest that was named. |
| data dir | `--data-dir`, else `<WorkingDir>/data` from the image config, else `/work/data`. A `--data-dir` is an absolute path of letters, digits, `/`, `.`, `_` and `-`: it goes into the rendered YAML and the init container's `chown` as it is. |
| user | `--user` (a name or an id, with a group after a `:`, such as `1000:1000`), else the image's own `User` when it is numeric or an account `varde` knows, else the numbers a `docker run ... id -u` probe reads out of the image, else the name with a warning. |
| amd64 | Whether ghcr publishes the image for amd64 alone; `--runtime-amd64` records it either way. |

The data directory and the user are resolved the same way for a marketplace
model, and from the same two fields of the same image config; see
[Data directories and users](#data-directories-and-users). The difference is
only where the answer is recorded: `.varde/models-manual.yaml` holds a manual
model's definition, `.varde/models.yaml` holds every enabled model's.

`--port N|auto` and `--bind ADDR` publish a host port, exactly as on
`models enable`, and are the flags about the deployment rather than about the
model.

### A source the marketplace already lists

`models add` is for models the catalogue does not have, but a URL copied from
a model's page usually names one it does. Without `--id`, such a source is
enabled as the marketplace model, with the catalogue's reviewed pin:

```sh
varde models add https://github.com/chap-models/chapkit_ewars_model
varde models add ghcr.io/chap-models/chapkit_ewars_model:sha-24d58c0   # the same, pinned to 1.0.3
```

```text
https://github.com/chap-models/chapkit_ewars_model is the marketplace model chapkit_ewars_model; enabling that (pass `--id <other>` to add it beside it instead)
enabled chapkit_ewars_model v1.0.4 at http://localhost:8700/v2/services/chapkit-ewars-model/run/
run `varde up` to apply
```

A repository URL matches the entry with that `repository` (case, a trailing
slash and `.git` aside) and follows its stable channel; an image matches only
when its tag is one the entry's versions publish, and pins that version. Any
other tag of the same image is a build the catalogue has not reviewed, so it is
refused:

```text
error: the marketplace already lists chapkit_ewars_model (CHAP-EWARS); run `varde models enable chapkit_ewars_model`, or pass `--id auto` (or `--id <other>`) to add this one beside it
```

A local image (`my-model:dev`) is never matched:
running your own build of a listed model is what `models add` is for.

To keep a separate entry anyway - a fork's builds, a branch the catalogue does
not follow - pass `--id`. `--id auto` takes the source's own id, or that id
with the lowest free `_2`, `_3`, ... suffix when the marketplace or an earlier
`models add` already uses it.

```text
$ varde models add https://github.com/chap-models/chapkit_ewars_model --id auto
added chapkit_ewars_model_2 (chapkit-ewars-model-2)
enabled chapkit_ewars_model_2 sha-964eea8 at http://localhost:8700/v2/services/chapkit-ewars-model-2/run/
run `varde up` to apply
warning: this is the image of the marketplace model chapkit_ewars_model, which registers with chap-core as `chapkit-ewars-model`, so `chapkit-ewars-model-2` may never register; to run chapkit_ewars_model, run `varde models remove chapkit_ewars_model_2`, then `varde models enable chapkit_ewars_model`
```

`--id auto` changes the id and the service id here, not the code in the
image. The image above still registers as `chapkit-ewars-model`, so `varde
status` never shows `chapkit-ewars-model-2` as registered. Any `--id` on the
repository or an image of a marketplace model gives the same warning. `--service-id chapkit-ewars-model` is
refused, because the marketplace model uses that name. Use `--id` for a fork
that sets its own `MLServiceInfo.id`.

The **service id has to match the id the service registers with chap-core**
(chapkit's `MLServiceInfo.id`). It is the Compose service name, the DNS name
and the name the registration resolves to, all at once. Where a model's own id
differs from its repository name, pass `--service-id`; `varde status` shows the
mismatch as a registered service nobody manages next to a model that never
arrived.

The uid probe is the one step that needs Docker: an image that runs as an
account name (`USER app`) says nothing about what that name resolves to, and
the overlay's init container chowns the data volume from busybox, which
resolves no names at all. So `varde models add` pulls the image once and asks
it. Pass `--user <uid>:<gid>` to skip that.

### Where it is recorded

The definition goes in `.varde/models-manual.yaml`, beside the enabled set:

```yaml
# Managed by varde; change it with the varde commands, not by hand.
chapkit_dengue_model:
  service_id: chapkit-dengue-model
  display_name: chapkit_dengue_model
  repository: https://github.com/my-org/chapkit_dengue_model
  image: ghcr.io/my-org/chapkit_dengue_model
  tag: sha-b1d6c31
  commit: b1d6c312a83f07aa1a4e66fce05ae7f4eccb8188
  follow: main
  data_dir: /work/data
  user: 10001:10001
  runtime_amd64: true
  added: 2026-09-24
```

That file is the definition; `.varde/models.yaml` still says which models are
on. So `varde models disable chapkit_dengue_model` keeps the definition and
`varde models enable chapkit_dengue_model` brings the model back at the recorded
tag, data directory and user, without asking the network anything. The host
port is not in the definition: if the model had one, give `--port` again. It is
carried over by `varde init --force` and included in `varde backup create`.
A deployment that has added nothing has no such file.

Both listings mark these entries, because a local definition is not a reviewed
catalogue entry:

```text
ID                    SERVICE               NAME                  STATUS        STABLE       LATEST       PORT  KIND
chapkit_ewars_model   chapkit-ewars-model   CHAP-EWARS            limited data  1.0.4        1.0.4        -     model
chapkit_dengue_model  chapkit-dengue-model  chapkit_dengue_model  not for use   sha-b1d6c31  sha-b1d6c31  5001  manual
```

`varde models info chapkit_dengue_model` says the same in its own words - a
`kind` of `manual`, a `source` line naming `varde models add` and the day it
was added, and a `follows` line naming the branch or reading
`nothing (pinned)` - and leaves out every field a
marketplace entry would have filled in (the horizon, the covariates, the
period types, the assessment) rather than reporting them as zeroes. The
browser shows `manual` in its `KIND` column.

### Removing one

```sh
varde models remove ID [--purge]
```

`remove` is the way back: it disables the model if it is enabled - stopping its
container and removing its overlay, exactly as `models disable` does - and then
drops the definition. `--purge` takes the data volume as well, and means the
same thing here as it does there. A marketplace model has no local definition
to remove, and says so:

```text
error: chapkit_ewars_model is a marketplace model, so there is no local definition to remove; run `varde models disable chapkit_ewars_model`
```

### What it needs, and what it refuses

`varde models add` reads GitHub's REST API and ghcr; both are public for the
Chap model repositories, so neither needs a credential. GitHub allows 60
requests an hour to an address that sends none, and `varde` sends
`GITHUB_TOKEN` (or `GH_TOKEN`) where the environment names one, for 5000 an
hour instead - it is never stored and never printed, and `varde doctor`'s
`github api` line says how much of the hour is left. The
repository form cannot work under `--offline` and says so, naming the image
reference to pass instead. The image form works offline as long as the image's
`linux/amd64` variant is in the local image store, because then `docker image
inspect` can answer what the registry would have; the error names the `docker
pull --platform linux/amd64` that puts it there.

Refused rather than guessed at: a bare image name (`chapkit_dengue_model`), an
image on a registry other than ghcr, an image reference with no tag or digest,
an id or service name the marketplace already uses (`--id auto` picks a free
id), and an id this deployment
has already added. The marketplace always wins a collision, so an id it
publishes later shadows nothing: the local entry is reported and ignored until
it is removed or renamed.

## The browser

`varde ui` opens two pages: **models**, the marketplace catalogue, and
**components**, what this deployment is made of. `Tab` goes to the next page
and `shift-Tab` back; one `s` saves both and leaves the browser, so the two
are a single sitting rather than two commands.

The models page is the catalogue as one table across the width of the terminal,
with a one-line summary of the row under the cursor under it:

```text
varde · models                                                registry: cache · 2 min   7 models · 1 enabled · 0 pending
╭ Marketplace ─────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│     MODEL                  ID                                 STATUS         VERSION  PORT                           │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│ ▸ ✓ CHAP-EWARS             chapkit_ewars_model                ● limited data 1.0.4    via chap-core                  │
│     Simple Multistep       chapkit_simple_multistep_model     ● limited data 0.1.1                                   │
│     Auto-ARIMA             auto_arima_chapkit                 ● experimental 1.0.1                                   │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│ CHAP-EWARS  ● limited data  1.0.4 (sha-964eea8)  enabled, via chap-core  requires population            i for details│
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 [j/k] move  [tab] page  [space] toggle  [i] info  [m] configs  [p] port  [s] save  [ctrl+k] commands  [?] help  [q] quit
```

The summary line names the model, what the author's assessment means, what the
row is pinned to, whether this deployment runs it and what data it needs, with
`i for details` held at the right-hand edge. A terminal too narrow for all of
it cuts the covariates first, marked with `~`; the hint never goes. Everything
the line leaves out - the summary text, the defaults, the period types, the
forecast horizon and the service user - is behind `i`.

The first column is the cursor, the second what the row will be: `✓` for a
model this deployment runs, `+` for one you have switched on, `-` for one you
have switched off. `STATUS` is the author's own chapkit assessment, the same
five words [the table above](#what-the-status-column-means) explains; `VERSION`
is what the row's channel resolves to; and `PORT` is how the model is reached.
A `KIND` column appears when the list holds something that is not a plain
marketplace model: a template, or an entry `varde models add` created. The
rows are in the same maturity order `varde models list` uses, with templates
last when `t` shows them; nothing done in the browser reorders them, so the
cursor stays on the row it was on.

The keys, none of which need Alt on a Norwegian keyboard:

```text
j / k / arrows     move            space            enable or disable
g / G              first / last    i / Enter        the full details
PageUp / PageDown  jump a page     p                publish a host port: a number, auto, or none
ctrl-u / ctrl-d    jump a page     P                take the host port away
Tab / shift-Tab    change page     v                pick the channel (stable, latest)
s                  save            t                show or hide templates
u                  discard         /                filter (Enter keeps, Esc clears)
q                  quit            Esc              clear the filter, or quit
?                  help            ctrl-k / ctrl-p  the command palette
o                  open in a browser: the model's repository, or the component's web interface
c                  the model's image reference, on the status line
m                  the configured models of the model in chap-core
y / n              answer the quit confirmation
```

`ctrl-n` also moves down and `ctrl-c` always leaves. While filtering, the
arrow keys still move the cursor and `ctrl-u` empties the filter without
leaving it. An active filter shows in the title as `· filter <text>` and in
the box title as how many entries it left. The key bar gives up its entries
from the least useful end when the terminal is too narrow for all of them.

The `PORT` column reads `via chap-core` for an enabled model with no host port
of its own, the port number for one that publishes one, and `auto` for a row
that has asked for a port that is not picked until you save. A row you have
switched on or off reads `will be enabled` or `will be disabled` until you
save.

### The configured models page

`m` on a model row opens the [configured models](#configured-models) of the
model in chap-core, with the columns NAME, COVARIATES, SOURCE and OPTIONS.
The keys on this page:

```text
j / k / arrows     move
a                  add a configured model
e                  change the values of the configured model under the cursor
d                  archive the configured model under the cursor
r                  ask chap-core again
Esc / q / m        go back to the models page
```

`a` and `e` open the form. It has one field for each user option of the
model, with its kind, its default and its description, and one field for the
covariates. The form of `a` also has a field for the variant name. The form
of `e` starts with the current values. An empty field uses the default.

- `Tab` and the arrows move between the fields.
- `Enter` saves the form.
- `Esc` closes the form and writes nothing.

varde writes nothing to chap-core until you answer `y` to the question. For
`d`, the question says that chap-core keeps the configured model, and that the
Modeling App shows it as Archived.

The page needs a chap-core that runs and a model that the deployment runs.
If chap-core does not answer, the page says:

```text
chap-core at http://localhost:8700 is not running; start it with `varde up`, then press r
```

If the model is not enabled, or the deployment has no chap-core, the footer
line gives the way out.

### Publishing a host port

`p` opens a dialog over the list, prefilled with the port the row has today or
with `auto`:

```text
╭ Host port for CHAP-EWARS ──────────────────────────────────────────────╮
│ port  › 80_                                                            │
│ now: via chap-core · range 5001-5999 · api port 8140 is taken          │
│ port 80 is outside this deployment's range 5001-5999                   │
│                                                                        │
│ [enter] apply  [esc] cancel  [auto] any free port  [none] no host port │
╰────────────────────────────────────────────────────────────────────────╯
```

It takes a port number, `auto` for the lowest free port in the project's
range, or an empty line or `none` to take the port away again. A number it
cannot accept - outside the range, chap-core's own API port, or one another
model in the list is already asking for - keeps the dialog up with the reason
in it, as above, so the number is corrected rather than typed again. The two
checks only a save can make - a port another compose file claims, and a port
something on this machine is listening on - happen when the selection is
applied, exactly as they do for `varde models expose`. `P` takes the port away
without the dialog.

### Picking a channel

`v` opens the same kind of dialog, listing both channels with the version each
one resolves to and a `✓` on the one the row follows today:

```text
╭ Channel for CHAP-EWARS ─────────────────────────────────╮
│ ▸ ✓ stable  1.0.4                                       │
│     latest  1.0.4                                       │
│                                                         │
│ [enter] apply  [esc] cancel  [j/k] move  [s/l] pick one │
╰─────────────────────────────────────────────────────────╯
```

`j` and `k` move between them, `s` and `l` jump straight to one, `Enter` takes
it and `Esc` leaves the row alone. Changing the channel re-resolves the
version when the selection is saved; changing only the port does not.

A row pinned with `--version` follows no channel. The `VERSION` column shows
the pin, and the dialog puts no `✓` on `stable` or `latest`. A third line,
which the cursor cannot reach, shows the pin:

```text
│   ✓ pinned  1.0.3                                       │
```

The summary strip shows the pin too. A save keeps the pin unless you pick a
channel.

### The components page

`Tab` from the models page opens what this deployment is made of, with the same
columns [`varde components list`](./components.md) prints:

```text
varde · components                                             registry: embedded   4 components · 2 enabled · 0 pending
╭ Components ──────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│     COMPONENT    STATE    REACH                    WHAT IT IS                                                        │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│ ▸ ✓ chap-core    enabled  http://localhost:8700    Chap itself: chap-core, its worker, Valkey and PostgreSQL         │
│   ✓ ocs          enabled  http://localhost:8790    Open Climate Service: climate data, reachable at http://ocs:9000  │
│     s3           off      -                        RustFS, an S3-compatible object store OCS will keep objects in    │
│     dhis2        off      -                        DHIS2 and its own database, for a deployment that wants one       │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│ chap-core  enabled here  http://localhost:8700  compose.yml + compose.varde.yml  Chap itself: chap-co~  i for details│
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 [j/k] move  [tab] page  [space] toggle  [i] info  [o] open  [p] port  [s] save  [ctrl+k] commands  [?] help  [q] quit
```

The marker column reads the same way it does on the models page: `✓` for a
component this deployment has, `+` for one you have switched on this session,
`-` for one you have switched off. `STATE` follows it - `enabled`, `off`, and
`adding` or `removing` while a change is pending - so the row says both what is
recorded and what saving would do to it. `REACH` is where the component answers
from this machine, or `internal` for one that publishes no host port, and `-`
for one that is off. The strip under the table is the row the cursor is on:
whether this deployment has it, where it answers, which compose file it is
rendered into, and what it is, with the same `i for details` held at the
right-hand edge.

```text
varde · components                                             registry: embedded   4 components · 3 enabled · 1 pending
╭ Components ──────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│     COMPONENT    STATE    REACH                    WHAT IT IS                                                        │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│   ✓ chap-core    enabled  http://localhost:8700    Chap itself: chap-core, its worker, Valkey and PostgreSQL         │
│   ✓ ocs          enabled  http://localhost:8790    Open Climate Service: climate data, reachable at http://ocs:9000  │
│ ▸ + s3           adding   internal                 RustFS, an S3-compatible object store OCS will keep objects in    │
│     dhis2        off      -                        DHIS2 and its own database, for a deployment that wants one       │
│                                                                                                                      │
│                                                                                                                      │
│──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────│
│ 1 pending change  nothing is written until you save                                                                  │
│ + s3                      enable, on the compose network                                                             │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 [j/k] move  [tab] page  [space] toggle  [i] info  [o] open   [s] save 1 change   [u] discard  [?] help  [q] quit
```

`space` toggles the component under the cursor, `p` sets its host port and `P`
takes it away, `o` opens its web interface in a browser, and `i` or `Enter`
opens the details. The pending counter in the
header, the summary strip and the quit confirmation all count both pages, so a
component change cannot be lost by saving from the models page - and `u`
discards both pages, because it means "nothing I did this session" rather than
"nothing on this page".

The port prompt here is narrower than the models one: it takes a port number,
or an empty line or `none` to take the port away, and **not** `auto`. `auto`
means "the lowest free port in this project's model range", and a component
publishes a well-known port of its own that is not in that range, so the prompt
says what to type rather than picking a number nobody asked for. `p` on
`chap-core` is refused outright, because its host port is the API port, which
`CHAP_API_PORT` in `.env` sets: the footer names that line instead. `p`
on a component that is off asks you to enable it first.

`o` opens the row's web interface in a browser - chap-core's API documentation,
the OCS web interface, DHIS2's own - and says in the footer when there is none
to open: a component this deployment does not have, one that publishes no host
port (naming the address it is reached at inside the deployment), one this
session has only just enabled and not saved, and the object store, which speaks
the S3 API and serves no interface a browser is useful for. It opens what this
deployment publishes **now**, so a port changed in this session and not yet
saved is not the port it opens. `Open web interface` in the palette does the
same thing. It is the same command as
[`varde open`](./components.md#reaching-a-component-from-a-browser), which is
the one that can also say whether the container is running.

`i` opens the details, which say where the component's compose file and data
volume are, and - for OCS - which of its settings this page deliberately does
not edit:

```text
╭ ocs                                                                   enabled here ╮
│ Open Climate Service: climate data, reachable at http://ocs:9000                   │
│                                                                                    │
│ state       enabled                                                                │
│ reach       http://localhost:8790                                                  │
│ compose     compose.ocs.yml · rendered from .varde/components.yaml by `varde sync` │
│ volume      ocs_data · kept when the component is disabled                         │
│ config      ocs/climate-service.yaml · yours to edit, and varde never rewrites it  │
│                                                                                    │
│ not on this page:                                                                  │
│   `varde components enable ocs --base-url URL`                                     │
│     the public origin OCS builds its STAC and openEO links from                    │
│   `varde components enable ocs --read-only`                                        │
│     refuse ingestion over HTTP (--read-write allows it again)                      │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

Those two are one-way settings on a file rather than a field on a row:
`--base-url` is a URL to type correctly rather than pick, and `--read-only`
edits `ocs/climate-service.yaml`, which is the operator's file. A dialog that
looked like it owned them would be a dialog that quietly rewrote your config,
so the overlay names the commands instead. See
[Behind a reverse proxy](./components.md#behind-a-reverse-proxy) and
[Read-only instances](./components.md#read-only-instances).

DHIS2's overlay is the longest one, because a DHIS2 is the component with the
most that cannot be seen from its row:

```text
╭ dhis2                                                                 enabled here ╮
│ DHIS2 and its own database, for a deployment that wants one                        │
│                                                                                    │
│ state       enabled                                                                │
│ reach       http://localhost:8780                                                  │
│ compose     compose.dhis2.yml · rendered from .varde/components.yaml by `varde     │
│             sync`                                                                  │
│ volume      dhis2_home, dhis2_db, dhis2_dump · kept when the component is disabled │
│ config      dhis2/dhis.conf · yours to edit, and DHIS2 will not start without it   │
│ seed        https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz · restored   │
│             once, into the database the first `varde up` creates                   │
│ cache       dhis2_dump holds the downloaded dump rather than data; the dhis2-dump  │
│             one-shot fetches it again when the volume is empty                     │
│ first start minutes, not seconds: DHIS2 migrates its schema on the way up, and     │
│             `varde logs dhis2` is where that shows                                 │
│                                                                                    │
│ not on this page:                                                                  │
│   `varde components enable dhis2 --seed SPEC`                                      │
│     the dump a database being created is restored from: default, none, a URL, or a │
│     path in the deployment directory                                               │
│   `v` on this row, or `varde components enable dhis2 --tag TAG`                    │
│     the DHIS2 version, 2.42 here; it migrates a schema forward only, so run `varde │
│     backup create` first                                                           │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

Four things only the overlay says. That `dhis2/dhis.conf` is yours **and** that
DHIS2 does not start without it. That one of the three volumes is a download
cache rather than data. What the database will be restored from, and that a
restore only ever happens to a database being created - so changing the seed on
a deployment that has already started means removing the volume. And that the
first start takes minutes, which is the difference between reading the first
`varde status` as slow or as broken.

The two settings at the bottom are a key in `.varde/components.yaml` and a
`varde sync`. No flag changes the seed after `init`: it is a one-chance setting.
The image tag also has `--tag` on `varde components enable dhis2`, and `v` on
the `dhis2` row picks it, since DHIS2 is the one component with versions to
choose between. See
[Changing the DHIS2 version](./dhis2.md#changing-the-dhis2-version).

### The details, and the command palette

`i` or `Enter` opens the full entry over the list, in the order the Chap
Modeling App presents a model: what it does, what it is, who wrote it, and
only then how this deployment runs it.

```text
╭ Rwanda Malaria BYM  chapkit_rwanda_malaria_bym_model              not enabled here ╮
│ Spatio-temporal Bayesian model for malaria incidence in Rwanda at sector (ADM3)    │
│ level: BYM spatial effects, RW1 temporal effects and an IID space-time interaction │
│ with lagged climate covariates, fitted with R-INLA. Built for one country's data — │
│ it needs geometry and the full climate covariate set.                              │
│                                                                                    │
│ version     0.1.3 (sha-1ee5ea1) · verified · channels stable, latest               │
│             0.1.2 (sha-a7b2892) · verified                                         │
│ author      Similien NDAGIJIMANA · HISP Centre, University of Oslo ·               │
│             knut.rand@dhis2.org                                                    │
│ status      ● gray, not intended for use, deprecated or kept for backwards         │
│               compatibility                                                        │
│ period      monthly · horizon 1 to 24 periods                                      │
│ target      disease cases                                                          │
│ covariates  population, rainfall, mean_temperature, relative_humidity · defaults - │
│             · free extras allowed · geometry required                              │
│                                                                                    │
│ reach       via chap-core (through chap-core's /run/ proxy)                        │
│ image       ghcr.io/chap-models/chapkit_rwanda_malaria_bym_model:sha-1ee5ea1       │
│ runtime     ghcr.io/dhis2-chap/chapkit-r-inla (amd64 only)                         │
│                                                                                    │
│ maintainers mortenoh, edvinstava                                                   │
│ repository  https://github.com/chap-models/chapkit_rwanda_malaria_bym_model        │
│ citation    Climate Health Analytics Platform. 2025. "Kigali Malaria BYM Model".   │
│             HISP Centre, University of Oslo.                                       │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

The title carries the name, the marketplace id and whether this deployment
runs it - `enabled, via chap-core`, `enabled on port 5010` or `not enabled
here`; the id is the part that gives way when the three do not fit. `version`
is the pin, how far that release is trusted and which channels point at it,
with the rest of the published history under it when there is more than one.
`target` is what chap-core forecasts, the same for every model in the
catalogue. A model this deployment runs also lists its data directory and its
service user with where that user came from. A model with a citation in the
marketplace also gets a `citation` row, with the attribution as the marketplace
gives it. Long values - the summary, the assessment, the citation - wrap under
their label rather than being cut.

`j` and `k` scroll it when it is taller than the terminal, `o` opens the
repository in a browser, `c` puts the image reference on the status line, and
`esc`, `i` or `q` closes it again. A component's overlay binds `o` too, where it
opens that component's web interface, and binds no `c`: a component has no image
reference of its own.

`ctrl-k` or `ctrl-p` opens a command palette: type to narrow it, `up` and
`down` to move, `Enter` to run, `esc` to leave. Its first entry is always the
other page - `Go to the components page`, or `Go to the models page` - and the
rest follow whichever page you are on, so on the components page it offers
`Enable or disable the ocs component`, `Set a host port for the ocs component`
and `Open the web interface of the ocs component` where the models page offers
the channel, the template filter and `Open the repository of <model>`.

It offers everything the keys do, plus three the keys do not: refreshing the
catalogue from the marketplace without leaving the browser, the way
`varde registry update` does, opening this documentation - the chapter for the
page you are on - and `Save a screenshot (SVG)`, which writes the frame the
palette just closed over to `varde-ui-<date>-<time>.svg` in the directory
`varde ui` was started in and names the file on the status line. The two port
commands are separate, so "set a host port" never takes one away.

Every component is also in the palette by name, from either page - `Turn the
dhis2 component on or off` and one such entry per component - because that is
where somebody looking for OCS, an object store or a DHIS2 finds out the browser
has them at all. Running one turns it on or off and moves the components page's
cursor to it, without moving the page you are on.

### Saving

With something unsaved, the summary strip lists what saving would write, one
line per change, and the key bar grows a filled `[s] save N changes` and a
`[u] discard` next to it. Both pages are in that count and in that one save:
`s` writes the model changes through the same path as `varde models enable` and
the component changes through the same path as `varde components enable`. Then
the browser closes and prints what changed:

```text
updated chapkit_ewars_model v1.0.4 on http://localhost:5001
enabled the s3 component
run `varde up` to apply
```

A save writes the files and starts nothing: run `varde up` to apply it. The
help line of `s` says `` save the changes; `varde up` applies them ``, and the
palette entry says `` Save the changes; `varde up` applies them ``. With nothing pending, `s` prints `no
changes to save` and closes the browser.

Nothing is written until you save, and quitting with unsaved changes asks
first, on either page and for either kind of change: `Quit anyway? y / n`.
`varde ui` owns the terminal, so it rejects `--json`.

`varde init --interactive` opens the same browser to pick the initial model
set instead of reading `--models`.

## How a model overlay looks

Each marketplace model is a [chapkit](https://github.com/dhis2-chap/chapkit)
container listening on port 8000 with `/health` and `/api/v1/info`. This is the
overlay `varde` writes for the default model. The file carries two header lines
and nothing else by way of explanation: what it does is below.

```yaml
# Generated by varde from .varde/; edit there and run `varde sync`.
# chapkit_ewars_model 1.0.4 (https://github.com/chap-models/chapkit_ewars_model)
services:
  chapkit-ewars-model:
    labels:
      com.winterop.varde.role: model
      com.winterop.varde.model: "chapkit_ewars_model"
      com.winterop.varde.kind: init
    restart: unless-stopped
    image: ghcr.io/chap-models/chapkit_ewars_model:${CHAPKIT_EWARS_MODEL_IMAGE_TAG:-sha-964eea8}
    platform: linux/amd64
    init: true
    expose:
      - "8000"
    environment:
      SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register
      # Uncomment if chap has SERVICEKIT_REGISTRATION_KEY set:
      # SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    user: 1000:1000
    volumes:
      - type: tmpfs
        target: /tmp
        tmpfs:
          size: 2000000000
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    depends_on:
      chapkit-ewars-model-init:
        condition: service_completed_successfully
      chap:
        condition: service_healthy

  chapkit-ewars-model-init:
    labels:
      com.winterop.varde.role: model
      com.winterop.varde.model: "chapkit_ewars_model"
      com.winterop.varde.kind: init
    image: busybox:1.38
    command: ["sh", "-c", "chown -R 1000:1000 /app/data"]
    user: "0:0"
    volumes:
      - type: volume
        source: ck_chapkit_ewars_model_data
        target: /app/data
    restart: "no"

volumes:
  ck_chapkit_ewars_model_data: {}
```

### What an overlay contains

The overlay does seven things.

- **Labels both containers** with their role, the marketplace id and the
  deployment's kind, so `docker ps --filter label=com.winterop.varde.model`
  finds every model on the machine. See
  [Container labels](./concepts.md#container-labels).
- **Pins the image.** `image: <repo>:${<ID>_IMAGE_TAG:-sha-<commit>}`, where the
  default is the `image_tag` of the version the channel resolves to. The
  deployment stays reproducible, and a different build is one `.env` line away.
- **Self-registration.** The service registers itself with chap-core through
  `SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register` (the
  `$$` is a literal `$` for Compose). The Compose service name, the DNS name
  and the marketplace `service_id` are the same string, which is what makes the
  registration resolvable. Set `SERVICEKIT_REGISTRATION_KEY` in `.env` to
  require a shared secret. In a deployment without chap-core the whole
  `environment:` block is left out, so the service registers nowhere; see
  [Model services without chap-core](./components.md#model-services-without-chap-core).
- **Publishes no host port.** The service gets `expose: ["8000"]` and nothing
  else, unless the deployment has no chap-core, where a model gets a port by
  default. See [Ports](./ports.md).
- **Hardening**, matching the posture of the base services: `init: true`,
  `read_only: true`, `no-new-privileges`, `cap_drop: [ALL]`, a 2 GB tmpfs at
  `/tmp`, and a named volume for the model's data directory (the only writable
  path besides `/tmp`).
- **Runs it as the account its image declares.** `user: <uid>:<gid>`, in
  numbers, for an image that drops to an account of its own. An image that
  runs as root gets no `user:` line at all: the line exists to override the
  image, and overriding root with root only risks taking a permission the
  image's own binaries need. See
  [Data directories and users](#data-directories-and-users).
- **Hands the data volume to the model user**, through the init container
  below - again, only where the model is not root.
- **Ordering.** `depends_on`: the init container with
  `condition: service_completed_successfully` (where there is one) and `chap`
  with `condition: service_healthy`, so a model only starts once its volume is
  writable and chap-core can accept its registration. Without chap-core only
  the init container is waited for.

Three things an overlay deliberately leaves out:

- no `networks:` key, so the service joins the compose default network only -
  which reaches chap-core but not PostgreSQL or Valkey;
- no `healthcheck:`, because chapkit images ship their own;
- no `depends_on` between models, because they are independent of each other.

## The init container, and why

Docker seeds a fresh named volume from whatever the image has at the mount
point, ownership included, so an image that never creates its data directory
yields a root-owned volume the unprivileged model cannot write to, and chapkit
dies on `sqlite3.OperationalError: unable to open database file`.

Compose cannot `chown` a volume, so the overlay of a model that runs as an
unprivileged account ships a one-shot `<service_id>-init` container (busybox,
as root, `restart: "no"`) that chowns the mount point, recursively, to the
model's **numeric** uid:gid before the model starts. Busybox resolves no `chapkit`
account, which is why the numbers matter, and why the `user:` line above
carries the same two numbers rather than the name. It is the one service in a
deployment that is deliberately not `restart: unless-stopped` (the `"no"` is
quoted because bare `no` is YAML's `false`).

**Every** model overlay gets one, root included, where the chown is to `0:0`.
A volume docker has just created is root-owned already, so on a fresh
deployment a root model's init container does nothing - but a volume that
already exists carries whoever owned it last. If a model that ran as
`1000:1000` now runs as root (a new image, or a `--user` override), its volume
is owned by 1000, and the model cannot write to it: the overlay drops every
capability, and `CAP_DAC_OVERRIDE` is the one that lets root ignore the
permission bits. One busybox one-shot costs a second on `varde up` and repairs
that change, which is worth more than the line it saves.

The container has a second job: because it mounts the same named volume at the
same path as the model itself, `varde backup` reads and writes model data
through it, whether the model is running, stopped, or brought down entirely.
See [Backup and restore](./backup.md).

## Data directories and users

Both differ per image, and both are read off the image itself when the model
is enabled - `varde models enable`, `varde init --models`, the browser's
toggle. The answers go into `.varde/models.yaml`, so `varde sync` renders the
overlay from what is recorded and never asks the network: the same state
renders the same bytes on any machine.

| Value | Where it comes from, in order |
| --- | --- |
| data dir | `--data-dir`, else `<WorkingDir>/data` from the image config, else the built-in table, else `/work/data`. |
| user | `--user`, else `config.User` from the image config on ghcr, else the same field from an image already pulled here (`docker image inspect --platform linux/amd64`), else the built-in table. |

`varde models info <id>` and the browser's details overlay name which of those
answered: `image config`, `docker probe`, `--user` or `table`. A run that could
reach neither ghcr nor a local copy says so, because the table is a snapshot
this binary was built with and an image can change what it runs as.

What the marketplace images declare today:

| Image | Data directory | User |
| --- | --- | --- |
| EWARS | `/app/data` | `chapkit`, rendered as `1000:1000` |
| The simple multistep model | `/app/data` | `chap`, rendered as `1001:1001` |
| The Rwanda BYM model | `/work/data` | `chapkit`, rendered as `1000:1000` |
| GHRmodel | `/work/data` | `app`, probed as `10001:10001` |
| Everything else | `/work/data` | `root` |

**An image that runs as root gets no `user:` line**; its init container is
rendered like any other model's and chowns the volume to `0:0`. Three of the
seven marketplace images end their Dockerfile on `USER root`, and forcing an
unprivileged uid on one of them takes away a permission its own binaries need:
the Rwanda BYM model up to 0.1.1 kept its INLA binaries root-owned and mode
744, so every prediction failed with `inla.run: Permission denied`. That is
why the image is asked rather than assumed: 0.1.2 rebuilt it to run as
`chapkit`, and re-enabling the model is what picks that up. See
[`Permission denied` from a model's own binaries](./troubleshooting.md#permission-denied-from-a-models-own-binaries).

The init container needs an account name as numbers: `chapkit` is uid/gid 1000
and `chap` is 1001. GHRmodel's `app` is not one of them, so enabling it pulls
the image once and asks it, exactly as `varde models add` does. A `--user` that
is already numeric is passed through, and a name nothing could resolve falls
back to `1000:1000` with a warning from `varde sync`. `--data-dir` and `--user` override everything above, for an image
whose config says something the deployment has to contradict.

`varde doctor` re-checks each enabled model against the image on this machine
and warns when the two have drifted apart, which is what a marketplace model
moved to a new tag by hand looks like.

A model added with `varde models add` is resolved the same way at the moment it
is added, and carries its answers in `.varde/models-manual.yaml`. See
[Models outside the marketplace](#models-outside-the-marketplace).

## The amd64 pin, and why

The overlay of every marketplace model pins `platform: linux/amd64`, and the
overlay of a model added with `varde models add` pins it only when its image is
published for amd64 alone (or `--runtime-amd64` says so). The marketplace images are
published for amd64, as is chap-core itself, so an arm64 host such as Apple
silicon pulls that variant and runs it under emulation instead of failing with
`no matching manifest for linux/arm64`.
