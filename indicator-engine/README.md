# Flowsurface indicator engine

The Rust library and `flowsurface-indicator` CLI compile and run `.ind` chart
indicators. See [LANGUAGE.md](LANGUAGE.md) for authoring and
[indicator sources](../indicators/README.md) for installation.

The host supplies base-chart frames. Closed bars commit state; open-bar
previews evaluate from the last committed state. Execution is bounded by
runtime resource limits and has no arbitrary I/O or remote requests.

## CLI

Build once with Rust, then use the native executable:

```sh
cargo build --locked -p flowsurface-indicator-engine --bin flowsurface-indicator
target/debug/flowsurface-indicator new /tmp/my-indicators --id community.example --title "Example"
target/debug/flowsurface-indicator check /tmp/my-indicators
target/debug/flowsurface-indicator test /tmp/my-indicators
```

`new` creates `<short-name>.ind` without overwriting an existing file.
`check` accepts a file or flat directory and checks syntax, types, IDs,
filenames, and duplicate short names. Add `--namespace <prefix>` to enforce a
collection's namespace; the engine accepts any valid namespace.

`test` also runs default inputs against deterministic synthetic closed bars
and an open-bar preview, using the application's runtime limits for warmup and
execution. It catches runtime failures but does not verify indicator formulas,
every input configuration, long-history performance, dependency integration, or
rendered appearance. See [resource limits](LANGUAGE.md#resource-limits).

The engine is [Apache-2.0 licensed](LICENSE). The application and bundled
`core.*` sources are GPL-3.0-or-later; indicator source licenses are separate.
