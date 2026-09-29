# A model image you built yourself

For testing a model the way it will really run: build its image on this
machine and run it in a chaps deployment, registered with chap-core, without
publishing it anywhere first.

In the model's checkout:

```sh
docker build --platform linux/amd64 -t my-model:dev .
```

Then in the deployment:

```sh
chaps models add my-model:dev
chaps up
chaps status                 # the model registered
chaps models test my_model
```

A reference with no registry host (`my-model:dev`, `me/my-model:dev`) is read
as an image in the local image store:

- **It has to be there already.** `models add` reads the user and data
  directory from the local image, and refuses with the build command if it is
  missing.
- **It is never pulled.** The overlay says `pull_policy: never`, so `chaps up`
  starts what is in the store and `chaps update` skips it.
- **`--platform linux/amd64`** matches the platform every model overlay pins.
  On an Apple Silicon Mac the build runs under emulation and takes longer.
- **Rebuilding** with the same tag and running `chaps restart` picks up the new
  build.

## A local build of a marketplace model

If the model is one the marketplace already lists (you are working on it),
give the local build the marketplace model's service id, so it registers
under the name chap-core expects:

```sh
chaps models disable chapkit_ewars_model          # if it is enabled
chaps models add ewars:dev --service-id chapkit-ewars-model
chaps up
```

The two cannot run at the same time: enabling one while the other holds the
service is refused, naming the `chaps models disable` that frees it. To go back
to the published image:

```sh
chaps models remove ewars_dev
chaps models enable chapkit_ewars_model
chaps up
```

More: [Models outside the marketplace](../models.md#models-outside-the-marketplace).

All shapes: [Use cases](../use-cases.md).
