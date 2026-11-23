//! The quevu visual part as a dynamic library with a C interface.
//!
//! This crate is only the door: it turns C calls into commands for the UI
//! thread and hands events back. The window itself lives in a backend
//! crate chosen by the target system, and the logic both share lives in
//! `quevu-ui-core`.
//!
//! The interface is described for C in `include/quevu_ui.h`. Its rules:
//!
//! - `quevu_ui_run` blocks the calling thread for the life of the window.
//!   On macOS that must be the main thread of the process, so a host that
//!   wants to be portable calls it from there everywhere.
//! - Every other function may be called from any thread, before or while
//!   `quevu_ui_run` is running, and never waits for the window.
//! - Nothing calls back into the host. What the user does is read with
//!   `quevu_ui_next_event`, from a thread of the host's choosing.
//! - Strings are NUL-terminated UTF-8, copied before the call returns.
//! - The window has no word of its own and decides nothing: every tab,
//!   control, verb, menu entry and text is the host's, and so is the
//!   decision to close.

use std::ffi::{c_char, CStr};
use std::sync::{Arc, OnceLock};

use quevu_ui_core::console::Line;
use quevu_ui_core::shared::{Command, Event, Next, Shared, Verb};
use quevu_ui_core::text::display_line;

/// Version of this interface. A host checks it before anything else.
pub const QUEVU_UI_ABI: u32 = 5;

/// Flag of a delivery: the source paused the console by itself.
pub const QUEVU_FLAG_FULL: u32 = 1;
/// Flag: there is a history, or the history is involved, depending on
/// the call.
pub const QUEVU_FLAG_HISTORY: u32 = 2;

/// A chunk as the host hands it over. The data is copied before the call
/// returns.
#[repr(C)]
pub struct QuevuUiChunk {
    pub seq: u64,
    pub time_ns: i64,
    pub data: *const u8,
    pub len: u32,
}

/// A verb as the host hands it over: the number that comes back when the
/// user presses it, its attributes and the words on its button. The label
/// is copied before the call returns.
#[repr(C)]
pub struct QuevuUiVerb {
    pub id: u32,
    pub attrs: u32,
    pub label: *const c_char,
}

/// Something the user did. See `quevu_ui_core::shared::event` for kinds.
pub type QuevuUiEvent = Event;

fn shared() -> &'static Arc<Shared> {
    static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(Shared::new()))
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    CStr::from_ptr(p).to_string_lossy().into_owned()
}

/// Copies the verbs the host hands over.
unsafe fn verbs_from(verbs: *const QuevuUiVerb, n: u32) -> Vec<Verb> {
    if verbs.is_null() {
        return Vec::new();
    }
    std::slice::from_raw_parts(verbs, n as usize)
        .iter()
        .map(|v| Verb { id: v.id, attrs: v.attrs, label: text(v.label) })
        .collect()
}

#[cfg(windows)]
fn backend_run(title: &str) -> i32 {
    quevu_ui_win32::run(title, shared().clone())
}

#[cfg(not(windows))]
fn backend_run(_title: &str) -> i32 {
    // No backend for this system yet.
    -1
}

/// Copies the chunks the host hands over into lines a console can draw.
unsafe fn lines_from(chunks: *const QuevuUiChunk, n: u32) -> Vec<Line> {
    let mut lines = Vec::with_capacity(n as usize);
    if !chunks.is_null() {
        for c in std::slice::from_raw_parts(chunks, n as usize) {
            let data: &[u8] = if c.data.is_null() || c.len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(c.data, c.len as usize)
            };
            let (text, spans) = display_line(data);
            lines.push(Line { seq: c.seq, time_ns: c.time_ns, text, spans });
        }
    }
    lines
}

/// Returns the version of the interface this library implements.
#[no_mangle]
pub extern "C" fn quevu_ui_abi() -> u32 {
    QUEVU_UI_ABI
}

/// Opens the window and runs its event loop on the calling thread until
/// `quevu_ui_quit`. Returns 0 on a clean end, -1 if this system has no
/// visual part, another value if the window could not be created.
///
/// Whatever was asked for before this call is on the window when it
/// first shows: it does not open empty and fill up afterwards.
///
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_run(title: *const c_char) -> i32 {
    backend_run(&text(title))
}

/// Ends the event loop: `quevu_ui_run` returns. It is the only way the
/// window goes away. A user who asks to close it only sends an event.
#[no_mangle]
pub extern "C" fn quevu_ui_quit() {
    shared().post(Command::Quit);
}

/// Sets one of the texts the window shows on its own account: what a
/// source tab says of its view and of its search, and the entries of its
/// menu. They are numbered in `quevu_ui.h`. Until the host sets a text it
/// is empty, and its place on the window stays blank.
///
/// # Safety
/// `value` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_text(id: u32, value: *const c_char) {
    shared().post(Command::Text { id, text: text(value) });
}

/// Opens a tab with a pseudo-console and returns its identifier. The tab
/// takes its place in the row by `weight`: the heavier, the nearer the
/// start; of two that weigh the same, the one opened first goes first.
/// The console asks for its first page at once, with a console request
/// event.
///
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_add(title: *const c_char, weight: i32) -> u32 {
    let id = shared().new_id();
    shared().post(Command::ConsoleAdd { id, title: text(title), weight });
    id
}

/// Opens a tab with a table of controls and returns its identifier. It
/// takes its place in the row by `weight`, like any other tab.
///
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_controls_add(title: *const c_char, weight: i32) -> u32 {
    let id = shared().new_id();
    shared().post(Command::ControlsAdd { id, title: text(title), weight });
    id
}

/// Changes the name of a tab.
///
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_tab_title(tab: u32, title: *const c_char) {
    shared().post(Command::TabTitle { id: tab, title: text(title) });
}

/// Brings a tab to the front.
#[no_mangle]
pub extern "C" fn quevu_ui_tab_select(tab: u32) {
    shared().post(Command::TabSelect { id: tab });
}

/// Closes a tab, with everything on it.
#[no_mangle]
pub extern "C" fn quevu_ui_tab_remove(tab: u32) {
    shared().post(Command::TabRemove { id: tab });
}

/// Sets what the status strip of a tab says on the host's account. On a
/// source tab it goes next to the state of the view: the place for the
/// queues of its log. On a control tab it is all the strip says.
///
/// # Safety
/// `note` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_tab_note(tab: u32, note: *const c_char) {
    shared().post(Command::TabNote { tab, text: text(note) });
}

/// Sets the verbs of a control tab as a whole: the buttons of its bar,
/// in the order given. Pressing one sends a tab verb event with the
/// identifier of the tab and of the verb. The list replaces the one
/// before it; an empty one leaves the tab with no buttons.
///
/// # Safety
/// `verbs` must point to `n` valid verbs, each with a null or valid
/// NUL-terminated label; it may be null when `n` is zero.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_tab_verbs(tab: u32, verbs: *const QuevuUiVerb, n: u32) {
    shared().post(Command::TabVerbs { tab, verbs: verbs_from(verbs, n) });
}

/// Adds a row to a control tab and returns the identifier of the control.
/// The row shows its name and an unlit light until its state is set.
///
/// # Safety
/// `name` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_control_add(tab: u32, name: *const c_char) -> u32 {
    let id = shared().new_id();
    shared().post(Command::ControlAdd { id, tab, name: text(name) });
    id
}

/// Sets everything a control shows: the color of its light as 0x00RRGGBB,
/// the attributes of the light (`QUEVU_LIGHT_*`), the text of its state
/// and the verbs it offers from now on, in the order given. Pressing one
/// sends a control verb event with the identifier of the control and of
/// the verb. The window draws what it is told and understands none of it.
///
/// # Safety
/// `state` must be null or a valid NUL-terminated string. `verbs` must
/// point to `n` valid verbs, each with a null or valid NUL-terminated
/// label; it may be null when `n` is zero.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_control_state(
    control: u32,
    rgb: u32,
    attrs: u32,
    state: *const c_char,
    verbs: *const QuevuUiVerb,
    n: u32,
) {
    shared().post(Command::ControlState { id: control, rgb, attrs, text: text(state), verbs: verbs_from(verbs, n) });
}

/// Removes a control from its tab.
#[no_mangle]
pub extern "C" fn quevu_ui_control_remove(control: u32) {
    shared().post(Command::ControlRemove { id: control });
}

/// Hands a console what came down its data lane: `op` is the operation
/// being answered (1 for live output). In `flags`, `QUEVU_FLAG_FULL` marks
/// a pause the source decided by itself and `QUEVU_FLAG_HISTORY` an answer
/// that ran short of recent output while a history exists.
///
/// # Safety
/// `chunks` must point to `n` valid chunks, each with `len` readable
/// bytes at `data`; it may be null when `n` is zero.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_deliver(
    tab: u32,
    op: u32,
    flags: u32,
    chunks: *const QuevuUiChunk,
    n: u32,
) {
    shared().post(Command::Deliver {
        tab,
        op,
        full: flags & QUEVU_FLAG_FULL != 0,
        history: flags & QUEVU_FLAG_HISTORY != 0,
        lines: lines_from(chunks, n),
    });
}

/// Reads the text the console of a tab is looking for, after a console
/// find event. Copies at most `cap - 1` bytes of UTF-8 plus a NUL into
/// `buf` and returns the length of the whole text, which may be larger.
///
/// # Safety
/// `buf` must point to `cap` writable bytes; it may be null when `cap`
/// is zero.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_find_text(tab: u32, buf: *mut u8, cap: u32) -> u32 {
    let text = shared().find_text(tab);
    let bytes = text.as_bytes();
    if !buf.is_null() && cap > 0 {
        let n = bytes.len().min(cap as usize - 1);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
    bytes.len() as u32
}

/// Reads the time range the console of a tab is searching within, after a
/// console find event: nanoseconds since the Unix epoch, with zero for no
/// bound on that side.
///
/// # Safety
/// `since_ns` and `until_ns` must each be null or point to writable
/// memory for one value.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_find_range(tab: u32, since_ns: *mut i64, until_ns: *mut i64) {
    let p = shared().find(tab);
    if !since_ns.is_null() {
        *since_ns = p.since_ns;
    }
    if !until_ns.is_null() {
        *until_ns = p.until_ns;
    }
}

/// Moves a console to the lines around something a search found: the
/// line at position `mark` is told apart and the view is held. `chunks`
/// need only hold that line: the console asks for what surrounds it.
/// `QUEVU_FLAG_HISTORY` in `flags` says it was found in the history.
///
/// # Safety
/// As for `quevu_ui_console_deliver`.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_goto(
    tab: u32,
    chunks: *const QuevuUiChunk,
    n: u32,
    mark: u64,
    flags: u32,
) {
    shared().post(Command::Goto {
        tab,
        lines: lines_from(chunks, n),
        mark,
        history: flags & QUEVU_FLAG_HISTORY != 0,
    });
}

/// Tells the user how a step of a search went, in the host's words. A
/// non-zero `resume` is the position the next step should start from if
/// the user asks for the same text again: the step ran out of budget, not
/// out of log. `QUEVU_FLAG_HISTORY` in `flags` offers the history: the
/// step ran out of recent output and did not look further.
///
/// # Safety
/// `status` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_find_status(
    tab: u32,
    status: *const c_char,
    resume: u64,
    flags: u32,
) {
    shared().post(Command::FindStatus {
        tab,
        text: text(status),
        resume,
        history: flags & QUEVU_FLAG_HISTORY != 0,
    });
}

/// Adds a menu entry and returns its identifier, which comes back in a
/// menu event when the user chooses it. `parent` is where it hangs from:
/// `QUEVU_MENU_BAR` for the bar of the window, `QUEVU_MENU_TRAY` for the
/// menu of the icon in the notification area, or the identifier of an
/// entry added with `QUEVU_MENU_SUBMENU`. The window has no menu of its
/// own: a bar with no entries is not shown.
///
/// # Safety
/// `label` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_menu_add(parent: u32, label: *const c_char, attrs: u32) -> u32 {
    let id = shared().new_id();
    shared().post(Command::MenuAdd { id, parent, label: text(label), attrs });
    id
}

/// Changes the label and the attributes of a menu entry. Whether it opens
/// a menu of its own was settled when it was added and does not change.
/// Nothing changes by itself when an entry is chosen: an entry that
/// carries a mark gets it, and loses it, from the host.
///
/// # Safety
/// `label` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_menu_set(item: u32, label: *const c_char, attrs: u32) {
    shared().post(Command::MenuSet { id: item, label: text(label), attrs });
}

/// Removes a menu entry, with whatever hangs from it.
#[no_mangle]
pub extern "C" fn quevu_ui_menu_remove(item: u32) {
    shared().post(Command::MenuRemove { id: item });
}

/// Turns the icon in the notification area on or off. While it is on,
/// minimizing the window hides it there. Its menu is the one the host
/// hangs from `QUEVU_MENU_TRAY`.
#[no_mangle]
pub extern "C" fn quevu_ui_tray(enabled: u32) {
    shared().post(Command::Tray { enabled: enabled != 0 });
}

/// Shows or hides the window.
#[no_mangle]
pub extern "C" fn quevu_ui_show(visible: u32) {
    shared().post(Command::Show { visible: visible != 0 });
}

/// Waits for the next thing the user did. Returns 1 with `*ev` filled,
/// 0 if nothing happened within `timeout_ms` (negative waits without
/// limit), or -1 once the event loop has ended and nothing is left.
///
/// # Safety
/// `ev` must point to writable memory for one event.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_next_event(ev: *mut QuevuUiEvent, timeout_ms: i32) -> i32 {
    match shared().next_event(timeout_ms) {
        Next::Event(e) => {
            if !ev.is_null() {
                *ev = e;
            }
            1
        }
        Next::Timeout => 0,
        Next::Ended => -1,
    }
}
