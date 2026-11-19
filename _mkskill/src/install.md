---
mkskill:
  pos: 50
  in: readme
---

## Build

```
cargo build --release
```

The library is left in `target/release/` and the header is
`include/quevu_ui.h`. On Windows the MSVC toolchain of Rust is enough: no
resource compiler, no manifest and no extra SDK step.

```
cargo test -p quevu-ui-core
```

runs the tests of the logic every backend shares.
