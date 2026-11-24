package qui

import (
	"context"
	"sync"
	"time"

	"github.com/pablo-botella/quevu"
)

// Option configures the window Run opens over a panel.
type Option func(*config)

type config struct {
	tray    bool
	onClose func()
	onReady func()
	menu    []menuEntry
	stats   time.Duration
	budget  int
}

type menuEntry struct {
	label string
	fn    func()
}

// WithTray puts an icon in the notification area. While it is there,
// closing or minimizing the window hides it instead; the way out is the
// Exit entry of the menu. Where the system has no such area the window
// simply minimizes.
func WithTray() Option {
	return func(c *config) { c.tray = true }
}

// OnReady sets what happens once the window is on the screen. fn runs
// once, on a goroutine of its own, while Run goes on. If the window cannot
// be created fn never runs, and Run returns the error.
func OnReady(fn func()) Option {
	return func(c *config) { c.onReady = fn }
}

// OnClose sets what happens when the user asks to close the application.
// The window stays open, showing the units change state, until fn
// returns; then it goes away and Run returns.
//
// The default stops every unit in order, which is what the window of an
// application should do. A controller attached to somebody else's
// service wants an fn that does nothing: closing the window must not
// stop the service.
func OnClose(fn func()) Option {
	return func(c *config) { c.onClose = fn }
}

// WithMenu adds an entry to the Application menu. fn runs on a goroutine
// of its own. quevu has no dialogs: an application that needs one serves
// a page and opens it from an entry like this.
func WithMenu(label string, fn func()) Option {
	return func(c *config) { c.menu = append(c.menu, menuEntry{label, fn}) }
}

// WithStats sets how often the queues of each log are read and shown in
// the status strip of its tab. Zero turns it off. Default one second.
func WithStats(every time.Duration) Option {
	return func(c *config) { c.stats = every }
}

// WithFindBudget sets how many chunks one step of a search may examine.
// Default quevu.DefaultSearchBudget.
func WithFindBudget(chunks int) Option {
	return func(c *config) { c.budget = chunks }
}

// Run opens a window over a panel and returns when it is gone. It must be
// called from the main goroutine, and at most once per process.
//
// It is the window of an application that is happy with the usual one,
// built with what Window offers: a tab called Units with a row for each
// unit of the panel and a button to start or stop it, a tab for each of
// its sources, and a menu with an entry to exit. Its words are English.
// An application that wants other tabs, other states, other verbs or
// other words builds its own with Window.
//
// The units and sources are read once, when the window opens.
func Run(title string, c quevu.Controller, opts ...Option) error {
	cfg := config{stats: time.Second}
	for _, o := range opts {
		o(&cfg)
	}
	if cfg.onClose == nil {
		cfg.onClose = func() { c.StopAll(context.Background()) }
	}

	w := NewWindow(title)
	w.SetStats(cfg.stats, nil)
	w.SetFindBudget(cfg.budget)
	if cfg.onReady != nil {
		w.OnReady(cfg.onReady)
	}

	// Leaving is asked for once: the window stays, showing the units
	// change state, until the close function returns.
	var leaving sync.Once
	exit := func() {
		leaving.Do(func() {
			cfg.onClose()
			w.Quit()
		})
	}
	w.OnClose(func() {
		if cfg.tray {
			// With a tray icon the cross hides; Exit is in the menu.
			w.Show(false)
			return
		}
		exit()
	})

	if units := c.Units(); len(units) > 0 {
		tab := w.OpenControls("Units", Weight(100))
		tab.SetVerbs(
			Verb{Label: "Start all", Do: func() { c.StartAll(context.Background()) }},
			Verb{Label: "Stop all", Do: func() { c.StopAll(context.Background()) }},
		)
		rows := make(map[string]*Control, len(units))
		for _, u := range units {
			rows[u.Name] = tab.Add(u.Name)
			rows[u.Name].Set(unitState(c, u))
		}
		unwatch := c.Watch(func(u quevu.UnitInfo) {
			if row, ok := rows[u.Name]; ok {
				row.Set(unitState(c, u))
			}
		})
		defer unwatch()
	}
	for _, src := range c.Sources() {
		w.OpenSource(src)
	}

	w.Menu("&Panel").Add("E&xit", exit)
	if len(cfg.menu) > 0 {
		app := w.Menu("&Application")
		for _, m := range cfg.menu {
			app.Add(m.label, m.fn)
		}
	}
	if cfg.tray {
		w.Tray(true)
		tray := w.TrayMenu()
		tray.Add("Open", func() { w.Show(true) })
		tray.Separator()
		tray.Add("Exit", exit)
	}
	return w.Run()
}

// unitState is how a unit of a panel looks as a control: a light, its
// state in a word and the one thing that can be asked of it. An error
// shows up as the state of the unit.
func unitState(c quevu.Controller, u quevu.UnitInfo) State {
	start := Verb{Label: "Start", Do: func() { c.Start(context.Background(), u.Name) }}
	stop := Verb{Label: "Stop", Do: func() { c.Stop(context.Background(), u.Name) }}
	switch u.State {
	case quevu.Starting:
		return State{Color: Amber, Blink: true, Text: "starting"}
	case quevu.Running:
		return State{Color: Green, Text: "running", Verbs: []Verb{stop}}
	case quevu.Stopping:
		return State{Color: Amber, Blink: true, Text: "stopping"}
	case quevu.Failed:
		text := "failed"
		if u.Err != "" {
			text += ": " + u.Err
		}
		return State{Color: Red, Text: text, Verbs: []Verb{start}}
	}
	return State{Color: Gray, Hollow: true, Text: "stopped", Verbs: []Verb{start}}
}

// Quit closes the window from the host's side: Run returns. It may be
// called from any goroutine, and does nothing if no window is open. It is
// for the host that has to leave for reasons of its own, such as a
// controller whose application went away.
func Quit() { uiQuit() }
