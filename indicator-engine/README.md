# Flowsurface indicator engine

The Rust library and `flowsurface-indicator` CLI compile and run `.ind` chart
indicators. See [LANGUAGE.md](LANGUAGE.md) for authoring and
[indicator sources](../indicators/README.md) for installation.

## CLI

Build once with Rust, then use the native executable:

```sh
cargo build --locked -p flowsurface-indicator-engine --bin flowsurface-indicator
target/debug/flowsurface-indicator new /tmp/my-indicators --id community.example --title "Example"
target/debug/flowsurface-indicator check /tmp/my-indicators
target/debug/flowsurface-indicator test /tmp/my-indicators
```

`new` creates `<short-name>.ind` without overwriting an existing file.
`check` accepts a file or recursive collection directory and checks syntax, types, IDs,
filenames, and duplicate full IDs. Add `--namespace <prefix>` to enforce a
collection's namespace.

`test` also runs default inputs on synthetic bars and an open-bar preview using
the [application limits](LANGUAGE.md#resource-limits). Verify formulas, other
input settings, performance on real histories, and chart appearance separately.

The engine is [Apache-2.0 licensed](LICENSE). The application and bundled
indicator sources are GPL-3.0-or-later; indicator source licenses are separate.
