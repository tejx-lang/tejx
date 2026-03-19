# TejX Module System

TejX uses a static, compile-time module system. Imports are resolved during lowering, merged into the active compilation unit, and checked before code generation.

## Import Forms

TejX supports:

- relative imports such as `"./util.tx"` or `"../shared.tx"`
- standard library imports using the `std:` prefix
- named imports
- aliased imports
- default exports and default imports

Examples:

```tx
import { Map } from "std:collections";
import { helper as runHelper } from "./helper.tx";
import defaultThing from "./module.tx";
```

If a relative import omits `.tx`, the compiler appends it automatically.

## Resolution Rules

### Standard Library Imports

`std:` imports are resolved from the configured stdlib root:

- explicit `--stdlib-path`
- local project `lib/`
- installed SDK under `$HOME/.tejx/lib`

For example:

```tx
import { now } from "std:time";
```

resolves under:

```text
<stdlib-root>/std/time.tx
```

### Relative Imports

Relative imports are resolved from the importing file's directory and then canonicalized.

## Implicit Core Imports

For normal source files, lowering injects core library files automatically:

- `core/prelude.tx`
- `core/array.tx`
- `core/string.tx`

This happens before user imports are resolved, which is why common language helpers are available without manual imports in most source files.

The compiler avoids reinjecting those files when compiling the core library itself to prevent cycles.

## Export Rules

To make a declaration importable, mark it with `export`:

```tx
export function add(a: int, b: int): int {
    return a + b;
}

export class Point {
    x: int;
    y: int;
}

export default function mainHelper(): int {
    return 1;
}
```

Named imports require a matching exported symbol. Default imports require an `export default` declaration.

## Circular Dependency Detection

The compiler tracks an import stack while resolving modules. If file `A` imports `B` and `B` imports `A` back through the active chain, compilation fails with a circular dependency diagnostic.

Each imported file is processed once per compilation and canonicalized before insertion into the processed-files set.

## Lowering Behavior

Module contents are resolved before the main type-checking pass. After resolution:

- imported files become part of the merged program
- diagnostics are reported against the original source file paths
- duplicate imports of the same canonicalized file are skipped

This keeps the model simple: TejX does not use runtime module loading or package-manager-style search rules.
