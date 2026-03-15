# Tejx Module System & Resolution Architecture

Tejx provides a simple, deterministic, static module resolution system built directly into the compiler's Lowering phase (`src/lowering.rs`). It guarantees cyclic-dependency protection and enforces strict semantic `export` checking before variables or types are allowed to cross module boundaries.

## 1. Import Syntax Variations

The compiler supports ECMAScript-style structured imports to acquire external resources:

### Default Imports

Used when a module provides a single primary class or function.

```typescript
import Controller from "./controller.tx";
```

_Compiler Enforcement_: Checks that the target file contains an explicit `export default` statement. If missing, it throws `E0203: Module has no default export`.

### Named Imports

Used to pick specific functions, types, or variables from a target module.

```typescript
import { Model, validate } from "./models.tx";
```

_Compiler Enforcement_: Reads the parsed Abstract Syntax Tree (AST) of `models.tx`. If either `Model` or `validate` does not have an `export` modifier attached to their declaration, it throws `E0202: 'Model' is not exported from './models.tx'`.

### Aliased Named Imports

Used to prevent namespace collisions when multiple files export identically named structures.

```typescript
import { init as initDatabase } from "./database.tx";
import { init as initServer } from "./server.tx";
```

_Compiler Action_: The AST seamlessly re-binds references to the target alias before injecting the file body into the compiler context.

---

## 2. Path Resolution Semantics

Tejx does not rely on complex node_modules traversal algorithms. It strictly supports two resolution paradigms: Relative and Standard Library paths.

### Standard Library (`std:`)

Prefixing a path with `std:` tells the compiler to bypass the local working directory and look directly inside the Tejx compiler installation's `stdlib/` folder.

```typescript
import { Map, Set } from "std:collections";
import { assert } from "std:testing";
```

_(Expands to: `<TEJX_ROOT>/stdlib/collections.tx`)_

### Relative Paths (`./` or `../`)

Used to traverse the developer's local project boundaries.

```typescript
import { config } from "./utils/config.tx";
import shared from "../shared.tx";
```

_(Expands iteratively relative to the calling file's canonicalized directory path)_

_Note:_ If an import omits the `.tx` file extension, the Tejx compiler implicitly appends it during resolution.

---

## 3. The Prelude

Tejx enforces an implicit, invisible auto-import mechanism to inject core runtime logic, memory allocators, and basic constants (like `None`).

**The Auto-Injection Rule:**
During the AST Lowering phase, the compiler inspects every file. If the file is **not** named `prelude.tx` and **not** `runtime.tx`, it automatically pushes this exact statement to the absolute top of the AST:

```typescript
import "std:prelude";
```

This ensures developers never have to manually import `Array_concat`, `string_length`, or intrinsic string parsing modules while maintaining a pure logical separation of standard mechanics within the Tejx source engine.

---

## 4. Circular Dependency Protection

Tejx module resolution happens synchronously in a depth-first traversal during the lowering pass.

To prevent infinite loops when two files reference each other, the compiler tracks absolute file paths in `import_stack`.
If file A imports B, and B attempts to import A again, the `import_stack` trap catches the matching canonicalized path and halts compilation:
`E0204: Circular dependency detected: circularly imported here`.

Processed files are permanently appended to a `processed_files` HashSet to guarantee exactly-once parsing, preventing duplicated code bloat or conflicting re-declarations when multiple files pull from the same utility script.

---

## 5. Exports Lifecycle

To make an entity legally accessible via `import`, developers must preface the declaration with `export`.

Supported Export Types:

```typescript
export function calculate() {}
export class Entity {}
export let MAX_RETRIES: int = 5;

// Default
export default class Engine {}
```

During resolution, Tejx slices the external module's AST and merges it directly into the current execution chunk, seamlessly preserving type information and execution traces across files without requiring a discrete linkage stage.
