# Jobs and the API

chap-core does everything slow on a worker: building a dataset, backtesting a
model, making a prediction. Each one of those is a *job*, and `/v1/jobs` is
where a deployment records what it has been asked to do. The Modeling App
starts jobs and watches them; `chaps jobs` is the same view from a terminal,
and `chaps api` is how a job is started from one.

```sh
chaps jobs                    # what this deployment has been doing
chaps jobs logs <id>          # why one of them failed
chaps api GET /v1/jobs        # the same list, unrendered
```

## What `chaps jobs` reports

One row per job, newest first, with a running job ahead of a finished one that
started in the same second:

```text
ID           TYPE               NAME                                   STATUS   STARTED  DURATION
f293520a...  create_prediction  eval-make-prediction-ewars             SUCCESS  25m ago  15s
ed3a0719...  create_prediction  eval-setup-run-ewars                   SUCCESS  25m ago  15s
e3e9f243...  create_backtest    eval-chapkit-rwanda-malaria-bym-model  FAILURE  30m ago  10s
501d5b5d...  create_backtest    eval-chapkit-ghr-model                 SUCCESS  33m ago  2m 9s
0fa57ec9...  create_dataset     eval-chapkit-5ou-202101-202312         SUCCESS  47m ago  2s

8 jobs: 7 done, 1 failed
  run `chaps jobs logs e3e9f243-2492-47cf-9d02-7de886154f70` to see why
```

| Column | What it holds |
| --- | --- |
| `ID` | The job id, cut to its first eight characters while those tell the listed jobs apart, and printed in full when they do not. Every subcommand takes the short form. |
| `TYPE` | chap-core's own job type: `create_dataset`, `create_backtest`, `create_prediction`. |
| `NAME` | The name whoever started the job gave it, which is the only field a person chose. |
| `STATUS` | The worker's state, unchanged. See the table below. |
| `STARTED` | How long ago the job began, or `queued` for one that has not. |
| `DURATION` | How long it took, for a job that has finished. `-` while it runs. |

`--status` filters (repeat it for more than one), `--type` filters by job type
and `--limit` keeps the newest N after the ordering, so `chaps jobs --limit 5`
is the five most recent. The filters work with or without the `list` verb:
`chaps jobs --status FAILURE` and `chaps jobs list --status FAILURE` are the
same command.

`--json` hands back the list chap-core sent, in the order the table was in,
with the fields this CLI does not render still in it.

## The status values

chap-core passes its worker's states through untouched, so the set is Celery's.
`chaps jobs` groups them into the four things the closing line counts:

| Status | Counted as | Meaning |
| --- | --- | --- |
| `PENDING` | running | Queued; no worker has picked it up. The `STARTED` cell reads `queued`. |
| `STARTED` | running | A worker is on it. |
| `RETRY` | running | It failed and the worker is trying again. |
| `SUCCESS` | done | Finished, and its result is in chap-core's database. |
| `FAILURE` | failed | It raised. The reason is in the log, not here. |
| `REVOKED` | cancelled | Stopped by `chaps jobs cancel` or by the Modeling App. |

A status this CLI has never seen is counted as running: a new state is far more
likely to be a new way of being in flight than a new way of being finished.

## Where the error text lives

A job description carries a status and nothing else, so `FAILURE` says only
that something went wrong. The message is in `GET /v1/jobs/{id}/logs`, which
holds chap-core's own log lines, then the model container's stdout, then its
stderr behind a `--- stderr ---` line, and finally the Python traceback. That
is the only place the real error text exists - it is not in the job list, and
it is not in `chaps logs`, because the model wrote it inside a chapkit job that
chap-core polled.

```sh
chaps jobs logs e3e9f243 --tail 20
```

```text
job e3e9f243-2492-47cf-9d02-7de886154f70 FAILURE (create_backtest eval-chapkit-rwanda-malaria-bym-model)
...
sh: 1: /usr/local/lib/R/site-library/INLA/bin/linux/64bit/inla.mkl.run: Permission denied
Error in inla.inlaprogram.has.crashed() :
  The inla-program exited with an error. Unless you interupted it yourself, please rerun with verbose=TRUE and check the output carefully.
Execution halted

stderr: The inla-program exited with an error. Unless you interupted it yourself, please rerun with verbose=TRUE and check the output carefully.
```

The log itself is the only thing on stdout, so it pipes into `grep` or `less`
unchanged. The line naming the job is on stderr, and so is the `stderr:` hint -
the last line of the `--- stderr ---` section that names an error, skipping the
warnings and the runtime's `Execution halted`. It is a best-effort read of
whatever the model wrote, printed only for a job that failed; the log above it
is the authority.

`--tail N` cuts the printed log to its last N lines. The hint is still read
from the whole log, so shortening the output never hides the reason.

## Stopping and forgetting a job

```sh
chaps jobs cancel <id>   # ask the worker to stop
chaps jobs delete <id>   # drop a finished job from the list
```

Both print the sentence chap-core answers with. `delete` on a job that is still
running is refused by chap-core with a 400, which `chaps` reports as
`job <id> is still running; cancel it first`.

## `chaps api`: one request, authenticated

```text
chaps api METHOD PATH [--data JSON|@FILE|-] [--url URL] [--raw] [--timeout SECONDS]
```

`chaps api` is `curl` with the three things that are tedious to get right
filled in: the base URL of *this* deployment, the `Authorization: Bearer`
header from its `.env`, and an exit code that distinguishes CHAP being down
from CHAP saying no.

| Part | Rule |
| --- | --- |
| `METHOD` | `GET`, `POST`, `PUT`, `PATCH` or `DELETE`, in any case. Anything else is a usage error before a byte is sent. |
| `PATH` | Starts with `/`, and may carry a query string: `/v1/analytics/evaluation-entry?backtestId=3&quantiles=0.5`. |
| `--data` | The JSON body: inline, `@path` to read a file, or `-` to read stdin. It is checked as JSON here and then sent byte for byte, with `Content-Type: application/json`. |
| `--url` | Talk to another chap-core instead of this deployment's. Works outside a project, where the token comes from `CHAP_API_TOKEN` in the environment. |
| `--raw` | Write the body through untouched, without any of the rendering below. |

The answer goes to stdout, rendered the way it reads best:

- A JSON object or list is pretty-printed, two spaces per level.
- A JSON *string* is printed as its text, so
  `chaps api GET /v1/jobs/<id>/logs` reads like a log and
  `chaps api GET /v1/jobs/<id>` prints `SUCCESS` rather than `"SUCCESS"`.
  `--json` turns that off, because a caller piping into a parser wants the
  document.
- Anything that is not JSON is written as it arrived.

Exit codes are the point of the command being a command rather than a shell
function:

| Code | Meaning |
| --- | --- |
| 0 | A 2xx. The body is on stdout; an empty body gets its status line on stderr so the run is never silent. |
| 1 | A 4xx or 5xx. The status line (`HTTP 404 Not Found`) is on stderr and the body is *still* on stdout, because a 422 from chap-core names the field it did not like. |
| 2 | chap-core could not be reached at all, with the sentence [`chaps status`](./status.md) uses, or the command line was wrong (an unknown method, a path with no leading slash, a `--data` that is not JSON). |

`-v` narrates the request line and the headers that were sent. The token's
value never appears - the line reads `Authorization: Bearer <token>` - so a
`-v` transcript is safe to paste into an issue. `-d` adds the bodies.

## The evaluation flow, end to end

The four steps behind one number in the Modeling App. Every one of them is a
job, and the job id is what carries you from one step to the next.

```sh
# 1. a dataset: observations plus the geojson they belong to
chaps api POST /v1/analytics/make-dataset --data @dataset.json
# -> {"id": "0fa57ec9-11b6-47a8-96ec-6c22580bbdf5"}   (a job id)

# 2. the row that job wrote, once it says SUCCESS
chaps jobs show 0fa57ec9            # Database result  1  -> /v1/crud/datasets/1

# 3. a backtest of one configured model against that dataset
chaps api POST /v1/analytics/create-backtest \
  --data '{"name":"eval-ewars","modelId":14,"datasetId":1,"nPeriods":3,"nSplits":3,"stride":1}'
# -> {"id": "59159ad4-893b-4bbf-a855-70a38c3fb787"}

# 4. poll it, then read what it produced
chaps api GET /v1/jobs/59159ad4-893b-4bbf-a855-70a38c3fb787     # SUCCESS
chaps jobs show 59159ad4                                        # Database result  3
chaps api GET /v1/crud/backtests/3 | head -30
chaps api GET /v1/visualization/metrics/3
chaps api GET '/v1/analytics/evaluation-entry?backtestId=3&quantiles=0.1&quantiles=0.5&quantiles=0.9'
```

Which model ids exist is `chaps api GET /v1/crud/models` (or the Modeling App's
model list); `chaps status` is what says whether the model service behind an id
has registered at all, which is the first thing to check when a backtest fails
immediately.

A prediction is the same shape one step further on: `POST
/v1/analytics/make-prediction` for a one-off, or a prediction setup promoted
from a backtest and run on a schedule.

```sh
chaps api POST /v1/crud/prediction-setups --data '{"backtestId":3,"name":"ewars-monthly"}'
chaps api POST /v1/crud/prediction-setups/1/run --data @run.json
chaps api GET  /v1/crud/prediction-setups/1
chaps api PATCH /v1/crud/prediction-setups/1 --data '{"scheduleCronExpression":"17 3 * * 1"}'
```

`chaps api GET /openapi.json` is the whole contract, straight from the running
deployment, which is the version that matters rather than the one in any
document.

## The token in a script

```sh
TOKEN=$(chaps auth token)
curl -H "Authorization: Bearer $TOKEN" http://localhost:8000/v2/services
```

`chaps auth token` prints the token and nothing else: no label, no
abbreviation, one trailing newline. On a deployment with authentication off it
prints nothing at all on stdout, says so on stderr and exits 1, so a script
that captured an empty string stops instead of carrying on with an empty
header. `--json` gives `{"token": "..."}`, or `{"token": null}` when there is
none.

For reading rather than capturing, `chaps auth show --reveal` prints the same
value inside the report that says what it protects. See
[Authentication](./auth.md).

Outside a deployment, `chaps api --url` reads `CHAP_API_TOKEN` from the
environment, which is the same variable `.env` sets:

```sh
CHAP_API_TOKEN=$(chaps -C ~/mychap auth token) \
  chaps api GET /v1/jobs --url https://chap.example.org
```

## What talks to what

| Command | Requests it makes |
| --- | --- |
| `chaps jobs list` | `GET /v1/jobs`, with `status` and `type` as query parameters. |
| `chaps jobs show` | `GET /v1/jobs`, then `GET /v1/jobs/{id}` for the current status and `GET /v1/jobs/{id}/database_result` for a finished one. |
| `chaps jobs logs` | `GET /v1/jobs` to resolve the id, then `GET /v1/jobs/{id}/logs`. |
| `chaps jobs cancel` | `POST /v1/jobs/{id}/cancel`. |
| `chaps jobs delete` | `DELETE /v1/jobs/{id}`. |
| `chaps api` | Exactly the one request you asked for, and nothing else. |

Every one of them sends the API token when `.env` sets one, and every one of
them reports an unreachable chap-core with the sentence `chaps status` uses and
exit code 2. An id prefix is resolved against the job list, and `-v` says which
full id it landed on.
