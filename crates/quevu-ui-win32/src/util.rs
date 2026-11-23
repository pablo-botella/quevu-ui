//! Small helpers shared by the window and the console view.

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject, HFONT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowLongPtrW, SendMessageW, SetWindowLongPtrW, BS_PUSHBUTTON, GWLP_USERDATA, HMENU,
    WM_SETFONT, WS_CHILD, WS_DISABLED, WS_TABSTOP, WS_VISIBLE,
};

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

/// The width a text takes in a font, in pixels.
pub(crate) unsafe fn text_width(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    let w: Vec<u16> = text.encode_utf16().collect();
    if w.is_empty() {
        return 0;
    }
    let dc = GetDC(hwnd);
    let old = SelectObject(dc, font);
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(dc, w.as_ptr(), w.len() as i32, &mut size);
    SelectObject(dc, old);
    ReleaseDC(hwnd, dc);
    size.cx
}

/// The width of a push button that says `label`: the words and some air,
/// and never less than a button is expected to be.
pub(crate) unsafe fn button_width(hwnd: HWND, font: HFONT, label: &str, dpi: u32) -> i32 {
    (text_width(hwnd, font, label) + scale(24, dpi)).max(scale(64, dpi))
}

/// A push button as a child of `parent`, with the host's words on it.
pub(crate) unsafe fn button(parent: HWND, hinst: HINSTANCE, font: HFONT, label: &str, disabled: bool, id: u32) -> HWND {
    let class = wide("BUTTON");
    let text = wide(label);
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32 | if disabled { WS_DISABLED } else { 0 };
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        style,
        0,
        0,
        0,
        0,
        parent,
        id as usize as HMENU,
        hinst,
        std::ptr::null(),
    );
    SendMessageW(h, WM_SETFONT, font as usize, 1);
    h
}
