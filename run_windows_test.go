//go:build cgo

package qui

import (
	"context"
	"errors"
	"runtime"
	"testing"
	"time"

	"github.com/pablo-botella/quevu"
)

// The usual window of a panel, as a real window, for a moment: the console asks for its
// first page through the event lane, and the host closes the window.
func TestWindowRoundTrip(t *testing.T) {

	src := &fakeSource{got: make(chan quevu.Request, 16)}
	panel := quevu.New()
	panel.AddSource(src)
	started := make(chan struct{})
	panel.Add("a", quevu.Func(func(context.Context) error { close(started); return nil }, nil))
	panel.Add("b", quevu.Func(func(context.Context) error { return errors.New("broken") }, nil))

	closed := make(chan struct{})
	shown := make(chan struct{})
	ran := make(chan error, 1)
	go func() {
		// A window lives and dies on one thread.
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		ran <- Run("qui test", panel, OnReady(func() { close(shown) }), OnClose(func() { close(closed) }), WithStats(0))
	}()

	// The host is told once the window is on the screen.
	select {
	case <-shown:
	case <-time.After(10 * time.Second):
		t.Fatal("the host was never told that the window is there")
	}

	select {
	case r := <-src.got:
		if r.Op != quevu.OpFollow || r.N <= 0 {
			t.Fatalf("the console opened with %+v", r)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the console never asked for its first page")
	}
	// State changes reach the window without blocking anybody.
	if err := panel.Start(context.Background(), "a"); err != nil {
		t.Fatal(err)
	}
	<-started
	panel.Start(context.Background(), "b")

	Quit()
	select {
	case err := <-ran:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the window did not close")
	}
	select {
	case <-closed:
		t.Fatal("a quit from the host must not run the user's close handler")
	default:
	}
}
