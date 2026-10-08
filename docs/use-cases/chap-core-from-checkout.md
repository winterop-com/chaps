# chap-core built from its checkout

For testing a chap-core change the way it will ship: varde builds the chap-core
and worker images from your checkout and runs them with everything else it
normally runs (PostgreSQL, Valkey, the models, and OCS or DHIS2 if you add them
with `--with`).

Replace `~/dev/chap-core` with the path of your chap-core checkout:

```sh
varde init mychap --source ~/dev/chap-core --models default
cd mychap
varde up                     # builds both images from the checkout, then starts them
varde status
```

The images are `linux/amd64`, so on an Apple Silicon Mac the build runs under
emulation. The first `varde up` pulls the base images and builds both images,
which took about five minutes on an Apple Silicon Mac. It prints the full
docker build log, and ends with the services it started:

```text
started chap, chapkit-ewars-model, postgres, redis, worker
```

```text
chap-core   up   http://localhost:8700   2.5.0.dev0   auth: off

MODEL                STATE       REACH          LAST PING
chapkit-ewars-model  registered  via chap-core  14s ago

1 model registered
```

It worked when `varde status` shows chap-core `up` and the model `registered`.
The version is the one that your checkout reports.

- **`compose.yml` comes from the checkout's own `compose.ghcr.yml`**, re-read on
  every `varde sync`, so the services match the code you are building.
- **Every `varde up` builds the checkout as it is now.** `compose.varde.yml`
  gives `chap` and `worker` a `build:` from the checkout and
  `pull_policy: build`. Docker's layer cache makes a build with nothing changed
  take some seconds. After a change in the code, the build installs the
  dependencies again (about a minute on an Apple Silicon Mac), and `varde up`
  recreates only what moved: `started chap, worker`.
- **The images are `<project>-chap:checkout` and `<project>-worker:checkout`**,
  named after the deployment's compose project (such as
  `mychap-77c0c2-chap:checkout`), so a build never overwrites a released
  `ghcr.io/dhis2-chap/...` tag on this machine, and two deployments built from
  two checkouts never overwrite each other's. The worker image is about 12 GB.
- **There is no pin.** `varde update` still moves the models and components, and
  refuses `--chap-tag`: check out the version you want in the checkout instead.

The checkout has to have `Dockerfile`, `Dockerfile.worker` and
`compose.ghcr.yml`, which every chap-core clone does; anything else is refused
before a file is written (see
[`is not a chap-core checkout`](../troubleshooting.md#is-not-a-chap-core-checkout)).

## Go back to a released chap-core

`.env` and the data are kept. Do these steps in the deployment directory:

1. Stop the deployment:

   ```sh
   varde down
   ```

2. Run `init` again without `--source`, with the same `--with` and `--models`
   values you used:

   ```sh
   varde init . --force --models default
   ```

   It prints the released tag, such as `chap-core: v2.4.0, API on
   http://localhost:8700`.
3. In `.env`, change the line `CHAP_IMAGE_TAG=checkout` to that tag, such as
   `CHAP_IMAGE_TAG=v2.4.0`. `init` never rewrites `.env`. Without this change,
   compose looks for `ghcr.io/dhis2-chap/chap-core:checkout`, and `varde up`
   fails with `not found`.
4. Start the deployment:

   ```sh
   varde up
   ```

It worked when `varde status` shows the released version, such as `2.4.0`. If
the checkout added a database migration, the released chap-core may not start
on that data.

The images built from the checkout stay on this machine. To get the disk space
back, remove them with `docker image rm <project>-chap:checkout
<project>-worker:checkout`.

More: [chap-core from its checkout, with the models](./chap-core-on-host.md)
runs chap-core as a process instead, without building images.

All shapes: [Use cases](../use-cases.md).
