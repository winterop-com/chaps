# Several model services side by side

The same, with more than one model: each gets its own port and its own data
volume, and they do not depend on each other.

```sh
chaps init models --only none --models chapkit_ewars_model,auto_arima_chapkit
cd models
chaps up
chaps models list            # the PORT column says where each one answers
```

All shapes: [Use cases](../use-cases.md).
