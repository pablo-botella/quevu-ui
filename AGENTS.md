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
  returns. So is chunk data, and so are the labels of verbs.
- What is asked for before `quevu_ui_run` is on the window when it first
  shows: it does not open empty and fill up afterwards.
- `quevu_ui_abi()` returns the version of the interface. Check it first.
  This is version 5.

| Function | What it does |
|---|---|
| `quevu_ui_console_add` | Opens a source tab: a pseudo-console |
| `quevu_ui_controls_add` | Opens a control tab: a table of controls |
| `quevu_ui_tab_title`, `quevu_ui_tab_select`, `quevu_ui_tab_remove` | Renames a tab, brings it to the front, closes it |
| `quevu_ui_tab_note` | What the status strip of a tab says on the host's account |
| `quevu_ui_tab_verbs` | The verbs of a control tab as a whole: the buttons of its bar |
| `quevu_ui_control_add`, `quevu_ui_control_remove` | A row of a control tab |
| `quevu_ui_control_state` | Everything a control shows: light, text and verbs |
| `quevu_ui_console_deliver` | What came down the data lane of a console |
| `quevu_ui_console_time` | Whether a console shows the time of each line |
| `quevu_ui_console_find_text` | The text a console is looking for |
| `quevu_ui_console_find_range` | The range of time it is looking within |
| `quevu_ui_console_goto` | Moves a console to the lines around something found |
| `quevu_ui_console_find_status` | How a step of a search went |
| `quevu_ui_menu_add`, `quevu_ui_menu_set`, `quevu_ui_menu_remove` | The entries of the menus |
| `quevu_ui_text` | One of the texts the window shows on its own account |
| `quevu_ui_tray`, `quevu_ui_show` | The notification area icon; showing and hiding |
| `quevu_ui_next_event` | The next thing the user did |

Events: `READY`, `CONSOLE_REQUEST`, `MENU`, `CLOSE`, `CONSOLE_FIND`,
`CONTROL_VERB`, `TAB_VERB`.

## A window that decides nothing

The window has **no language, no commands of its own and no behaviour**
beyond drawing what it is told and reporting what the user does.

- **No tab is fixed.** A window with no tab at all is a valid window. The
  host opens and closes tabs at any moment.
- **Every word is the host's**: the names of tabs and controls, the text
  of a state, the labels of verbs and menu entries. The few texts a source
  tab needs for what it does by itself (holding the view, searching) are
  numbered, empty until the host sets them with `quevu_ui_text`, and
  listed in `quevu_ui.h`. A text nobody set leaves its place blank; a menu
  entry of a source tab with no text is left out.
- **Pressing changes nothing.** A verb pressed or a menu entry chosen is
  an event. The button does not change, the mark of the entry does not
  move: the host says what is shown next.
- **Closing is an event too.** The cross of the window sends `CLOSE` and
  the window stays. It goes away only on `quevu_ui_quit`, orderly or not:
  what the host does before calling it is the host's business. A quit
  asked for while no window is open does nothing.

## Tabs and their weight

A tab takes its place in the row by the weight the host gives it. The
heavier a tab, the nearer the start. Of two tabs that weigh the same, the
one opened first goes first. A tab that says nothing weighs zero, so a
negative weight sends a tab after all of those. A tab opened while the
window is on the screen goes where its weight says, between the others if
need be. The first tab opened is in front until the host selects another
or the user does.

Every tab is a whole page: its content and, under it, a bar that belongs
to that tab alone, with its tools and its status strip. Nothing on the
window follows the tab in front around.

## Control tabs

A control tab is a table: one row per control, in columns that line up.

| Column | What the host says |
|---|---|
| Light | A color, as `0x00RRGGBB`, and a word of attributes: `QUEVU_LIGHT_BLINK` makes it go on and off, `QUEVU_LIGHT_HOLLOW` draws it as a ring |
| Name | Given when the control is added |
| State | Any text |
| Verbs | Any number of buttons, each with a number and a label; `QUEVU_VERB_DISABLED` shows one that cannot be pressed |

`quevu_ui_control_state` sets light, text and verbs at once, every time
the control changes. A control with no verbs is one the user can only
look at. A list of verbs equal to the one the control already has leaves
its buttons untouched, so a text that changes often does not disturb them
or take the keyboard away from them. Pressing a button sends
`CONTROL_VERB` with the control in `id` and the number of the verb in `op`.

A control tab has verbs of its own too, set with `quevu_ui_tab_verbs`:
buttons in its bar that act on the whole table (`TAB_VERB`). The strip of
its bar says only what the host sets with `quevu_ui_tab_note`. A control
tab with no verbs and no note has no bar. Controls are added and removed
at any moment; a table longer than the window scrolls.

There may be any number of control tabs, none included.

## Menus

The window has no menu of its own. An entry hangs from the bar of the
window (`QUEVU_MENU_BAR`), from the menu of the notification icon
(`QUEVU_MENU_TRAY`) or from another entry that was added with
`QUEVU_MENU_SUBMENU`. A bar with no entries is not shown.

An entry can be a line between others (`QUEVU_MENU_SEPARATOR`), carry a
mark (`QUEVU_MENU_CHECKED`) or be shown without being choosable
(`QUEVU_MENU_DISABLED`). `quevu_ui_menu_set` changes its label and those
attributes later; `quevu_ui_menu_remove` takes it away with whatever
hangs from it. Choosing an entry sends `MENU` with its identifier.

## The pseudo-console

A console is a view, not a store. It keeps a bounded window of lines
(3000) over a log that may be far longer, and asks for everything else by
position, 300 chunks at a time, with `CONSOLE_REQUEST` events.

This is the one part of the window with mechanics of its own: scrolling,
following or holding the view, the steps of a search, selecting and
copying. It still decides nothing for the host, and says nothing in words
of its own.

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
keys work wherever the keyboard is, on the tab in front, and the two
arrows next to the search box do the same for the mouse: up is older,
down is newer. Shift+wheel scrolls sideways.

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

What the strip says of a step is the host's text, handed over with
`quevu_ui_console_find_status`. The next step starts from the line last
found, or, if there is none, from the edge of what is on screen.

The button with three dots next to the arrows folds out the rest of what
a search can be given. For now that is a range of time: a start and an
end, each with a box to tick; unticked, that side has no bound. The range
counts only while the row is open, so a bound nobody can see never
narrows a search. The host reads it with `quevu_ui_console_find_range`.

**History.** The console escalates in three steps: its own memory, recent
output, and the history. The first two are automatic. When recent output
runs out and the source says a history exists, the history button appears
and the strip says so; nothing is asked of the history until the user
presses it. The same goes for a search: a step that finds nothing in
recent output offers the history and stops. Going back to the live output
leaves the history.

**Time.** The entry of the menu of the tab numbered
`QUEVU_TEXT_MENU_TIME` adds a margin with the local time each chunk
entered its log. The margin is not part of the text: it is never selected
or copied. Each tab keeps its own choice, and the host sets how a tab
starts with `quevu_ui_console_time`.

A chunk is always exactly one line: tabs are expanded and control
characters become a middle dot.

**Colors.** ANSI color sequences (ECMA-48 SGR, what programs written for
a terminal emit) are taken out of the text and drawn as color: the eight
basic colors and their bright versions, bold, the 256-color table and
24-bit colors, foreground only. Other escape sequences are dropped. A
selection or a copy never contains an escape code. Color does not carry
over from one chunk to the next.

**The bar of a source tab.** Under the console, from left to right: the
status strip, the history button when there is a history to offer, the
search box with its two arrows and the button that folds out the rest of
a search, the transport button and the button that opens the menu of the
tab. All of it belongs to that tab: the text typed in the search box of
one tab, its range of time and whether it shows the time stay with it.

The transport button is a small pause/play mark, as on a tape deck, and
shows what pressing it does. It holds the view and nothing else: the
application and its log go on while a view is held. The menu of the tab
offers the same things the keys do. A dot on the title of a tab means it
holds output nobody has looked at.

## Dialogs and web views

quevu has no dialogs. An application that needs one serves a page and
opens it in the browser from a menu entry or from a verb.

A tab is a container, and a pseudo-console and a table of controls are
two kinds of content. A web view is meant to be another, optional one:
quevu must allow a web view to live in a tab and must never depend on
one. That kind of tab is not implemented yet.

## Writing a backend

A backend is one function, `run(title, shared) -> i32`, that owns the
event loop of its toolkit. Everything it needs is in `quevu-ui-core`:

1. Build the window with nothing on it, call `shared.begin()`, and apply
   whatever `shared.drain()` already holds before showing it. A `Quit`
   found there was asked for while no window was open: drop it.
2. Show the window and call `shared.attach(waker)`. The waker must make
   the UI thread call `shared.drain()` soon and be callable from any
   thread (a posted message, an idle callback, a dispatch to the main
   queue).
3. Emit `READY`, drain once more, and run the loop.
4. Apply each `Command` drained: open a tab where its weight says
   (`tabs::insert_at`), add a control, set its state, deliver to a
   console, change a menu, and so on.
5. Keep one `console::Console` per source tab. Draw `lines()` from
   `top()`, call `scroll_by`, `scroll_to`, `pause`, `follow` and
   `set_rows` as the user acts, and emit the requests they return as
   `Event::request`.
6. Emit an `Event` for every verb pressed, menu entry chosen and close
   request, and change nothing on their account.
7. Take every word from the host: the `Command`s carry them, and
   `texts::Texts` holds the few a source tab needs for itself.
8. On the way out, call `shared.detach()`.

The logic of following, pausing and moving by position is in the model and
is already tested; a backend only draws and reports.

## Rules and gotchas

- **One window per process.** The library keeps one shared state. A
  second window may be opened once the first is gone.
- **Scoped to its thread.** The Win32 backend sets DPI awareness for its
  thread only and takes the modern look of the controls from an activation
  context that is active only while the window runs. A library must not
  decide those things for its host.
- **`unsafe` is confined** to the Win32 backend and the C door. The core
  has none.
- **The console view and the table of controls are layers below the
  window** and know nothing of it: what they cannot do themselves (a
  search, a verb) they report upwards, and decide nothing.
- **Window state hangs from the window** (`GWLP_USERDATA`), the same job a
  `void *context` does in C. A reference to it must not be kept across a
  call that can run another window procedure.
- **Columns are UTF-16 units** in the Win32 console, so characters outside
  the basic plane and double-width characters do not line up exactly.
- **Colors are the host's as given.** The window does not adapt a color
  to a light or dark theme.
