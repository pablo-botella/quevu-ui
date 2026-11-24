package qui

import (
	"fmt"
	"runtime"
	"sync"
	"time"

	"github.com/pablo-botella/quevu"
)

// Window is the quevu window. It starts with nothing on it: the
// application opens its tabs and gives it its menu, before Run or at any
// moment while it is open.
//
// A process has one window. Every method may be called from any
// goroutine; only Run is tied to the main one.
type Window struct {
	title string

	mu       sync.Mutex
	texts    map[Text]string
	sources  map[uint32]*SourceTab
	tabs     map[uint32]*ControlTab
	controls map[uint32]*Control
	items    map[uint32]func()
	verbIDs  map[string]uint32
	nextVerb uint32
	onReady  func()
	onClose  func()
	stats    time.Duration
	format   func([]quevu.WriterStats) string
	budget   int

	done chan struct{}
	wg   sync.WaitGroup
}

// NewWindow returns the window of this process, with the title given and
// nothing on it. It is not on the screen until Run.
func NewWindow(title string) *Window {
	w := &Window{
		title:    title,
		texts:    make(map[Text]string, len(defaultTexts)),
		sources:  make(map[uint32]*SourceTab),
		tabs:     make(map[uint32]*ControlTab),
		controls: make(map[uint32]*Control),
		items:    make(map[uint32]func()),
		verbIDs:  make(map[string]uint32),
		stats:    time.Second,
		format:   formatStats,
		done:     make(chan struct{}),
	}
	// The window has no word of its own: it is handed the ones of this
	// package, which the application may change.
	for id, text := range defaultTexts {
		w.texts[id] = text
		if id < firstOwnText {
			uiText(uint32(id), text)
		}
	}
	return w
}

// OnReady sets what happens once the window is on the screen. fn runs
// once, on a goroutine of its own, while Run goes on. If the window cannot
// be created fn never runs, and Run returns the error.
//
// It is the place to start what should only run with its window in
// sight, and to put away whatever stood in for the window until then.
func (w *Window) OnReady(fn func()) {
	w.mu.Lock()
	w.onReady = fn
	w.mu.Unlock()
}

// OnClose sets what happens when the user asks to close the window. The
// request is an event like any other: the window stays open, and goes
// away only when the application calls Quit. fn runs on a goroutine of
// its own every time the user asks, so one that takes its time should
// expect to be asked again.
//
// With no function set, asking to close the window closes it.
func (w *Window) OnClose(fn func()) {
	w.mu.Lock()
	w.onClose = fn
	w.mu.Unlock()
}

// SetStats sets how often the queues of each log are read and shown in
// the status strip of its tab, and how they are written. Zero turns it
// off; a nil format keeps the one in use. Default one second, in English.
func (w *Window) SetStats(every time.Duration, format func([]quevu.WriterStats) string) {
	w.mu.Lock()
	w.stats = every
	if format != nil {
		w.format = format
	}
	w.mu.Unlock()
}

// SetFindBudget sets how many chunks one step of a search may examine.
// A step that finds nothing within it says how far it got and stops;
// going further is the user's decision. The stored-data provider of a
// log may allow less. Default quevu.DefaultSearchBudget.
func (w *Window) SetFindBudget(chunks int) {
	w.mu.Lock()
	w.budget = chunks
	w.mu.Unlock()
}

// Tray puts an icon in the notification area, or takes it away. While it
// is there, minimizing the window hides it; its menu is TrayMenu. Where
// the system has no such area the window simply minimizes.
func (w *Window) Tray(on bool) { uiTray(flag(on)) }

// Show puts the window on the screen, or hides it without closing it.
func (w *Window) Show(visible bool) { uiShow(flag(visible)) }

// Quit closes the window: Run returns. It is the only way the window goes
// away, orderly or not: what happens before calling it is the
// application's business. It does nothing if no window is open.
func (w *Window) Quit() { uiQuit() }

func flag(on bool) uint32 {
	if on {
		return 1
	}
	return 0
}

// Run puts the window on the screen and returns when it is gone. It must
// be called from the main goroutine, and at most once per process.
func (w *Window) Run() error {
	w.mu.Lock()
	every := w.stats
	w.mu.Unlock()
	if every > 0 {
		w.wg.Add(1)
		go func() {
			defer w.wg.Done()
			w.showStats(every)
		}()
	}
	// The user's lane: what happens in the window is read here, never
	// called back from the window's own thread.
	w.wg.Add(1)
	go func() {
		defer w.wg.Done()
		w.events()
	}()

	code := uiRun(w.title)

	close(w.done)
	w.mu.Lock()
	open := make([]*SourceTab, 0, len(w.sources))
	for _, t := range w.sources {
		open = append(open, t)
	}
	w.mu.Unlock()
	for _, t := range open {
		t.stop()
	}
	w.wg.Wait()
	switch code {
	case 0:
		return nil
	case -1:
		return fmt.Errorf("%w: it was built for %s, and the window needs Windows and cgo", ErrNoWindow, runtime.GOOS)
	}
	return fmt.Errorf("qui: the window could not be created (code %d)", code)
}

// events reads what the user does until the window is gone.
func (w *Window) events() {
	var ready sync.Once
	started := false
	for {
		ev, code := uiNextEvent(500)
		switch code {
		case -1:
			if started {
				return
			}
			// The window code still remembers the end of a window that was
			// here before: this one has not begun.
			select {
			case <-w.done:
				return
			case <-time.After(10 * time.Millisecond):
			}
			continue
		case 0:
			continue
		}
		started = true
		switch ev.kind {
		case evReady:
			w.mu.Lock()
			fn := w.onReady
			w.mu.Unlock()
			if fn != nil {
				ready.Do(func() { go fn() })
			}
		case evConsoleRequest:
			if t := w.source(ev.id); t != nil {
				r := quevu.Request{Op: quevu.Op(ev.op), Seq: ev.seq, N: int(ev.n)}
				select {
				case t.reqs <- r:
				default:
					// A console that asks faster than it is answered
					// will ask again when the user moves.
				}
			}
		case evConsoleFind:
			if t := w.source(ev.id); t != nil {
				f := find{text: findText(ev.id), from: ev.seq, newer: ev.op&1 != 0, history: ev.op&2 != 0}
				f.since, f.until = findRange(ev.id)
				// Only the last thing asked for matters: a request still
				// waiting its turn is replaced, not queued behind.
				select {
				case <-t.finds:
				default:
				}
				select {
				case t.finds <- f:
				default:
				}
			}
		case evMenu:
			w.mu.Lock()
			fn := w.items[ev.id]
			w.mu.Unlock()
			if fn != nil {
				go fn()
			}
		case evControlVerb:
			w.mu.Lock()
			var fn func()
			if c := w.controls[ev.id]; c != nil {
				fn = c.verbs[ev.op]
			}
			w.mu.Unlock()
			if fn != nil {
				go fn()
			}
		case evTabVerb:
			w.mu.Lock()
			var fn func()
			if t := w.tabs[ev.id]; t != nil {
				fn = t.verbs[ev.op]
			}
			w.mu.Unlock()
			if fn != nil {
				go fn()
			}
		case evClose:
			w.mu.Lock()
			fn := w.onClose
			w.mu.Unlock()
			if fn == nil {
				fn = w.Quit
			}
			go fn()
		}
	}
}

func (w *Window) source(tab uint32) *SourceTab {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.sources[tab]
}

// TabOption says something of a tab when it is opened.
type TabOption func(*tabConfig)

type tabConfig struct {
	name   string
	weight int
}

// Weight says where a tab goes in the row: the heavier, the nearer the
// start. Of two tabs that weigh the same, the one opened first goes
// first. A tab that says nothing weighs zero, so a negative weight sends
// a tab after all of those.
func Weight(weight int) TabOption {
	return func(c *tabConfig) { c.weight = weight }
}

// Named gives a source tab a name other than the name of its source.
func Named(name string) TabOption {
	return func(c *tabConfig) { c.name = name }
}

// Color is the color of a light: red, green and blue, as 0xRRGGBB.
type Color uint32

// RGB makes a color out of its three parts.
func RGB(r, g, b uint8) Color {
	return Color(r)<<16 | Color(g)<<8 | Color(b)
}

// A few colors that read well as lights on the window. They are a
// convenience: any color will do.
const (
	Green Color = 0x107c10
	Amber Color = 0xc27c00
	Red   Color = 0xc42b1c
	Blue  Color = 0x0067c0
	Gray  Color = 0x707070
)

// Verb is something a control, or a control tab, lets the user ask for:
// a button with the application's words on it.
type Verb struct {
	Label string
	// Do runs, on a goroutine of its own, when the user presses the
	// button. Nothing changes on the window by itself: the application
	// says what the control shows from then on.
	Do func()
	// Disabled shows the button without letting it be pressed.
	Disabled bool
}

// State is everything a control shows.
type State struct {
	// Color is the color of its light.
	Color Color
	// Blink makes the light go on and off: something is under way.
	Blink bool
	// Hollow draws the light as a ring: it is off.
	Hollow bool
	// Text says what state the control is in, in the application's words.
	Text string
	// Verbs are the buttons the control offers in this state, in order.
	// A state with none is one the user can only look at.
	Verbs []Verb
}

// ControlTab is a tab with a table of controls.
type ControlTab struct {
	w  *Window
	id uint32
	// verbs, by the number the window sends back. Guarded by w.mu.
	verbs map[uint32]func()
}

// OpenControls opens a tab with a table of controls and nothing in it.
// The application adds the rows; there may be any number of such tabs.
func (w *Window) OpenControls(name string, opts ...TabOption) *ControlTab {
	cfg := tabConfig{name: name}
	for _, o := range opts {
		o(&cfg)
	}
	t := &ControlTab{w: w, id: uiControlsAdd(cfg.name, int32(cfg.weight))}
	w.mu.Lock()
	w.tabs[t.id] = t
	w.mu.Unlock()
	return t
}

// Add adds a row, at the end of the table. It shows its name and an
// unlit light until Set says what state it is in.
func (t *ControlTab) Add(name string) *Control {
	c := &Control{tab: t, id: uiControlAdd(t.id, name)}
	t.w.mu.Lock()
	t.w.controls[c.id] = c
	t.w.mu.Unlock()
	return c
}

// SetVerbs sets the verbs of the tab as a whole: buttons in its bar that
// act on everything in the table and not on one row. With none, the bar
// has no buttons.
func (t *ControlTab) SetVerbs(verbs ...Verb) {
	cs, fns := t.w.verbsToC(verbs)
	t.w.mu.Lock()
	t.verbs = fns
	t.w.mu.Unlock()
	uiTabVerbs(t.id, cs)
}

// SetNote sets what the status strip of the tab says. Empty leaves it
// blank.
func (t *ControlTab) SetNote(text string) { uiTabNote(t.id, text) }

// Rename changes the name of the tab.
func (t *ControlTab) Rename(name string) { uiTabTitle(t.id, name) }

// Select brings the tab to the front.
func (t *ControlTab) Select() { uiTabSelect(t.id) }

// Close closes the tab, with every control on it.
func (t *ControlTab) Close() {
	t.w.mu.Lock()
	delete(t.w.tabs, t.id)
	for id, c := range t.w.controls {
		if c.tab == t {
			delete(t.w.controls, id)
		}
	}
	t.w.mu.Unlock()
	uiTabRemove(t.id)
}

// Control is a row of a control tab.
type Control struct {
	tab *ControlTab
	id  uint32
	// verbs, by the number the window sends back. Guarded by the mutex
	// of the window.
	verbs map[uint32]func()
}

// Set says everything the control shows from now on: its light, the text
// of its state and its verbs. The window draws it and understands none
// of it.
//
// A control whose verbs do not change keeps its buttons as they are, so
// a text that changes often does not disturb them.
func (c *Control) Set(s State) {
	w := c.tab.w
	cs, fns := w.verbsToC(s.Verbs)
	w.mu.Lock()
	// Only the verbs of this state answer from now on: a button of the
	// state before, pressed just as it changed, asks for nothing.
	c.verbs = fns
	w.mu.Unlock()
	attrs := uint32(0)
	if s.Blink {
		attrs |= lightBlink
	}
	if s.Hollow {
		attrs |= lightHollow
	}
	uiControlState(c.id, uint32(s.Color)&0xffffff, attrs, s.Text, cs)
}

// Remove takes the row out of its table.
func (c *Control) Remove() {
	w := c.tab.w
	w.mu.Lock()
	delete(w.controls, c.id)
	w.mu.Unlock()
	uiControlRemove(c.id)
}

// verbsToC lays verbs out the way the window reads them and gives each
// the number the window will send back. The labels are not copied by
// A label gets the same number every time, and no other label gets it.
// That is what lets the window see that the buttons of a control did not
// change, and what keeps a button of one state from being taken for a
// different one of the next.
func (w *Window) verbsToC(verbs []Verb) (cs []uiVerb, fns map[uint32]func()) {
	fns = make(map[uint32]func(), len(verbs))
	cs = make([]uiVerb, len(verbs))
	for i, v := range verbs {
		w.mu.Lock()
		id, ok := w.verbIDs[v.Label]
		if !ok {
			w.nextVerb++
			id = w.nextVerb
			w.verbIDs[v.Label] = id
		}
		w.mu.Unlock()
		cs[i] = uiVerb{id: id, label: v.Label}
		if v.Disabled {
			cs[i].attrs = verbDisabled
		}
		if v.Do != nil {
			fns[id] = v.Do
		}
	}
	return cs, fns
}

// Menu is a menu of the window: the bar across its top, the menu of the
// icon in the notification area, or one that an entry of another opens.
// The window has no menu of its own: every entry is the application's.
type Menu struct {
	w  *Window
	id uint32
}

// MenuBar returns the bar across the top of the window. It is not shown
// while it has no entries.
func (w *Window) MenuBar() *Menu { return &Menu{w: w, id: menuBar} }

// Menu adds a menu to the bar of the window and returns it, empty.
func (w *Window) Menu(label string) *Menu { return w.MenuBar().Menu(label) }

// TrayMenu returns the menu of the icon in the notification area. It is
// only reachable while Tray is on.
func (w *Window) TrayMenu() *Menu { return &Menu{w: w, id: menuTray} }

// Menu adds an entry that opens a menu of its own, and returns that menu.
func (m *Menu) Menu(label string) *Menu {
	return &Menu{w: m.w, id: uiMenuAdd(m.id, label, menuSubmenu)}
}

// Add adds an entry. do runs, on a goroutine of its own, when the user
// chooses it.
func (m *Menu) Add(label string, do func()) *MenuItem {
	it := &MenuItem{w: m.w, label: label}
	it.id = uiMenuAdd(m.id, label, 0)
	if do != nil {
		m.w.mu.Lock()
		m.w.items[it.id] = do
		m.w.mu.Unlock()
	}
	return it
}

// Separator adds a line between entries.
func (m *Menu) Separator() *MenuItem {
	it := &MenuItem{w: m.w, attrs: menuSeparator}
	it.id = uiMenuAdd(m.id, "", menuSeparator)
	return it
}

// Remove takes the menu away, with everything in it. The bar and the
// menu of the tray icon are not removed: they go when they are empty.
func (m *Menu) Remove() {
	if m.id != menuBar && m.id != menuTray {
		uiMenuRemove(m.id)
	}
}

// MenuItem is an entry of a menu.
type MenuItem struct {
	w  *Window
	id uint32

	mu    sync.Mutex
	label string
	attrs uint32
}

// SetLabel changes the words of the entry.
func (i *MenuItem) SetLabel(label string) {
	i.mu.Lock()
	i.label = label
	i.mu.Unlock()
	i.push()
}

// SetChecked puts on the entry the mark of something that is on, or
// takes it off. Nothing changes by itself when the user chooses an
// entry: the mark is the application's to move.
func (i *MenuItem) SetChecked(on bool) { i.attr(menuChecked, on) }

// SetDisabled shows the entry without letting it be chosen.
func (i *MenuItem) SetDisabled(on bool) { i.attr(menuDisabled, on) }

func (i *MenuItem) attr(bit uint32, on bool) {
	i.mu.Lock()
	if on {
		i.attrs |= bit
	} else {
		i.attrs &^= bit
	}
	i.mu.Unlock()
	i.push()
}

func (i *MenuItem) push() {
	i.mu.Lock()
	label, attrs := i.label, i.attrs
	i.mu.Unlock()
	uiMenuSet(i.id, label, attrs)
}

// Remove takes the entry out of its menu.
func (i *MenuItem) Remove() {
	i.w.mu.Lock()
	delete(i.w.items, i.id)
	i.w.mu.Unlock()
	uiMenuRemove(i.id)
}
