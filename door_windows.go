//go:build cgo

package qui

/*
#cgo CFLAGS: -DUNICODE -D_UNICODE -D_WIN32_WINNT=0x0A00 -DWINVER=0x0A00 -Wall
#cgo LDFLAGS: -lcomctl32 -lshell32 -lgdi32 -luser32 -lkernel32

#include <stdlib.h>
#include <string.h>
#include "qv.h"
*/
import "C"

import "unsafe"

// The door: every function of quevu_ui.h, with Go values in place of C
// ones. The rest of the package calls these and nothing else of C.
//
// A string or a slice is copied into C memory for the length of the call:
// C may not be handed Go memory that points at more Go memory, and the
// window copies what it keeps before the call returns.

const uiAvailable = true

func uiRun(title string) int32 {
	t := C.CString(title)
	defer C.free(unsafe.Pointer(t))
	return int32(C.quevu_ui_run(t))
}

func uiQuit() { C.quevu_ui_quit() }

func uiText(id uint32, text string) {
	t := C.CString(text)
	defer C.free(unsafe.Pointer(t))
	C.quevu_ui_text(C.uint32_t(id), t)
}

func uiConsoleAdd(title string, weight int32) uint32 {
	t := C.CString(title)
	defer C.free(unsafe.Pointer(t))
	return uint32(C.quevu_ui_console_add(t, C.int32_t(weight)))
}

func uiControlsAdd(title string, weight int32) uint32 {
	t := C.CString(title)
	defer C.free(unsafe.Pointer(t))
	return uint32(C.quevu_ui_controls_add(t, C.int32_t(weight)))
}

func uiTabTitle(tab uint32, title string) {
	t := C.CString(title)
	defer C.free(unsafe.Pointer(t))
	C.quevu_ui_tab_title(C.uint32_t(tab), t)
}

func uiTabSelect(tab uint32) { C.quevu_ui_tab_select(C.uint32_t(tab)) }

func uiTabRemove(tab uint32) { C.quevu_ui_tab_remove(C.uint32_t(tab)) }

func uiTabNote(tab uint32, note string) {
	t := C.CString(note)
	defer C.free(unsafe.Pointer(t))
	C.quevu_ui_tab_note(C.uint32_t(tab), t)
}

func uiTabVerbs(tab uint32, verbs []uiVerb) {
	vs, free := cVerbs(verbs)
	defer free()
	C.quevu_ui_tab_verbs(C.uint32_t(tab), vs, C.uint32_t(len(verbs)))
}

func uiControlAdd(tab uint32, name string) uint32 {
	t := C.CString(name)
	defer C.free(unsafe.Pointer(t))
	return uint32(C.quevu_ui_control_add(C.uint32_t(tab), t))
}

func uiControlState(control, rgb, attrs uint32, state string, verbs []uiVerb) {
	t := C.CString(state)
	defer C.free(unsafe.Pointer(t))
	vs, free := cVerbs(verbs)
	defer free()
	C.quevu_ui_control_state(C.uint32_t(control), C.uint32_t(rgb), C.uint32_t(attrs), t, vs, C.uint32_t(len(verbs)))
}

func uiControlRemove(control uint32) { C.quevu_ui_control_remove(C.uint32_t(control)) }

func uiConsoleDeliver(tab, op, flags uint32, chunks []uiChunk) {
	cs, free := cChunks(chunks)
	defer free()
	C.quevu_ui_console_deliver(C.uint32_t(tab), C.uint32_t(op), C.uint32_t(flags), cs, C.uint32_t(len(chunks)))
}

func uiConsoleTime(tab, on uint32) { C.quevu_ui_console_time(C.uint32_t(tab), C.uint32_t(on)) }

func uiMenuAdd(parent uint32, label string, attrs uint32) uint32 {
	t := C.CString(label)
	defer C.free(unsafe.Pointer(t))
	return uint32(C.quevu_ui_menu_add(C.uint32_t(parent), t, C.uint32_t(attrs)))
}

func uiMenuSet(item uint32, label string, attrs uint32) {
	t := C.CString(label)
	defer C.free(unsafe.Pointer(t))
	C.quevu_ui_menu_set(C.uint32_t(item), t, C.uint32_t(attrs))
}

func uiMenuRemove(item uint32) { C.quevu_ui_menu_remove(C.uint32_t(item)) }

func uiTray(enabled uint32) { C.quevu_ui_tray(C.uint32_t(enabled)) }

func uiShow(visible uint32) { C.quevu_ui_show(C.uint32_t(visible)) }

// uiNextEvent waits for the next thing the user did: 1 with the event, 0
// if nothing happened in time, -1 once the window is gone.
func uiNextEvent(timeoutMs int32) (uiEvent, int32) {
	var ev C.quevu_ui_event
	code := int32(C.quevu_ui_next_event(&ev, C.int32_t(timeoutMs)))
	return uiEvent{kind: uint32(ev.kind), id: uint32(ev.id), op: uint32(ev.op), n: uint32(ev.n), seq: uint64(ev.seq)}, code
}

func uiFindText(tab uint32) string {
	// The first call says how long the text is.
	n := C.quevu_ui_console_find_text(C.uint32_t(tab), nil, 0)
	if n == 0 {
		return ""
	}
	buf := (*C.char)(C.malloc(C.size_t(n) + 1))
	defer C.free(unsafe.Pointer(buf))
	C.quevu_ui_console_find_text(C.uint32_t(tab), buf, n+1)
	return C.GoString(buf)
}

func uiFindRange(tab uint32) (sinceNs, untilNs int64) {
	var since, until C.int64_t
	C.quevu_ui_console_find_range(C.uint32_t(tab), &since, &until)
	return int64(since), int64(until)
}

func uiConsoleGoto(tab uint32, chunks []uiChunk, mark uint64, flags uint32) {
	cs, free := cChunks(chunks)
	defer free()
	C.quevu_ui_console_goto(C.uint32_t(tab), cs, C.uint32_t(len(chunks)), C.uint64_t(mark), C.uint32_t(flags))
}

func uiFindStatus(tab uint32, status string, resume uint64, flags uint32) {
	t := C.CString(status)
	defer C.free(unsafe.Pointer(t))
	C.quevu_ui_console_find_status(C.uint32_t(tab), t, C.uint64_t(resume), C.uint32_t(flags))
}

// cVerbs lays verbs out in C memory, labels included, in one block.
func cVerbs(verbs []uiVerb) (*C.quevu_ui_verb, func()) {
	if len(verbs) == 0 {
		return nil, func() {}
	}
	head := C.size_t(len(verbs)) * C.size_t(unsafe.Sizeof(C.quevu_ui_verb{}))
	size := head
	for _, v := range verbs {
		size += C.size_t(len(v.label) + 1)
	}
	block := C.malloc(size)
	out := unsafe.Slice((*C.quevu_ui_verb)(block), len(verbs))
	text := unsafe.Slice((*byte)(unsafe.Add(block, head)), int(size-head))
	at := 0
	for i, v := range verbs {
		out[i].id = C.uint32_t(v.id)
		out[i].attrs = C.uint32_t(v.attrs)
		out[i].label = (*C.char)(unsafe.Pointer(&text[at]))
		at += copy(text[at:], v.label)
		text[at] = 0
		at++
	}
	return &out[0], func() { C.free(block) }
}

// cChunks does the same for chunks and their data.
func cChunks(chunks []uiChunk) (*C.quevu_ui_chunk, func()) {
	if len(chunks) == 0 {
		return nil, func() {}
	}
	head := C.size_t(len(chunks)) * C.size_t(unsafe.Sizeof(C.quevu_ui_chunk{}))
	size := head
	for _, c := range chunks {
		size += C.size_t(len(c.data))
	}
	// One byte more, so the data of an empty last chunk has an address.
	block := C.malloc(size + 1)
	out := unsafe.Slice((*C.quevu_ui_chunk)(block), len(chunks))
	data := unsafe.Slice((*byte)(unsafe.Add(block, head)), int(size-head)+1)
	at := 0
	for i, c := range chunks {
		out[i].seq = C.uint64_t(c.seq)
		out[i].time_ns = C.int64_t(c.timeNs)
		out[i].data = (*C.uint8_t)(unsafe.Pointer(&data[at]))
		out[i].len = C.uint32_t(len(c.data))
		at += copy(data[at:], c.data)
	}
	return &out[0], func() { C.free(block) }
}

// selfTest runs the checks of the parts that need no window. It returns
// what failed, or nothing.
func selfTest() string {
	if msg := C.qv_selftest(); msg != nil {
		return C.GoString(msg)
	}
	return ""
}
