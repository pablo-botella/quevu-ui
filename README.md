# quevu-ui

The window of [quevu](https://github.com/pablo-botella/quevu): a row
of tabs, the first with the switches of an application (a global one and
one per unit) and the rest its screens (a tab per log, each a
pseudo-console), plus a menu and an icon in the
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

## The C interface

Described in full in `include/quevu_ui.h`. Its rules:

- `quevu_ui_run(title)` opens the window and blocks the calling thread
  until `quevu_ui_quit`. On macOS that must be the main thread of the
  process, so a portable host calls it from the main thread everywhere.
- Every other function may be called from any thread, before or while
  `quevu_ui_run` is running, and never waits for the window.
- **Nothing calls back into the host.** What the user does is read with
  `quevu_ui_next_event`, from a thread of the host's choosing. This keeps
  the interface trivial to bind from any language.
- Strings are NUL-terminated UTF-8, and are copied before the call
  returns. So is chunk data.
- `quevu_ui_abi()` returns the version of the interface. Check it first.

| Function | What it does |
|---|---|
| `quevu_ui_unit_add`, `quevu_ui_unit_state` | A switch, and the state it shows |
| `quevu_ui_console_add`, `quevu_ui_tab_remove` | A tab with a pseudo-console |
| `quevu_ui_console_deliver` | What came down the data lane of a console |
| `quevu_ui_console_note` | A note next to the state of a console (its queues) |
| `quevu_ui_console_find_text` | The text a console is looking for |
| `quevu_ui_console_goto` | Moves a console to the lines around something found |
| `quevu_ui_console_find_status` | How a step of a search went |
| `quevu_ui_menu_add` | An entry in the application menu |
| `quevu_ui_tray`, `quevu_ui_show` | The notification area icon; showing and hiding |
| `quevu_ui_next_event` | The next thing the user did |

Events: `READY`, `UNIT_START`, `UNIT_STOP`, `START_ALL`, `STOP_ALL`,
`CONSOLE_REQUEST`, `MENU`, `CLOSE`, `CONSOLE_FIND`.

The window decides nothing. A switch only reports that the user flipped
it; the host starts or stops the unit and reports the new state back.
Closing is the same: the window reports `CLOSE` and stays open until the
host, having stopped in order, calls `quevu_ui_quit`.

## The pseudo-console

A console is a view, not a store. It keeps a bounded window of lines
(3000) over a log that may be far longer, and asks for everything else by
position, 300 chunks at a time, with `CONSOLE_REQUEST` events.

- **Following:** pinned to the end, appending live output.
- **Paused:** the text holds still. Scrolling back, selecting text or the
  transport button hang the *do not disturb* sign; the host stops sending live
  output. If the console falls behind, the source hangs the sign itself.
- **Play** (the button, or End) does not catch up with what was missed:
  it asks for the last page and follows from there.
- Scrolling near either end of the window asks for the next page in that
  direction and drops lines at the far end. An empty answer looking back
  means there is nothing older.
- Where positions jump, a line says how many chunks were not kept.

Keys: arrows and Page Up/Down scroll, Home goes to the top of the window,
End resumes, Ctrl+A selects all, Ctrl+C copies, Ctrl+F goes to the search
box, F3 and Shift+F3 look for the next older and newer match. The search
keys work wherever the keyboard is, and the two arrows next to the search
box do the same for the mouse: up is older, down is newer. Shift+wheel
scrolls sideways.

**Searching.** The window never searches. It asks the host for one step
(`CONSOLE_FIND`: a text, a position to start from, a direction) and shows
what comes back. A step examines a bounded stretch of the log, so a search
never runs to the end of a log of millions of lines by itself:

- found: the view is held and the line told apart. If the console already
  holds that line nothing is reloaded, and the view moves only if the
  line is not comfortably in sight; otherwise the console moves to the
  lines around it;
- not found within the stretch: the strip says how far the step got, and
  asking for the same text again goes on from there;
- not found and nothing further: the strip says so.

The next step starts from the line last found, or, if there is none, from
the edge of what is on screen.

**History.** The console escalates in three steps: its own memory, recent
output, and the history. The first two are automatic. When recent output
runs out and the source says a history exists, a *History* button appears
and the strip says so; nothing is asked of the history until the user
presses it. The same goes for a search: a step that finds nothing in
recent output offers the history and stops. Going back to the live output
leaves the history.

**Time.** *Console > Show the time of each line* adds a margin with the
local time each chunk entered its log. The margin is not part of the text:
it is never selected or copied.

A chunk is always exactly one line: tabs are expanded and control
characters become a middle dot.

**Colors.** ANSI color sequences (ECMA-48 SGR, what programs written for
a terminal emit) are taken out of the text and drawn as color: the eight
basic colors and their bright versions, bold, the 256-color table and
24-bit colors, foreground only. Other escape sequences are dropped. A
selection or a copy never contains an escape code. Color does not carry
over from one chunk to the next.

**Layout.** One row of tabs across the window. The first tab, *Units*,
holds the switches: the global one and a row per unit with its state (in
color: running, in transit, stopped, failed) and, if it failed, the whole
error. Every other tab is a console; a dot on its title means it holds
output nobody has looked at.

The transport button is a small pause/play mark at the right end of the
tab row, as on a tape deck, and shows what pressing it does. It sits on
the console and away from the switches on purpose: it holds the view, not
the unit. The application and its log go on while a view is held.

## Dialogs and web views

quevu has no dialogs. An application that needs one serves a page and
opens it in the browser from an entry added with `quevu_ui_menu_add`.

A tab is a container, and a pseudo-console is one kind of content. A web
view is meant to be another, optional one: quevu must allow a web view to
live in a tab and must never depend on one. That kind of tab is not
implemented yet.

## Writing a backend

A backend is one function, `run(title, shared) -> i32`, that owns the
event loop of its toolkit. Everything it needs is in `quevu-ui-core`:

1. Build the window, then call `shared.attach(waker)`. The waker must make
   the UI thread call `shared.drain()` soon and be callable from any
   thread (a posted message, an idle callback, a dispatch to the main
   queue).
2. Emit `READY`, drain once, and run the loop.
3. Apply each `Command` drained: add a switch, change its state, add a
   tab, deliver to a console, and so on.
4. Keep one `console::Console` per tab. Draw `lines()` from `top()`, call
   `scroll_by`, `scroll_to`, `pause`, `follow` and `set_rows` as the user
   acts, and emit the requests they return as `Event::request`.
5. Emit an `Event` for every switch, menu entry and close request.
6. On the way out, call `shared.detach()`.

The logic of following, pausing and moving by position is in the model and
is already tested; a backend only draws and reports.

## Rules and gotchas

- **One window per process.** The library keeps one shared state.
- **Scoped to its thread.** The Win32 backend sets DPI awareness for its
  thread only and takes the modern look of the controls from an activation
  context that is active only while the window runs. A library must not
  decide those things for its host.
- **`unsafe` is confined** to the Win32 backend and the C door. The core
  has none.
- **The console view is a layer below the window** and knows nothing of
  it: what it cannot do itself (a search) it reports upwards as a wish,
  and the window decides what that means.
- **Window state hangs from the window** (`GWLP_USERDATA`), the same job a
  `void *context` does in C. A reference to it must not be kept across a
  call that can run another window procedure.
- **Columns are UTF-16 units** in the Win32 console, so characters outside
  the basic plane and double-width characters do not line up exactly.

## License

MIT - see [LICENSE](LICENSE).
