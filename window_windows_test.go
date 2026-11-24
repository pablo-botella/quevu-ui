//go:build cgo

package qui

import (
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/pablo-botella/quevu"
)

// fakeSource records the requests a console sends up its feed and answers
// the first follow.
type fakeSource struct {
	name string
	mu   sync.Mutex
	reqs []quevu.Request
	got  chan quevu.Request
}

func (s *fakeSource) Name() string {
	if s.name != "" {
		return s.name
	}
	return "out"
}

func (s *fakeSource) Tail(int) ([]quevu.Chunk, error)           { return nil, nil }
func (s *fakeSource) Before(uint64, int) ([]quevu.Chunk, error) { return nil, nil }
func (s *fakeSource) After(uint64, int) ([]quevu.Chunk, error)  { return nil, nil }
func (s *fakeSource) Stats() []quevu.WriterStats                { return nil }
func (s *fakeSource) Subscribe(int) *quevu.Feed {
	data := make(chan quevu.Delivery)
	req := make(chan quevu.Request, 16)
	done := make(chan struct{})
	go func() {
		defer close(data)
		for {
			select {
			case <-done:
				return
			case r := <-req:
				s.got <- r
				d := quevu.Delivery{Op: r.Op}
				if r.Op == quevu.OpFollow {
					d.Chunks = []quevu.Chunk{
						{Seq: 1, Time: time.Now(), Data: []byte("first line")},
						{Seq: 2, Time: time.Now(), Data: nil},
						{Seq: 3, Time: time.Now(), Data: []byte("tab\there \x1b[31mred\x1b[0m \xff")},
					}
				}
				select {
				case data <- d:
				case <-done:
					return
				}
			}
		}
	}()
	return quevu.NewFeed(data, req, func() { close(done) })
}

// library finds a built quevu-ui library, or skips the test.

// expect waits for one thing to be said on a channel of the test.
func expect(t *testing.T, got <-chan string, want string) {
	t.Helper()
	select {
	case s := <-got:
		if s != want {
			t.Fatalf("got %q, want %q", s, want)
		}
	case <-time.After(10 * time.Second):
		t.Fatalf("%q never happened", want)
	}
}

// static waits for a label of the window to read as given.
func static(t *testing.T, top uintptr, text string) {
	t.Helper()
	for i := 0; i < 500; i++ {
		for _, h := range descendants(top) {
			if className(h) == "Static" && visible(h) && windowText(h) == text {
				return
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("nothing on the window reads %q", text)
}

// The window built into this program, as a real window, driven from outside the way a user
// would: the window shows what the application says, in the order the
// application says, and tells it what was pressed. It decides nothing.
func TestTheWindowIsWhatTheApplicationSays(t *testing.T) {
	const title = "qui window test"
	w := NewWindow(title)
	w.SetStats(0, nil)
	w.SetText(TextFollowing, "Siguiendo la salida")

	said := make(chan string, 16)
	say := func(s string) func() { return func() { said <- s } }

	// Opened before the window exists, and not in the order of the row.
	src := &fakeSource{name: "log", got: make(chan quevu.Request, 16)}
	logTab := w.OpenSource(src, Weight(10))
	logTab.ShowTime(true)
	services := w.OpenControls("Services", Weight(100))
	web := services.Add("Web server")
	web.Set(State{Color: Green, Text: "listening on 127.0.0.1:8443"})
	mail := services.Add("Mail")
	mail.Set(State{Color: Gray, Hollow: true, Text: "stopped", Verbs: []Verb{{Label: "Start", Do: say("mail start")}}})
	watcher := services.Add("Folder watcher")
	watcher.Set(State{Color: Red, Text: "cannot watch the folder: access denied", Verbs: []Verb{{Label: "Start", Do: say("watcher start"), Disabled: true}}})
	services.SetVerbs(Verb{Label: "Stop all", Do: say("stop all")})
	services.SetNote("3 services")

	quit := w.Menu("Quit")
	normal := quit.Add("Normal", say("normal"))
	quit.Separator()
	forced := quit.Add("Forced", say("forced"))

	w.OnClose(say("close"))
	w.OnReady(say("ready"))

	ran := make(chan error, 1)
	go func() {
		// A window lives and dies on one thread.
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		ran <- w.Run()
	}()
	expect(t, said, "ready")
	top := topWindow(t, title)

	// The heavier tab goes first, whatever the order they were opened in.
	waitTabs(t, top, "Services", "log")
	if _, front := tabTitles(top); front != 1 {
		t.Fatalf("the tab in front is %d: the first one opened should be", front)
	}
	services.Select()
	static(t, top, "3 services")
	shot(top, "1-services")

	// A verb pressed is only told. One that is disabled cannot be.
	start := waitButton(t, top, "Start", true)
	if !enabled(start) {
		t.Fatal("the verb of Mail is disabled")
	}
	click(start)
	expect(t, said, "mail start")

	// The state is whatever the application says next, verbs included.
	mail.Set(State{Color: Amber, Blink: true, Text: "sending 3 of 40", Verbs: []Verb{{Label: "Stop", Do: say("mail stop")}, {Label: "Kick", Do: say("mail kick")}}})
	click(waitButton(t, top, "Kick", true))
	expect(t, said, "mail kick")
	shot(top, "2-services-sending")
	click(waitButton(t, top, "Stop all", true))
	expect(t, said, "stop all")

	// The menu is the application's, and choosing changes nothing by
	// itself.
	choose(top, waitMenu(t, top, "Quit", "Normal"))
	expect(t, said, "normal")
	if l := waitMenu(t, top, "Quit", "Normal"); l.checked {
		t.Fatal("choosing an entry marked it")
	}
	normal.SetChecked(true)
	forced.SetDisabled(true)
	for i := 0; ; i++ {
		if waitMenu(t, top, "Quit", "Normal").checked && waitMenu(t, top, "Quit", "Forced").grayed {
			break
		}
		if i > 500 {
			t.Fatal("the entries did not take their mark and their gray")
		}
		time.Sleep(10 * time.Millisecond)
	}
	forced.Remove()
	for i := 0; len(menuLines(waitMenu(t, top, "Quit").sub)) != 2; i++ {
		if i > 500 {
			t.Fatalf("the menu has %d entries after removing one of three", len(menuLines(waitMenu(t, top, "Quit").sub)))
		}
		time.Sleep(10 * time.Millisecond)
	}

	// Tabs open while the window is there, each where its weight says.
	jobs := w.OpenControls("Jobs", Weight(90))
	waitTabs(t, top, "Services", "Jobs", "log")
	debug := w.OpenSource(&fakeSource{name: "raw", got: make(chan quevu.Request, 16)}, Named("debug"), Weight(-1))
	plain := w.OpenControls("Plain")
	waitTabs(t, top, "Services", "Jobs", "log", "Plain", "debug")
	job := jobs.Add("Report 12")
	job.Set(State{Color: Blue, Blink: true, Text: "340 of 2000 orders"})
	jobs.Add("Report 13").Set(State{Color: Gray, Hollow: true, Text: "queued"})
	jobs.Select()
	shot(top, "3-jobs")
	job.Remove()
	plain.Rename("Renamed")
	waitTabs(t, top, "Services", "Jobs", "log", "Renamed", "debug")

	// And close, taking what is on them along.
	jobs.Close()
	plain.Close()
	debug.Close()
	waitTabs(t, top, "Services", "log")

	// The console of a source tab asked for its first page by itself and
	// says, in the words it was given, that it follows.
	select {
	case r := <-src.got:
		if r.Op != quevu.OpFollow || r.N <= 0 {
			t.Fatalf("the console opened with %+v", r)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the console never asked for its first page")
	}
	logTab.Select()
	static(t, top, "Siguiendo la salida")
	shot(top, "4-log")

	// Asking to close is an event like any other: the window stays.
	askToClose(top)
	expect(t, said, "close")
	time.Sleep(100 * time.Millisecond)
	if h, _, _ := procFindWindow.Call(0, uintptr(unsafePointer(title))); h == 0 {
		t.Fatal("the window closed by itself")
	}

	// With nothing on it the window is still a window.
	services.Close()
	logTab.Close()
	waitTabs(t, top)
	shot(top, "5-empty")

	// It goes when the application says so.
	w.Quit()
	select {
	case err := <-ran:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the window did not close")
	}
}
