# quevu-ui

The quevu window, as a Go package: `qui`. It is compiled into the program
that uses it, so an application ships as **one executable, with nothing
next to it**.

A row of tabs the application fills, and a menu that is the
application's. A tab is one of two kinds: a source tab shows a log of
[quevu](https://github.com/pablo-botella/quevu) in a pseudo-console; a
control tab shows a table of controls, each with a light, a name, the
text of its state and the buttons of its verbs. The window has no
language and decides nothing: states, verbs, colors, menu entries and
every word come from the application, and what the user does runs a
function of the application's.

```go
import qui "github.com/pablo-botella/quevu-ui"

func main() {
	win := qui.NewWindow("My application")

	services := win.OpenControls("Services", qui.Weight(100))
	web := services.Add("Web server")
	web.Set(qui.State{Color: qui.Green, Text: "listening on :8443"})
	mail := services.Add("Mail")
	mail.Set(qui.State{Color: qui.Gray, Hollow: true, Text: "stopped",
		Verbs: []qui.Verb{{Label: "Start", Do: startMail}}})

	win.OpenSource(logSource, qui.Weight(10)) // a quevu.Source

	win.Menu("&Quit").Add("&Now", win.Quit)
	win.OnClose(win.Quit)

	if err := win.Run(); err != nil && !qui.IsNoWindow(err) {
		log.Fatal(err)
	}
}
```

## How it is made

The package is Go on top and C underneath, joined by cgo:

- **Go** (`window.go`, `source.go`, `run.go`, `texts.go`): what an
  application uses. It keeps the tabs, the controls and the menus, feeds
  each console from its quevu source, runs searches, and holds every text
  the window shows (English by default, all of them replaceable).
- **C** (`qv_*.c`): the window itself, against the Win32 API. It draws
  what it is told and reports what the user does, through the interface
  in `quevu_ui.h`: commands go down, events come up, nothing calls back.

No library is loaded and no third-party package is involved; the only
dependency is `quevu` itself.

## Building

- Go 1.26 or later, with `CGO_ENABLED=1` (the default when a C compiler
  is found).
- gcc from MinGW-w64, 64-bit. With MSYS2 that is the `ucrt64` or
  `mingw64` toolchain.
- Nothing else: the window links against `user32`, `gdi32`, `comctl32`,
  `shell32` and the C runtime that is part of Windows. The executable
  needs no file of the compiler's where it runs.

```
go build -o demo.exe ./demo
```

The demo is a log that keeps talking and a table with two controls.

**If the build fails with nothing but `cgo.exe: exit status 2`**, gcc is
dying before it can say anything, which happens when the `bin` folder of
a different MinGW comes earlier in `PATH` than its own (a shell that
ships its own, for instance): gcc then starts with the wrong runtime
libraries. Put the folder of the gcc in use first in `PATH`, or build
from a shell that has only one.

## A program with no window

The window exists for Windows, and only in a build with cgo. Anywhere
else the package still builds, with no C at all: `qui.Available()` says
false, every call does nothing, and `Run` returns at once an error for
which `qui.IsNoWindow` is true. That is a normal situation, not a
failure: the same source serves a desktop and a server with no display.

## Tests

```
go test ./...
```

- The parts of the C that need no window (the text of a line, the order
  of the tabs, the model of a console, the two lanes between the
  application and the window) are checked in C, in
  `qv_selftest_windows.c`, and run from a Go test.
- The window is opened for real and driven from outside the way a user
  would: buttons pressed, menu entries chosen, tabs read. With
  `QUEVU_UI_SHOTS=<folder>` the test saves pictures of it.

## The files

| File | What it holds |
|---|---|
| `qui.go` | The package: its documentation, `Available`, `IsNoWindow`. |
| `window.go` | `Window`, control tabs, controls, verbs, menus. |
| `source.go` | Source tabs: feeding a console, searching. |
| `texts.go` | Every text the window shows, and `SetText`. |
| `run.go` | `Run`: the usual window of a quevu panel, ready made. |
| `door_windows.go` | The only Go that touches C: one function per function of `quevu_ui.h`. |
| `door_none.go` | The same functions for a program with no window. |
| `quevu_ui.h` | The interface between the Go and the C. |
| `qv.h`, `qv_win.h` | What the C files share among themselves. |
| `qv_core_windows.c` | Strings, the text of a line (ANSI colors, tabs), the order of the tabs, the model of a console. Knows no system. |
| `qv_shared_windows.c` | The two one-way lanes and the functions of `quevu_ui.h`. |
| `qv_win_windows.c` | The window: tabs, pages, bars, menus, the notification area. |
| `qv_view_windows.c` | The pseudo-console as a control. |
| `qv_table_windows.c` | The table of controls as a control. |

Rules that hold throughout: the window runs on the thread that calls
`Run`, which must be the main one; every other call may come from any
goroutine and never waits for the window.

## Not here yet

- Any system other than Windows. `qv_core` is written to be shared when
  there is a second one.
- 32-bit builds have not been tried.

## License

MIT. See `LICENSE`.
