//go:build cgo

package qui

import "testing"

// The parts of the window that need no window are checked in C, where
// they live: the text of a line, the order of the tabs, the model of a
// console and the two lanes between the host and the window.
func TestThePartsThatNeedNoWindow(t *testing.T) {
	if msg := selfTest(); msg != "" {
		t.Fatal(msg)
	}
}
