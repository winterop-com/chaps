# chap-core built from its checkout

For testing a chap-core change the way it will ship: varde builds the chap-core
and worker images from your checkout and runs them with everything else it
normally runs (PostgreSQL, Valkey, the models, and OCS or DHIS2 if you add them
with `--with`).

```sh
varde init mychap --source ~/dev/chap-core --models default
cd mychap
varde up                     # builds both images from the checkout, then starts them
varde status
```

- **`compose.yml` comes from the checkout's own `compose.ghcr.yml`**, re-read on
  every `varde sync`, so the services match the code you are building.
- **Every `varde up` builds the checkout as it is now.** `compose.varde.yml`
  gives `chap` and `worker` a `build:` from the checkout and
  `pull_policy: build`. Docker's layer cache makes a build with nothing changed
  quick, and after a change `varde up` recreates what moved.
- **The images are `<project>-chap:checkout` and `<project>-worker:checkout`**,
  named after the deployment's compose project, so a build never overwrites a
  released `ghcr.io/dhis2-chap/...` tag on this machine, and two deployments
  built from two checkouts never overwrite each other's.
- **There is no pin.** `varde update` still moves the models and components, and
  refuses `--chap-tag`: check out the version you want in the checkout instead.

The checkout has to have `Dockerfile`, `Dockerfile.worker` and
`compose.ghcr.yml`, which every chap-core clone does; anything else is refused
before a file is written. The images are `linux/amd64`, so on an Apple Silicon
Mac the build runs under emulation and the first one takes a while.

To go back to a released chap-core, run `init` again without `--source`, with
the same `--with` and `--models` values you used. `.env` and the data are kept:

```sh
varde init . --force --models default
```

More: [chap-core from its checkout, with the models](./chap-core-on-host.md)
runs chap-core as a process instead, without building images.

All shapes: [Use cases](../use-cases.md).
