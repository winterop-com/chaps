# Updating

This chapter is about updating a deployment: the model versions and the
chap-core release it runs. Updating the `chaps` binary itself is
`chaps self update`, which is in [Install](./install.md) and has nothing to do
with anything here.

## Three commands, one job each

| Command | What it does |
| --- | --- |
| `chaps up` | Starts what is on disk. Never changes a version. |
| `chaps update` | Fetches newer versions: moves the pins, pulls the images. Never touches a container. |
| `chaps restart` | Applies what was fetched to the services that are running. |

An update that restarted things by itself would decide for you when your
deployment goes down, and an update that started a stopped deployment would be
a deployment nobody asked for. So `update` ends by telling you which of the
other two to run, and you run it when it suits you:

```sh
chaps update     # ... restart needed: chap, worker (run `chaps restart`)
chaps restart    # recreated: chap, worker; unchanged: postgres, redis
```

## What moves a pin

Model pins only move in two ways: `chaps models enable ID` (with `--channel`
or `--version`) and `chaps update`. Nothing else, `up`, `restart`, `expose` and
`unexpose` included, changes the version a model runs.

| Command | Moves a pin? |
| --- | --- |
| `chaps up`, `chaps up --pull` | No. Starts what is on disk. |
| `chaps restart` | No. Recreates containers from the files as they are. |
| `chaps sync` | No. Renders the compose files from `.chaps/`. |
| `chaps docker pull` | No. Downloads the pinned images. |
| `chaps models expose`, `chaps models unexpose` | No. Only the `ports:` of one overlay. |
| `chaps models enable ID --channel C` | Yes, for that one model. |
| `chaps models enable ID --version X` | Yes, to exactly `X`. |
| `chaps models add SOURCE` | Yes, for the model it adds: to the newest published build of the branch, or to the tag or digest it was given. |
| `chaps update` | Yes: every channel-following model, every model added from a repository URL, and chap-core. |

## `chaps update`

```sh
chaps update [--dry-run] [--pin-chap-core]
```

`chaps update` fetches the registry from the network: no cache, no fallback. It
fails under `--offline`, `--dry-run` included, because a plan made from a stale
catalogue is not a plan.

A run is four steps, printed in that order:

1. **The plan.** One row per enabled model, one for chap-core and one per
   optional component, saying what each is pinned to and what it would move
   to. For every model that follows a channel the channel is re-resolved; a
   pin that moved is recorded in `.chaps/models.yaml` and its
   `# <ID>_IMAGE_TAG=` comment in `.env` is updated. Models enabled with
   `--version` are listed as pinned and skipped.

   A model added with `chaps models add` is resolved against its own
   repository rather than the catalogue, so its row reads in image tags:

   ```text
     chapkit_ghr_model  sha-1eb8cf1 -> sha-b1d6c31
   ```

   One added from a repository URL follows that repository's default branch:
   the row moves when a newer commit has a published `sha-` build, and the new
   pin is written to `.chaps/models-manual.yaml` as well, so a later
   `models enable` brings back what is running now. One added from an image
   reference is `pinned, skipped`. A repository or registry that will not
   answer is a warning and a row reading `unchanged (could not check)`: the
   pin stays where it is, and the rest of the update goes ahead.
2. **The sync**, which renders the compose files from `.chaps/`.
3. **The pull**: `docker compose pull`, with docker's own output.
4. **One closing line**, below.

## The closing line

It answers the only question a finished update leaves: is there anything to do
now? There are four answers, and every run ends on exactly one of them.

Nothing moved, no new image arrived, and nothing running is out of date:

```text
already up to date
```

Running services are behind what the files now say, which is the case the whole
command is shaped around:

```text
updated 1 model pin; restart needed: chap, worker (run `chaps restart`)
```

Something moved and there is nothing running for it to be ahead of:

```text
updated chap-core v2.3.0 -> v2.3.1; CHAP is not running, the new versions start with `chaps up`
```

Something moved, the stack is up, and none of it was affected:

```text
updated 1 model pin; nothing needs a restart
```

The first clause names what actually moved: the model pins, chap-core's tag,
and the services the pull brought a genuinely different image for. A moving
tag that pulled the same image this machine already had is not a change and is
not named, which is why a second `chaps update` on a `latest` deployment says
`already up to date` rather than claiming to have done something.

## What "needs a restart" means

For every running container, `update` asks docker two questions:

- Is the image it is running still the image its reference points at, now that
  the pull has finished? A moving tag such as OCS's `main` answers no as soon
  as upstream publishes.
- Is the configuration hash Compose stamped on it
  (`com.docker.compose.config-hash`) still the hash Compose computes for the
  files as they are now? A pin that moved, a port that changed, a secret that
  was rotated: all of them answer no.

Either answer being no means that container is running something this project
no longer describes, and it is named in the closing line. A question docker
would not answer is never counted as a difference, so an unreachable daemon
cannot turn into "restart everything".

## `chaps restart`

```sh
chaps restart [SERVICE..] [--all]
```

`chaps restart` is `docker compose up -d`, which recreates only the containers
that no longer match the files and leaves the rest running. It changes no file,
no pin and nothing in `.chaps/`, and it never syncs: it applies what is already
on disk.

```sh
chaps restart                       # the whole project; compose decides
chaps restart chap worker           # only these, without their dependencies
chaps restart --all                 # recreate everything, changed or not
chaps restart --all my-model        # recreate this one, changed or not
```

It ends with what it did:

```text
recreated: chap, worker; unchanged: postgres, redis
```

or, when it turned out there was nothing to apply, `nothing needed a restart`.

A service name the project does not have is refused with the list of the names
it does have. A deployment with nothing running at all is not this command's
job, and it says which command it is, then exits non-zero:

```text
CHAP is not running; start it with `chaps up`
```

`--all` is for the case where nothing has changed and a restart is still what
you want. A model whose container is up but which never registered with
chap-core is the example, and `chaps status` prints it as a hint:

```sh
chaps restart --all chapkit-rwanda-malaria-bym-model
```

## `--dry-run`

`chaps update --dry-run` prints the plan and stops: nothing is pulled and
nothing is written. It still needs the network, and it says what it would do:

```text
would update 1 model pin; nothing written (run `chaps update` to do it)
```

It says nothing about restarting. The images were not pulled and the files were
not written, so there is nothing yet for a running container to be out of step
with.

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
