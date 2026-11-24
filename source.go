package qui

import (
	"fmt"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/pablo-botella/quevu"
)

// SourceTab is a tab that shows a log: a feed of its source and the
// goroutines on its two lanes.
type SourceTab struct {
	w     *Window
	id    uint32
	src   quevu.Source
	feed  *quevu.Feed
	reqs  chan quevu.Request
	finds chan find
	done  chan struct{}
	once  sync.Once
}

// find is one step of a search asked for from a console.
type find struct {
	text  string
	from  uint64
	newer bool
	// history is set only when the user allowed the history in the
	// window: this package never sets it by itself.
	history bool
	// since and until are the time range the user set, if any.
	since, until time.Time
}

// OpenSource opens a tab that shows a source in a pseudo-console. The tab
// is called as the source is, unless Named says otherwise.
//
// Closing the tab only takes it off the window: whatever feeds the source
// goes on, and the same source can be opened again later.
func (w *Window) OpenSource(src quevu.Source, opts ...TabOption) *SourceTab {
	cfg := tabConfig{name: src.Name()}
	for _, o := range opts {
		o(&cfg)
	}
	t := &SourceTab{
		w:     w,
		src:   src,
		feed:  src.Subscribe(0),
		reqs:  make(chan quevu.Request, 64),
		finds: make(chan find, 1),
		done:  make(chan struct{}),
	}
	t.id = uiConsoleAdd(cfg.name, int32(cfg.weight))
	w.mu.Lock()
	w.sources[t.id] = t
	w.mu.Unlock()

	w.wg.Add(3)
	go func() {
		defer w.wg.Done()
		t.finder()
	}()
	go func() {
		defer w.wg.Done()
		t.down()
	}()
	go func() {
		defer w.wg.Done()
		t.up()
	}()
	return t
}

// Rename changes the name of the tab.
func (t *SourceTab) Rename(name string) { uiTabTitle(t.id, name) }

// Select brings the tab to the front.
func (t *SourceTab) Select() { uiTabSelect(t.id) }

// ShowTime shows, or stops showing, next to each line when it entered the
// log. The user can change it too, from the menu of the tab.
func (t *SourceTab) ShowTime(on bool) { uiConsoleTime(t.id, flag(on)) }

// Close closes the tab. Its source is left as it was.
func (t *SourceTab) Close() {
	t.stop()
	uiTabRemove(t.id)
}

// stop ends the goroutines of the tab and lets go of its feed.
func (t *SourceTab) stop() {
	t.once.Do(func() {
		t.w.mu.Lock()
		delete(t.w.sources, t.id)
		t.w.mu.Unlock()
		close(t.done)
		t.feed.Close()
	})
}

// findText reads the text a console is looking for.
func findText(tab uint32) string { return uiFindText(tab) }

// findRange reads the time range a console is searching within. A zero
// time is no bound on that side.
func findRange(tab uint32) (since, until time.Time) {
	s, u := uiFindRange(tab)
	if s != 0 {
		since = time.Unix(0, s)
	}
	if u != 0 {
		until = time.Unix(0, u)
	}
	return since, until
}

// finder runs the searches of the tab, one step at a time. The step is
// the source's to run: here it is only asked for, and its outcome shown.
func (t *SourceTab) finder() {
	w := t.w
	for {
		var f find
		select {
		case <-t.done:
			return
		case f = <-t.finds:
		}
		w.mu.Lock()
		budget := w.budget
		w.mu.Unlock()
		res, err := quevu.Search(t.src, quevu.Query{Text: f.text, From: f.from, Forward: f.newer, Budget: budget, History: f.history, Since: f.since, Until: f.until})
		if os.Getenv("QUEVU_UI_TRACE") != "" {
			fmt.Fprintf(os.Stderr, "find %q from=%d newer=%v history=%v range=[%s, %s] -> %d matches, reached=%d examined=%d done=%v out-of-range=%v offer=%v err=%v\n",
				f.text, f.from, f.newer, f.history, w.stamp(f.since), w.stamp(f.until),
				len(res.Matches), res.Reached, res.Examined, res.Done, res.OutOfRange, res.History, err)
		}
		switch {
		case err != nil:
			uiFindStatus(t.id, fmt.Sprintf(w.text(TextFindFailed), err), 0, 0)
		case len(res.Matches) > 0:
			t.showFound(f, res.Matches[0])
		case res.History:
			// Recent output has no more of it. There is a history, and
			// looking into it is the user's decision, not this one's.
			uiFindStatus(t.id, fmt.Sprintf(w.text(TextFindHistory), f.text, res.Examined), res.Reached, flagHistory)
		default:
			uiFindStatus(t.id, w.notFound(f, res), resumeFrom(res), 0)
		}
	}
}

// showFound moves the console to a match. Only the line found is handed
// over: the console asks its source for what surrounds it, the same way
// it does when the user scrolls, so this package never has to know which
// provider holds what.
func (t *SourceTab) showFound(f find, m quevu.Chunk) {
	flags := uint32(0)
	if f.history {
		flags = flagHistory
	}
	one := []quevu.Chunk{m}
	cs := toC(one)
	uiConsoleGoto(t.id, cs, m.Seq, flags)
	uiFindStatus(t.id, fmt.Sprintf(t.w.text(TextFound), f.text, t.w.stamp(m.Time)), 0, 0)
}

// stamp writes a moment the way the texts of a search show it.
func (w *Window) stamp(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	return t.Local().Format(w.text(TextTimeLayout))
}

// resumeFrom is where the next step starts if the user insists: nowhere
// when the step ran out of log.
func resumeFrom(res quevu.Result) uint64 {
	if res.Done {
		return 0
	}
	return res.Reached
}

// notFound says how far a step that found nothing got.
func (w *Window) notFound(f find, res quevu.Result) string {
	switch {
	case res.OutOfRange:
		return fmt.Sprintf(w.text(TextFindOutOfRange), f.text, res.Examined)
	case res.Done && f.newer:
		return fmt.Sprintf(w.text(TextFindEndNewer), f.text, res.Examined)
	case res.Done:
		return fmt.Sprintf(w.text(TextFindEndOlder), f.text, res.Examined)
	}
	if res.ReachedTime == 0 {
		id := TextFindMoreOlder
		if f.newer {
			id = TextFindMoreNewer
		}
		return fmt.Sprintf(w.text(id), f.text, res.Examined)
	}
	id := TextFindMoreOlderAt
	if f.newer {
		id = TextFindMoreNewerAt
	}
	return fmt.Sprintf(w.text(id), f.text, res.Examined, w.stamp(time.Unix(0, res.ReachedTime)))
}

// up carries the requests of the console to its source.
func (t *SourceTab) up() {
	for {
		select {
		case <-t.done:
			return
		case r := <-t.reqs:
			select {
			case t.feed.Req <- r:
			case <-t.done:
				return
			}
		}
	}
}

// down carries what the source delivers to the console.
func (t *SourceTab) down() {
	for d := range t.feed.Data {
		if d.Err != nil {
			// The console must hear the answer either way, or it would
			// wait for it forever.
			d.Chunks = nil
		}
		deliver(t.id, d)
	}
}

func deliver(tab uint32, d quevu.Delivery) {
	flags := uint32(0)
	if d.Full {
		flags |= flagFull
	}
	if d.History {
		// The source's offer: the window shows it, the user decides.
		flags |= flagHistory
	}
	uiConsoleDeliver(tab, uint32(d.Op), flags, toC(d.Chunks))
}

// toC hands chunks over the way the window reads them. The data is not
// copied here: the window copies it before the call returns.
func toC(chunks []quevu.Chunk) []uiChunk {
	cs := make([]uiChunk, len(chunks))
	for i, c := range chunks {
		cs[i] = uiChunk{seq: c.Seq, timeNs: c.Time.UnixNano(), data: c.Data}
	}
	return cs
}

// showStats shows the queues of each log in the strip of its tab. A
// queue is there to absorb bursts; one that keeps growing says the
// writer behind it is too slow for this output.
func (w *Window) showStats(every time.Duration) {
	t := time.NewTicker(every)
	defer t.Stop()
	last := make(map[uint32]string)
	for {
		select {
		case <-w.done:
			return
		case <-t.C:
		}
		w.mu.Lock()
		format := w.format
		open := make([]*SourceTab, 0, len(w.sources))
		for _, s := range w.sources {
			open = append(open, s)
		}
		w.mu.Unlock()
		for _, s := range open {
			ss, ok := s.src.(quevu.StatSource)
			if !ok {
				continue
			}
			note := format(ss.Stats())
			if note != last[s.id] {
				last[s.id] = note
				uiTabNote(s.id, note)
			}
		}
	}
}

// formatStats writes the queues of a log, in English: the default of
// Window.SetStats.
func formatStats(stats []quevu.WriterStats) string {
	parts := make([]string, 0, len(stats))
	for i, s := range stats {
		name := s.Name
		if name == "" {
			name = fmt.Sprintf("writer %d", i+1)
		}
		p := fmt.Sprintf("%s: %d/%d, taken %d", name, s.Queued, s.Capacity, s.Written)
		if s.Waiting {
			p += ", HOLDING THE HUB"
		}
		if s.Waits > 0 {
			p += fmt.Sprintf(", waited for %d times", s.Waits)
		}
		if s.Skipped > 0 {
			p += fmt.Sprintf(", skipped %d", s.Skipped)
		}
		if s.Failed > 0 {
			p += fmt.Sprintf(", failed %d", s.Failed)
		}
		if s.LastErr != nil {
			p += " (" + s.LastErr.Error() + ")"
		}
		parts = append(parts, p)
	}
	return strings.Join(parts, "; ")
}
