# Flowsurface indicator catalog

Optional [Apache-2.0](LICENSE) indicators in the flat `sources/` directory.
Use `catalog.<name>` IDs and `<name>.ind` filenames. Flowsurface does not
bundle this collection.

See [installation](../README.md#install-and-run), the
[language reference](../../indicator-engine/LANGUAGE.md), and
[CLI workflow](../../indicator-engine/README.md#cli) for creating a source.
Use the [extension guidance](../../indicator-engine/LANGUAGE.md#extending-the-scripting-boundary)
when a source needs a capability the current language cannot express cleanly.

Validate the collection from the repository root:

```sh
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- check indicators/catalog/sources --namespace catalog
cargo run --locked -p flowsurface-indicator-engine --bin flowsurface-indicator -- test indicators/catalog/sources --namespace catalog
```

Contributions require Apache-2.0 licensing and a DCO sign-off; see
[CONTRIBUTING.md](CONTRIBUTING.md).
