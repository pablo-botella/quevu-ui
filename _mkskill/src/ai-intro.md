---
mkskill:
  pos: 5
  in: ai*
---

# quevu-ui - agent notes

Cargo workspace producing the `quevu_ui` dynamic library: the window of
quevu, in Rust, behind a C interface (`include/quevu_ui.h`).

- `crates/quevu-ui-core`: platform-neutral. The console model, the
  lanes between host and UI thread, the order of tabs by weight and the
  table of texts. No dependencies. All the tests.
- `crates/quevu-ui-win32`: the Win32 backend, on `windows-sys`: the
  window and its tabs (`lib.rs`), the console view (`view.rs`) and the
  table of controls (`table.rs`).
- `crates/quevu-ui`: the `cdylib`. Only the C door; picks the backend by
  target system.

Only Windows has a backend. On other systems the library builds and
`quevu_ui_run` returns -1.
