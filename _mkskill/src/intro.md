---
mkskill:
  pos: 10
  in: readme
---

# quevu-ui

The window of [quevu](https://github.com/pablo-botella/quevu): the
switches of an application (a global one and one per unit) and its screens
(a tab per log, each a pseudo-console), plus a menu and an icon in the
notification area to hide in. Nothing more: quevu wants a window for
applications, not a GUI toolkit.

It is written in Rust against the native toolkit of each system and ships
as a dynamic library with a C interface, so any language can use it:

| System | Toolkit | Library | State |
|---|---|---|---|
| Windows | Win32 | `quevu_ui.dll` | Working |
| Linux | GTK | `libquevu_ui.so` | Not written yet |
| macOS | Cocoa | `libquevu_ui.dylib` | Not written yet |

```c
#include "quevu_ui.h"

uint32_t web = quevu_ui_unit_add("web");
quevu_ui_unit_state(web, QUEVU_UNIT_RUNNING, NULL);
uint32_t tab = quevu_ui_console_add("web");

/* another thread reads what the user does with quevu_ui_next_event */

quevu_ui_run("my application"); /* blocks until quevu_ui_quit */
```
