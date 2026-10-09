# Bundled indicators

The `.ind` files in [sources/](sources/) are bundled with Flowsurface and licensed
under [GPL-3.0-or-later](LICENSE).

The data crate embeds these sources at build time and compiles and registers
them at startup. They ship inside the app binary through its data dependency;
no external source directory or separate download is needed. After editing a
bundled source, rebuild and restart the app.

The `core.*` IDs identify the publisher, not the distribution method.
The loader marks this collection as bundled independently of its IDs.
Optional catalog indicators live separately in [`indicators/sources/`](../../indicators/sources/)
and are installed from files; see [installation](../../indicators/README.md#install-and-run).

Validate the bundled collection from the repository root:

```sh
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test data/indicators/sources --namespace core
```
