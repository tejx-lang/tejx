# TejX Browser Wasm

This folder now contains a self-contained browser path for the compiler without touching the main compiler sources.

## Build

From the repo root:

```bash
cargo build --manifest-path wasm/Cargo.toml --target wasm32-unknown-unknown
```

## Run locally

Serve the repo root with any static server. For example:

```bash
python3 -m http.server 4173
```

Then open:

```text
http://localhost:4173/wasm/browser/
```

The page loads `wasm/target/wasm32-unknown-unknown/debug/tejxc_wasm.wasm`, lets you compile TejX source, and can run the generated wasm for the supported runtime subset.

## Smoke test

The browser/runtime bridge has a headless verification script:

```bash
node wasm/browser/smoke.mjs
```

Current smoke coverage:

- `tests/problems/balanced_parentheses.tx`: compile + run
- `tests/problems/benchmark.tx`: compile
- Browser runtime subset validated for `std:collections` stack/map paths, `std:math`, `std:time`, strings, arrays, conditionals, loops, and printing

## Known limits

- The exported browser `main` currently follows the generated `tejx_main` wrapper, so numeric return values are not surfaced reliably yet.
- Thread, fs, net, http, async timers, and promise-heavy runtime imports are still unsupported in the browser host.
- The benchmark sample compiles cleanly, but it is not part of the runtime smoke test because it is large and exercises a much wider host/runtime surface.
