# Updating

This chapter is about updating a deployment: the model versions and the
chap-core release it runs. Updating the `chaps` binary itself is
`chaps self update`, which is in [Install](./install.md) and has nothing to do
with anything here.

## What moves, and when

Model pins only move in two ways: `chaps models enable ID` (with `--channel`
or `--version`) and `chaps update`. Nothing else, `up`, `expose` and `unexpose`
included, changes the version a model runs.

| Command | Moves a pin? |
| --- | --- |
| `chaps up`, `chaps up --pull` | No. Starts what is on disk. |
| `chaps sync` | No. Renders the compose files from `.chaps/`. |
| `chaps docker pull` | No. Downloads the pinned images. |
| `chaps models expose`, `chaps models unexpose` | No. Only the `ports:` of one overlay. |
| `chaps models enable ID --channel C` | Yes, for that one model. |
| `chaps models enable ID --version X` | Yes, to exactly `X`. |
| `chaps update` | Yes: every channel-following model, and chap-core. |

## `chaps update`

```sh
chaps update [--dry-run] [--no-restart] [--pin-chap-core]
```

`chaps update` fetches the registry from the network: no cache, no fallback. It
fails under `--offline`, `--dry-run` included, because a plan made from a stale
catalogue is not a plan.

For every model that follows a channel it re-resolves the channel. A pin that
moved is recorded in `.chaps/models.yaml` and its `# <ID>_IMAGE_TAG=` comment
in `.env` is updated. Models enabled with `--version` are listed as pinned and
skipped. Then it syncs and runs `docker compose pull`.

## Whether anything restarts

That depends on whether CHAP is running, and it is the one rule worth
remembering: **`update` never starts CHAP when it is stopped.**

- With containers running, `docker compose up -d` recreates the services whose
  pins moved.
- With nothing running, `update` stops after the pull and says
  ``CHAP is not running; run `chaps up` to start CHAP with the new versions``.
  Starting a deployment somebody stopped is not an update's business, and an
  `up -d` that did it would be a deployment nobody asked for.

`--no-restart` stops after the pull either way.

## `--dry-run`

`chaps update --dry-run` prints the plan, including which of those two it would
do, and writes nothing. It still needs the network.

## chap-core's own pin

chap-core's pin moves in the same run.

`chaps init` resolves the default `--chap-tag latest` to the release it points
at today (`v2.3.1`, say) and writes that into `.env` and `.chaps/project.yaml`,
so a deployment is reproducible rather than following a tag that changes under
it.

`chaps update` then looks up the newest release, and when there is a newer one
it downloads the `compose.ghcr.yml` that release publishes, moves
`chap_image_tag`, and rewrites the single active `CHAP_IMAGE_TAG=` line in
`.env`. Only that line, and only when it still says what the project recorded.
A value you pinned yourself, or the commented placeholder, is left alone with a
warning saying what CHAP will actually run.

## Moving tags

`--chap-tag latest|master|dev` keeps a moving tag instead. There is nothing to
move, so `update` only re-pulls it. Compose never re-pulls a tag it already
has, so the image is refreshed by `chaps docker pull`, `chaps update` or
`chaps up --pull`.

```sh
chaps update --pin-chap-core
```

converts such a tag into a release pin, the way `init` does by default.

A tag that is neither a release nor a moving one, a `sha-` build for instance,
is never moved.

## Without the network

`chaps init --offline`, or an `init` whose fetch fails, keeps the tag exactly as
given and renders `compose.yml` from the copy of `compose.ghcr.yml` compiled
into the binary, warning on stderr both times. With `--chap-tag latest` that
means the deployment does follow the moving tag rather than a resolved release.

An already cached `.chaps/compose.chap-core.<tag>.yml` is reused rather than
re-downloaded, so a second `init --force` at the same tag works offline.

## Pulling without updating

```sh
chaps docker pull      # download the pinned images; no files change
chaps up --pull        # docker compose up --pull always
```

`chaps up --pull` is how a moving chap-core tag gets refreshed on the way up.
Neither of these asks the marketplace anything, so neither moves a pin.
