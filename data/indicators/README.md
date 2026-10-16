# Bundled indicators

The `.ind` files in [sources/](sources/) are bundled with Flowsurface and licensed
under [GPL-3.0-or-later](LICENSE).

Use `core.<name>` IDs and `<name>.ind` filenames. After editing a source,
rebuild and restart the app.

See the [language reference](../../indicator-engine/LANGUAGE.md) for authoring
and [installation](../../indicators/README.md#install-and-run) for adding
indicators without rebuilding.

Validate the bundled collection from the repository root:

```sh
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test data/indicators/sources --namespace core
```
