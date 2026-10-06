# A DHIS2 from varde, with a chap-core elsewhere

For chap-core development with the Modeling App in front of it: varde runs a
DHIS2 (and any models), and chap-core runs somewhere else, usually from its own
checkout on this machine.

```sh
varde init lab --chap-core-url http://localhost:8000 --with dhis2
cd lab
varde up                     # DHIS2's first start takes several minutes
varde status                 # wait for the dhis2 line to say up
varde dhis2 connect
varde open dhis2
```

`--chap-core-url` records the chap-core elsewhere (see
[chap-core from its checkout, with the models](./chap-core-on-host.md)), and
for the DHIS2 beside it:

- **The route goes to that chap-core as the DHIS2 container reaches it.** A
  `localhost` URL becomes `host.docker.internal`, and the DHIS2 service maps
  that name to the host gateway.
- **`dhis2/dhis.conf` allows it.** A scaffolded file allows every http and
  https target in `route.remote_servers_allowed`, so the chap-core elsewhere is
  covered wherever it runs.
- **chap-core has to listen on more than the loopback** (`0.0.0.0`), since the
  route arrives from a container.

It worked when `varde dhis2 connect` finishes without `error:` and the Modeling
App opens in DHIS2.

More: [DHIS2](../dhis2.md), [A chap-core elsewhere](../components.md#a-chap-core-elsewhere).

All shapes: [Use cases](../use-cases.md).
