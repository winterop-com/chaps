# Several model services side by side

The same, with more than one model: each gets its own port and its own data
volume, and they do not depend on each other.

```sh
varde init models --only none --models chapkit_ewars_model,auto_arima_chapkit
cd models
varde up
varde models list            # the PORT column says where each one answers
```

It worked when `varde status` shows every model as `up`, each at its own
`http://localhost:<port>`. The ids are the ones `varde models list` prints.

Without a folder, `varde run` does the same one model at a time:
[Any number of model services, no folder](./models-with-run.md).

More: [A chapkit model service on its own](./model-alone.md).

All shapes: [Use cases](../use-cases.md).
