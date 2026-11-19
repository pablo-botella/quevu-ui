//! Small helpers shared by the window and the console view.

use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWLP_USERDATA};

/// A Rust string as the NUL-terminated UTF-16 the Win32 API wants.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A color as GDI wants it: 0x00BBGGRR.
pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

pub(crate) fn loword(v: usize) -> u32 {
    (v & 0xffff) as u32
}

pub(crate) fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xffff) as u32
}

/// The signed x coordinate packed in the low word of an LPARAM.
pub(crate) fn x_of(l: LPARAM) -> i32 {
    (l & 0xffff) as u16 as i16 as i32
}

/// The signed y coordinate packed in the high word of an LPARAM.
pub(crate) fn y_of(l: LPARAM) -> i32 {
    ((l >> 16) & 0xffff) as u16 as i16 as i32
}

/// The wheel movement packed in the high word of a WPARAM, in notches
/// times 120.
pub(crate) fn wheel_of(w: WPARAM) -> i32 {
    ((w >> 16) & 0xffff) as u16 as i16 as i32
}

/// Scales a length given for 96 dpi to the dpi of a window.
pub(crate) fn scale(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

/// Ties a heap-allocated state to a window. The window owns it from here
/// on and gives it back in [`take_state`].
pub(crate) unsafe fn set_state<T>(hwnd: HWND, state: *mut T) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
}

/// The state tied to a window, if any.
///
/// Win32 hands a window procedure nothing but a handle, so this is how a
/// procedure finds its data: the same job a `void *context` does in C.
/// The reference is only valid on the UI thread, and a caller must not
/// keep it across a call that can run another window procedure.
pub(crate) unsafe fn state<'a, T>(hwnd: HWND) -> Option<&'a mut T> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
    p.as_mut()
}

/// Unties the state from the window and returns ownership of it.
pub(crate) unsafe fn take_state<T>(hwnd: HWND) -> Option<Box<T>> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
    if p.is_null() {
        return None;
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    Some(Box::from_raw(p))
}
