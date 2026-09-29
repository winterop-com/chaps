# A DHIS2 from chaps, with a chap-core elsewhere

For chap-core development with the Modeling App in front of it: chaps runs a
DHIS2 (and any models), and chap-core runs somewhere else, usually from its own
checkout on this machine.

```sh
chaps init lab --chap-core-url http://localhost:8000 --with dhis2
cd lab
chaps up                     # DHIS2's first start takes several minutes
chaps status                 # wait for the dhis2 line to say up
chaps dhis2 connect
chaps open dhis2
```

`--chap-core-url` records the chap-core elsewhere (see
[chap-core from its checkout, with the models](./chap-core-on-host.md)), and
for the DHIS2 beside it:

- **The route goes to that chap-core as the DHIS2 container reaches it.** A
  `localhost` URL becomes `host.docker.internal`, and the DHIS2 service maps
  that name to the host gateway.
- **`dhis2/dhis.conf` allows it.** A freshly scaffolded file lists
  `http://chap:8000` and the chap-core elsewhere in
  `route.remote_servers_allowed`. The file is yours once written: if it
  predates `--chap-core-url`, add the origin (`http://host.docker.internal:8000`)
  there yourself and run `chaps restart --all dhis2`.
- **chap-core has to listen on more than the loopback** (`0.0.0.0`), since the
  route arrives from a container.

It worked when `chaps dhis2 connect` finishes without `error:` and the Modeling
App opens in DHIS2.

More: [DHIS2](../dhis2.md), [A chap-core elsewhere](../components.md#a-chap-core-elsewhere).

All shapes: [Use cases](../use-cases.md).
