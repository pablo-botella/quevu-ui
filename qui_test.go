package qui

import (
	"errors"
	"testing"

	"github.com/pablo-botella/quevu"
)

func TestFormatStats(t *testing.T) {
	got := formatStats([]quevu.WriterStats{
		{Queued: 3, Capacity: 4096, Written: 120},
		{Name: "disk", Queued: 4096, Capacity: 4096, Written: 7, Waiting: true, Waits: 3, Skipped: 2, Failed: 5, LastErr: errors.New("disk full")},
	})
	want := "writer 1: 3/4096, taken 120; disk: 4096/4096, taken 7, HOLDING THE HUB, waited for 3 times, skipped 2, failed 5 (disk full)"
	if got != want {
		t.Fatalf("got  %q\nwant %q", got, want)
	}
	if formatStats(nil) != "" {
		t.Fatal("no writers should be no note")
	}
}

// A program with no window says so, and nothing else changes: the same
// calls can be made and Run returns an error that is not a failure.
func TestAProgramSaysWhetherItHasAWindow(t *testing.T) {
	if Available() {
		t.Skip("this program has a window")
	}
	w := NewWindow("nobody will see this")
	w.OpenControls("Services").Add("Web server").Set(State{Color: Green, Text: "listening"})
	if err := w.Run(); !IsNoWindow(err) {
		t.Fatalf("with no window, Run said %v", err)
	}
}
