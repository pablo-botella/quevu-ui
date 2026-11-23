---
mkskill:
  pos: 10
  in: readme
---

# quevu-ui

The window of [quevu](https://github.com/pablo-botella/quevu): a row
of tabs that the application fills, a menu that is the application's, and
an icon in the notification area to hide in. Nothing more: quevu wants a
window for applications, not a GUI toolkit.

A tab is one of two kinds:

- a **source tab** shows a log in a pseudo-console, with its search;
- a **control tab** shows a table of controls: each row has a light, a
  name, the text of its state and the buttons of its verbs.

The window has no language and decides nothing. No tab is fixed: the
application opens and closes them, names them and says where each goes.
States, verbs, colors, menu entries and every word on the screen are the
application's; what the user does goes back as an event; and the window
goes away only when the application says so.

It is written in Rust against the native toolkit of each system and ships
as a dynamic library with a C interface, so any language can use it:

| System | Toolkit | Library | State |
|---|---|---|---|
| Windows | Win32 | `quevu_ui.dll` | Working |
| Linux | GTK | `libquevu_ui.so` | Not written yet |
| macOS | Cocoa | `libquevu_ui.dylib` | Not written yet |

```c
#include "quevu_ui.h"

uint32_t services = quevu_ui_controls_add("Services", 100);
uint32_t web = quevu_ui_control_add(services, "Web server");
quevu_ui_verb stop = { 1, 0, "Stop" };
quevu_ui_control_state(web, 0x107c10, 0, "listening on :8443", &stop, 1);

uint32_t log = quevu_ui_console_add("web", 0);

uint32_t quit = quevu_ui_menu_add(QUEVU_MENU_BAR, "Quit", QUEVU_MENU_SUBMENU);
uint32_t now = quevu_ui_menu_add(quit, "Now", 0);

/* another thread reads what the user does with quevu_ui_next_event:
   QUEVU_EV_CONTROL_VERB with id == web and op == 1 is "Stop" pressed */

quevu_ui_run("my application"); /* blocks until quevu_ui_quit */
```
