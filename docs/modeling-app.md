# Your first forecast in the Modeling App

A walk from a fresh deployment to an evaluated model and a three-month dengue
forecast, by clicking in the Modeling App. It takes about fifteen minutes, most
of it waiting for the model. It starts where
[CHAP with a local DHIS2](./use-cases/chap-with-local-dhis2.md) ends:

```sh
chaps init mychap --models default --with dhis2
cd mychap
chaps up
chaps status          # until the dhis2 line says up
chaps dhis2 connect
chaps open dhis2
```

Tested with Modeling App 7.1.0, DHIS2 2.42.6 with the Laos demo database,
chap-core v2.3.1 and CHAP-EWARS (chapkit) 1.0.3. Other versions of the Modeling
App may name a button differently; the steps stay the same. `chaps dhis2 show`
says which Modeling App version a deployment has.

## What the demo data holds

The DHIS2 that `chaps` starts is seeded with a demo database from Laos:

| Data | Where it is in DHIS2 | Range |
| --- | --- | --- |
| Dengue cases | `Dengue Cases (Any) - Weekly` | 2019 to 2024, weekly |
| Rainfall | `CCH - Precipitation (CHIRPS)` | 2015 to April 2025 |
| Temperature | `CCH - Air temperature (ERA5-Land)` | 2015 to April 2025 |
| Population | `LSB: Population (Estimated-single age)` | 2016 to 2025, yearly |

for Lao PDR, its 18 provinces and their districts. The Modeling App adds the
weekly cases up to months when an evaluation is monthly.

## 1. Open the Modeling App

Log in to DHIS2 with `admin` / `district`, open the app menu (the grid of
squares at the top right), type `Modeling` in its search box and pick
**Modeling**: the menu shows only a few top apps until you search. The
dashboard is empty until the first evaluation.

![The Modeling App dashboard](images/modeling-app/01-dashboard.png)

## 2. Start an evaluation

An evaluation asks: had this model been used in the past, how close would its
forecasts have come? It trains the model on the earlier data, forecasts the
months after, and compares the forecast with what really happened, several
times over.

In **Evaluate**, **Overview**, choose **New evaluation** and fill in:

| Field | Value |
| --- | --- |
| Name | anything, for example `EWARS Laos dengue` |
| Period type | Monthly |
| From period / To period | 2019 January to 2024 December |
| Organisation units | tick **Lao PDR**, then pick the level **Province** below the tree |
| Model | **CHAP-EWARS Model (chapkit)** |

![Selecting the model](images/modeling-app/02-select-model.png)

The list also holds models chap-core ships configured on its own and runs
inside its worker; some say *Deprecated* in their description. **CHAP-EWARS
Model (chapkit)** is the one `chaps` started with `--models default`, and each
model enabled with `chaps models enable` adds its own entry. `chaps status`
lists those, and `chaps models test` checks them.

![Organisation units: Lao PDR at the Province level](images/modeling-app/03-org-units.png)

## 3. Map the data

**Configure sources** asks which DHIS2 data each input of the model comes from.
Search by name in each box:

| Model input | DHIS2 data item |
| --- | --- |
| Disease cases | `Dengue Cases (Any) - Weekly` |
| Population | `LSB: Population (Estimated-single age)` |
| Rainfall | `CCH - Precipitation (CHIRPS)` |
| Mean temperature | `CCH - Air temperature (ERA5-Land)` |

![Mapping the model inputs to DHIS2 data](images/modeling-app/04-map-data.png)

**Save**, then **Start dry run**. It checks the data without starting
anything, and should say all 18 locations can be imported.

![The dry run: 18 valid locations](images/modeling-app/05-dry-run.png)

Close it and choose **Start import**. The app moves to **Jobs**, where the
evaluation runs. With CHAP-EWARS it took about four minutes on an Apple Silicon
Mac, where the model runs under emulation. `chaps jobs` in the terminal shows
the same job.

## 4. Read the result

Back in **Evaluate**, **Overview**, open the evaluation. The chart shows the
real cases (orange) and, for the period picked under **Split period**, the
model's forecast with its 50% and 80% prediction intervals. The list on the left
switches between all of Laos and one province.

![An evaluation result](images/modeling-app/06-evaluation.png)

**Evaluation metrics**, lower on the right, scores the forecasts over every
split and province: lower is better for CRPS, MAE and RMSE, and *Coverage 10-90*
should be close to its target of 0.8 (80% of the real values inside the 80%
interval).

![Evaluation metrics](images/modeling-app/07-metrics.png)

To compare two models, run a second evaluation on the same data: on the first
one, **Create new based on...** asks which settings to copy (leave them all
ticked) and **Create** copies the name, provinces, periods and data mapping;
**Select model** on the copy picks another model, then **Start import**. The
*Monthly CHAP-EWARS model* chap-core ships took about four minutes as well.
Then **Evaluate**, **Compare**: pick the first evaluation in the left box and
the copy in the one beside it, and the two are shown side by side, province by
province.
More marketplace models come with `chaps models enable ID` and `chaps up`
(`chaps models list` shows the ids).

![Two evaluations side by side](images/modeling-app/08-compare.png)

## 5. Make a forecast

On the evaluation, **Create prediction setup** keeps the model, the data
mapping and the provinces under a name: type one in **Setup name**, leave the
default import mapping off and **Save**. Once the evaluation has a setup, the
same button says **Predict**.

![Creating a prediction setup](images/modeling-app/09-prediction-setup.png)

On the setup, **Run prediction**. The last training period is the last month
with case data, **2024 December** for the demo data; the forecast covers the
three months after it.

![Running a prediction](images/modeling-app/10-run-prediction.png)

It took about twenty seconds. **Go to last run** shows one chart per province:
the real cases up to December 2024 and the forecast for January to March 2025.

![A forecast per province](images/modeling-app/11-forecast.png)

**Import** writes the forecast back into DHIS2 as data, where dashboards and
maps can use it. It needs five data elements for the forecast's quantiles, of
the same period type as the forecast; the demo database has weekly ones
(`CHAP Dengue Cases (Any) - Weekly Quantile ...`), so a monthly forecast has
none to go into until someone creates them in DHIS2's Maintenance app.

## When something goes wrong

| What you see | What to do |
| --- | --- |
| Typing `Modeling` in the app menu finds nothing | `chaps dhis2 connect` has not run, or failed; run it and read its last line. |
| The model list does not have the model `chaps` enabled | `chaps status`: the model must be `registered`. If it is not, the line under the table says why. |
| `Oops! Sorry, an unexpected error`, or `Unnamed evaluation` rows, after the deployment was recreated | The browser holds a login to the DHIS2 that was removed: open DHIS2 again and log in. See [Troubleshooting](./troubleshooting.md#oops-sorry-an-unexpected-error-or-unnamed-evaluation-in-the-modeling-app). |
| A job in **Jobs** says it failed | `chaps jobs`, then `chaps jobs logs ID` with the id it prints: the model's own error is at the end. |

More on connecting DHIS2 and CHAP: [DHIS2](./dhis2.md). The Modeling App's own
guide is at
[chap.dhis2.org](https://chap.dhis2.org/chap-modeling-platform/modeling-app/).
