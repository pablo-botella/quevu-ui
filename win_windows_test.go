//go:build windows

package qui

import (
	"image"
	"image/png"
	"os"
	"syscall"
	"testing"
	"time"
	"unsafe"
)

// What the tests need of Windows to look at a window from outside and to
// do what a user would: press a button, choose a menu entry, close it.

var (
	user32 = syscall.NewLazyDLL("user32.dll")
	gdi32  = syscall.NewLazyDLL("gdi32.dll")

	procFindWindow       = user32.NewProc("FindWindowW")
	procFindWindowEx     = user32.NewProc("FindWindowExW")
	procGetClassName     = user32.NewProc("GetClassNameW")
	procGetWindowText    = user32.NewProc("GetWindowTextW")
	procIsWindowVisible  = user32.NewProc("IsWindowVisible")
	procIsWindowEnabled  = user32.NewProc("IsWindowEnabled")
	procPostMessage      = user32.NewProc("PostMessageW")
	procSendMessage      = user32.NewProc("SendMessageW")
	procGetMenu          = user32.NewProc("GetMenu")
	procGetSubMenu       = user32.NewProc("GetSubMenu")
	procGetMenuItemCount = user32.NewProc("GetMenuItemCount")
	procGetMenuItemID    = user32.NewProc("GetMenuItemID")
	procGetMenuString    = user32.NewProc("GetMenuStringW")
	procGetMenuState     = user32.NewProc("GetMenuState")
	procGetWindowRect    = user32.NewProc("GetWindowRect")
	procGetDC            = user32.NewProc("GetDC")
	procReleaseDC        = user32.NewProc("ReleaseDC")
	procPrintWindow      = user32.NewProc("PrintWindow")
	procCreateDC         = gdi32.NewProc("CreateCompatibleDC")
	procCreateBitmap     = gdi32.NewProc("CreateCompatibleBitmap")
	procSelectObject     = gdi32.NewProc("SelectObject")
	procDeleteObject     = gdi32.NewProc("DeleteObject")
	procDeleteDC         = gdi32.NewProc("DeleteDC")
	procGetDIBits        = gdi32.NewProc("GetDIBits")
)

const (
	wmClose   = 0x0010
	wmCommand = 0x0111
	bmClick   = 0x00f5

	tcmGetItemCount = 0x1304
	tcmGetItemW     = 0x133c
	tcmGetCurSel    = 0x130b
	tcifText        = 1

	mfByPosition = 0x400
	mfChecked    = 0x8
	mfGrayed     = 0x1
	mfSeparator  = 0x800
)

func utf16(s string) *uint16 {
	p, _ := syscall.UTF16PtrFromString(s)
	return p
}

// topWindow waits for the window with a title to exist.
func topWindow(t *testing.T, title string) uintptr {
	t.Helper()
	for i := 0; i < 500; i++ {
		if h, _, _ := procFindWindow.Call(0, uintptr(unsafe.Pointer(utf16(title)))); h != 0 {
			return h
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("no window titled %q", title)
	return 0
}

func className(h uintptr) string {
	var buf [128]uint16
	n, _, _ := procGetClassName.Call(h, uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf)))
	return syscall.UTF16ToString(buf[:n])
}

func windowText(h uintptr) string {
	var buf [512]uint16
	n, _, _ := procGetWindowText.Call(h, uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf)))
	return syscall.UTF16ToString(buf[:n])
}

func visible(h uintptr) bool {
	v, _, _ := procIsWindowVisible.Call(h)
	return v != 0
}

func enabled(h uintptr) bool {
	v, _, _ := procIsWindowEnabled.Call(h)
	return v != 0
}

// descendants lists every window under one, at any depth.
func descendants(parent uintptr) []uintptr {
	var out []uintptr
	var after uintptr
	for {
		h, _, _ := procFindWindowEx.Call(parent, after, 0, 0)
		if h == 0 {
			return out
		}
		out = append(out, h)
		out = append(out, descendants(h)...)
		after = h
	}
}

// button finds a button by the words on it. With shown, only one that the
// user could see: every window between it and the top one is visible.
func button(top uintptr, label string) uintptr {
	for _, h := range descendants(top) {
		if className(h) == "Button" && windowText(h) == label && visible(h) {
			return h
		}
	}
	return 0
}

// waitButton waits for a button to be there, or to be gone.
func waitButton(t *testing.T, top uintptr, label string, there bool) uintptr {
	t.Helper()
	for i := 0; i < 500; i++ {
		h := button(top, label)
		if (h != 0) == there {
			return h
		}
		time.Sleep(10 * time.Millisecond)
	}
	if there {
		t.Fatalf("no button %q on the window", label)
	}
	t.Fatalf("the button %q is still on the window", label)
	return 0
}

func click(h uintptr) { procPostMessage.Call(h, bmClick, 0, 0) }

type tcItem struct {
	mask      uint32
	state     uint32
	stateMask uint32
	_         uint32
	text      *uint16
	cch       int32
	image     int32
	lparam    uintptr
}

// tabTitles reads the row of tabs, in the order it shows them, and which
// one is in front.
func tabTitles(top uintptr) (titles []string, front int) {
	for _, h := range descendants(top) {
		if className(h) != "SysTabControl32" {
			continue
		}
		n, _, _ := procSendMessage.Call(h, tcmGetItemCount, 0, 0)
		for i := uintptr(0); i < n; i++ {
			var buf [256]uint16
			it := tcItem{mask: tcifText, text: &buf[0], cch: int32(len(buf))}
			procSendMessage.Call(h, tcmGetItemW, i, uintptr(unsafe.Pointer(&it)))
			titles = append(titles, syscall.UTF16ToString(buf[:]))
		}
		sel, _, _ := procSendMessage.Call(h, tcmGetCurSel, 0, 0)
		return titles, int(int32(sel))
	}
	return nil, -1
}

// waitTabs waits for the row of tabs to read as given.
func waitTabs(t *testing.T, top uintptr, want ...string) {
	t.Helper()
	var got []string
	for i := 0; i < 500; i++ {
		got, _ = tabTitles(top)
		if equalStrings(got, want) {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("tabs are %q, want %q", got, want)
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// menuEntry is one entry of a menu as Windows holds it.
type menuLine struct {
	label   string
	id      uint32
	sub     uintptr
	checked bool
	grayed  bool
	line    bool
}

func menuLines(menu uintptr) []menuLine {
	if menu == 0 {
		return nil
	}
	n, _, _ := procGetMenuItemCount.Call(menu)
	var out []menuLine
	for i := uintptr(0); int32(n) > 0 && i < n; i++ {
		var buf [256]uint16
		procGetMenuString.Call(menu, i, uintptr(unsafe.Pointer(&buf[0])), uintptr(len(buf)), mfByPosition)
		id, _, _ := procGetMenuItemID.Call(menu, i)
		sub, _, _ := procGetSubMenu.Call(menu, i)
		state, _, _ := procGetMenuState.Call(menu, i, mfByPosition)
		out = append(out, menuLine{
			label:   syscall.UTF16ToString(buf[:]),
			id:      uint32(id),
			sub:     sub,
			checked: state&mfChecked != 0,
			grayed:  state&mfGrayed != 0,
			line:    state&mfSeparator != 0 && sub == 0,
		})
	}
	return out
}

// menuBarOf reads the bar of a window: nil if it has none.
func menuBarOf(top uintptr) []menuLine {
	m, _, _ := procGetMenu.Call(top)
	return menuLines(m)
}

// waitMenu waits for the entry at a path of labels to be on the bar.
func waitMenu(t *testing.T, top uintptr, path ...string) menuLine {
	t.Helper()
	for i := 0; i < 500; i++ {
		lines := menuBarOf(top)
		var found menuLine
		ok := true
		for _, label := range path {
			ok = false
			for _, l := range lines {
				if l.label == label {
					found, ok = l, true
					lines = menuLines(l.sub)
					break
				}
			}
			if !ok {
				break
			}
		}
		if ok {
			return found
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("no menu entry %q", path)
	return menuLine{}
}

// choose does what choosing a menu entry does.
func choose(top uintptr, l menuLine) { procPostMessage.Call(top, wmCommand, uintptr(l.id), 0) }

// askToClose does what the cross of the window does.
func askToClose(top uintptr) { procPostMessage.Call(top, wmClose, 0, 0) }

type bitmapInfoHeader struct {
	size          uint32
	width, height int32
	planes, bits  uint16
	compression   uint32
	sizeImage     uint32
	xppm, yppm    int32
	used, needed  uint32
}

// shot saves a picture of a window where QUEVU_UI_SHOTS says, for a
// person to look at. It does nothing when the variable is not set.
func shot(top uintptr, name string) {
	dir := os.Getenv("QUEVU_UI_SHOTS")
	if dir == "" {
		return
	}
	// Give the window a moment to paint what it was just told.
	time.Sleep(400 * time.Millisecond)
	var rc [4]int32
	procGetWindowRect.Call(top, uintptr(unsafe.Pointer(&rc[0])))
	w, h := int(rc[2]-rc[0]), int(rc[3]-rc[1])
	if w <= 0 || h <= 0 {
		return
	}
	screen, _, _ := procGetDC.Call(0)
	dc, _, _ := procCreateDC.Call(screen)
	bmp, _, _ := procCreateBitmap.Call(screen, uintptr(w), uintptr(h))
	old, _, _ := procSelectObject.Call(dc, bmp)
	const renderFullContent = 2
	procPrintWindow.Call(top, dc, renderFullContent)
	procSelectObject.Call(dc, old)

	img := image.NewRGBA(image.Rect(0, 0, w, h))
	bi := bitmapInfoHeader{size: 40, width: int32(w), height: -int32(h), planes: 1, bits: 32}
	procGetDIBits.Call(dc, bmp, 0, uintptr(h), uintptr(unsafe.Pointer(&img.Pix[0])), uintptr(unsafe.Pointer(&bi)), 0)
	// Windows hands the pixels over as blue, green, red.
	for i := 0; i+3 < len(img.Pix); i += 4 {
		img.Pix[i], img.Pix[i+2] = img.Pix[i+2], img.Pix[i]
		img.Pix[i+3] = 0xff
	}
	procDeleteObject.Call(bmp)
	procDeleteDC.Call(dc)
	procReleaseDC.Call(0, screen)

	os.MkdirAll(dir, 0o755)
	if f, err := os.Create(dir + string(os.PathSeparator) + name + ".png"); err == nil {
		png.Encode(f, img)
		f.Close()
	}
}

func unsafePointer(s string) unsafe.Pointer { return unsafe.Pointer(utf16(s)) }
