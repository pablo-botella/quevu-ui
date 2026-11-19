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

use std::ffi::{c_char, CStr};
use std::sync::{Arc, OnceLock};

use quevu_ui_core::console::Line;
use quevu_ui_core::shared::{Command, Event, Next, Shared};
use quevu_ui_core::text::display_line;
use quevu_ui_core::UnitState;

/// Version of this interface. A host checks it before anything else.
pub const QUEVU_UI_ABI: u32 = 1;

/// A chunk as the host hands it over. The data is copied before the call
/// returns.
#[repr(C)]
pub struct QuevuUiChunk {
    pub seq: u64,
    pub time_ns: i64,
    pub data: *const u8,
    pub len: u32,
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

#[cfg(windows)]
fn backend_run(title: &str) -> i32 {
    quevu_ui_win32::run(title, shared().clone())
}

#[cfg(not(windows))]
fn backend_run(_title: &str) -> i32 {
    // No backend for this system yet.
    -1
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
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_run(title: *const c_char) -> i32 {
    backend_run(&text(title))
}

/// Ends the event loop: `quevu_ui_run` returns.
#[no_mangle]
pub extern "C" fn quevu_ui_quit() {
    shared().post(Command::Quit);
}

/// Adds a switch for a unit and returns its identifier.
///
/// # Safety
/// `name` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_unit_add(name: *const c_char) -> u32 {
    let id = shared().new_id();
    shared().post(Command::UnitAdd { id, name: text(name) });
    id
}

/// Shows the state of a unit: 0 stopped, 1 starting, 2 running,
/// 3 stopping, 4 failed. `err` is shown next to a failed unit.
///
/// # Safety
/// `err` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_unit_state(unit: u32, state: u32, err: *const c_char) {
    shared().post(Command::UnitState { id: unit, state: UnitState::from_u32(state), err: text(err) });
}

/// Adds a tab with a pseudo-console and returns its identifier. The
/// console asks for its first page at once, with a console request event.
///
/// # Safety
/// `title` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_add(title: *const c_char) -> u32 {
    let id = shared().new_id();
    shared().post(Command::ConsoleAdd { id, title: text(title) });
    id
}

/// Hands a console what came down its data lane: `op` is the operation
/// being answered (1 for live output), `full` marks a pause the source
/// decided by itself.
///
/// # Safety
/// `chunks` must point to `n` valid chunks, each with `len` readable
/// bytes at `data`; it may be null when `n` is zero.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_deliver(
    tab: u32,
    op: u32,
    full: u32,
    chunks: *const QuevuUiChunk,
    n: u32,
) {
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
    shared().post(Command::Deliver { tab, op, full: full != 0, lines });
}

/// Sets the note shown in the status strip of a console, next to its
/// state: the place for the queues of its log.
///
/// # Safety
/// `note` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_console_note(tab: u32, note: *const c_char) {
    shared().post(Command::ConsoleNote { tab, text: text(note) });
}

/// Removes a tab.
#[no_mangle]
pub extern "C" fn quevu_ui_tab_remove(tab: u32) {
    shared().post(Command::TabRemove { id: tab });
}

/// Adds an entry to the application menu and returns its identifier,
/// which comes back in a menu event when the user chooses it.
///
/// # Safety
/// `label` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn quevu_ui_menu_add(label: *const c_char) -> u32 {
    let id = shared().new_id();
    shared().post(Command::MenuAdd { id, label: text(label) });
    id
}

/// Turns the icon in the notification area on or off. While it is on,
/// closing or minimizing the window hides it there instead.
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
