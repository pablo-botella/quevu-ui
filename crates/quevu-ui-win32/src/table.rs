//! The table of a control tab, as a Win32 control of its own.
//!
//! One row per control, in columns that line up: a light, the name, the
//! text of its state and the buttons of its verbs. The table knows nothing
//! of what any of it means. The host says what color the light is, what
//! the text says and which verbs there are; a button pressed goes back to
//! the host as an event and changes nothing here.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;

use quevu_ui_core::attr;
use quevu_ui_core::shared::{event, Event, Shared, Verb};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetSysColor, GetSysColorBrush, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, COLOR_BTNFACE, COLOR_WINDOW, COLOR_WINDOWTEXT,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HFONT,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetScrollInfo, KillTimer,
    LoadCursorW, MoveWindow, RegisterClassExW, SetTimer, CREATESTRUCTW, IDC_ARROW, SB_BOTTOM,
    SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP,
    SB_VERT, SCROLLINFO, SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS, WM_COMMAND, WM_ERASEBKGND,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_TIMER, WM_VSCROLL,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CONTROLPARENT, WS_VISIBLE,
    WS_VSCROLL,
};

use crate::util::{button, button_width, loword, rgb, scale, set_state, state, take_state, text_width, wheel_of, wide};

const CLASS: &str = "QuevuControlTable";

/// How often a light that blinks changes, in milliseconds.
const BLINK_MS: u32 = 500;
const TIMER_BLINK: usize = 1;

// A light that is on and one that is off, in the symbol font of the
// system: a black circle and a white one.
const LIGHT_ON: u16 = 0x25cf;
const LIGHT_OFF: u16 = 0x25cb;

struct Row {
    id: u32,
    name: String,
    /// The color of the light, as the host gives it: 0x00RRGGBB.
    rgb: u32,
    attrs: u32,
    text: String,
    verbs: Vec<Verb>,
    buttons: Vec<HWND>,
}

pub(crate) struct Table {
    hwnd: HWND,
    hinst: HINSTANCE,
    shared: Arc<Shared>,
    font: HFONT,
    /// The font the lights are drawn in. It belongs to the table.
    font_light: HFONT,
    dpi: u32,
    rows: Vec<Row>,
    /// How far down the table is scrolled, in pixels.
    scroll: i32,
    /// The half of the time a blinking light is off.
    dark: bool,
    blinking: bool,
    /// Width of the column of names and of the column of verbs.
    name_w: i32,
    verbs_w: i32,
}

/// What the creator of a table hands over through CreateWindowEx.
struct Seed {
    shared: Arc<Shared>,
    font: HFONT,
    dpi: u32,
}

pub(crate) unsafe fn register(hinst: HINSTANCE) {
    let class = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassExW(&wc);
}

pub(crate) unsafe fn create(parent: HWND, hinst: HINSTANCE, shared: Arc<Shared>, font: HFONT, dpi: u32) -> HWND {
    let class = wide(CLASS);
    let seed = Box::into_raw(Box::new(Seed { shared, font, dpi }));
    CreateWindowExW(
        WS_EX_CONTROLPARENT,
        class.as_ptr(),
        null(),
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0,
        0,
        0,
        0,
        parent,
        null_mut(),
        hinst,
        seed as *const _,
    )
}

/// The table behind a window handle. Only valid on the UI thread.
pub(crate) unsafe fn of<'a>(hwnd: HWND) -> Option<&'a mut Table> {
    state::<Table>(hwnd)
}

impl Table {
    fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }

    fn row_h(&self) -> i32 {
        self.s(34)
    }

    pub(crate) fn has(&self, id: u32) -> bool {
        self.rows.iter().any(|r| r.id == id)
    }

    /// A new row, at the end. It shows its name and nothing else until
    /// the host says what state it is in.
    pub(crate) unsafe fn add(&mut self, id: u32, name: String) {
        self.rows.push(Row {
            id,
            name,
            rgb: 0,
            attrs: attr::LIGHT_HOLLOW,
            text: String::new(),
            verbs: Vec::new(),
            buttons: Vec::new(),
        });
        self.arrange();
    }

    pub(crate) unsafe fn remove(&mut self, id: u32) {
        if let Some(i) = self.rows.iter().position(|r| r.id == id) {
            let row = self.rows.remove(i);
            for b in row.buttons {
                DestroyWindow(b);
            }
            self.arrange();
        }
    }

    /// A control changed. The buttons are made again only when the verbs
    /// are not the ones already there: a control that only changes its
    /// text keeps its buttons, and whichever of them has the keyboard.
    pub(crate) unsafe fn set(&mut self, id: u32, rgb: u32, attrs: u32, text: String, verbs: Vec<Verb>) {
        let Some(i) = self.rows.iter().position(|r| r.id == id) else { return };
        let (hwnd, hinst, font) = (self.hwnd, self.hinst, self.font);
        let row = &mut self.rows[i];
        row.rgb = rgb;
        row.attrs = attrs;
        row.text = text;
        let remade = row.verbs != verbs;
        if remade {
            for b in row.buttons.drain(..) {
                DestroyWindow(b);
            }
            for v in &verbs {
                row.buttons.push(button(hwnd, hinst, font, &v.label, v.attrs & attr::VERB_DISABLED != 0, 0));
            }
            row.verbs = verbs;
        }
        if remade {
            self.arrange();
        } else {
            self.watch_blink();
            InvalidateRect(self.hwnd, null(), 0);
        }
    }

    /// Makes the font of the lights for the scale the table is at.
    unsafe fn make_light_font(&mut self) {
        if !self.font_light.is_null() {
            DeleteObject(self.font_light);
        }
        let face = wide("Segoe UI Symbol");
        const CLEARTYPE_QUALITY: u32 = 5;
        self.font_light = CreateFontW(-self.s(17), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY, 0, face.as_ptr());
    }

    pub(crate) unsafe fn set_font(&mut self, font: HFONT, dpi: u32) {
        self.font = font;
        self.dpi = dpi;
        self.make_light_font();
        for r in &self.rows {
            for &b in &r.buttons {
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                    b,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_SETFONT,
                    font as usize,
                    1,
                );
            }
        }
        self.arrange();
    }

    unsafe fn client(&self) -> (i32, i32) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        (rc.right, rc.bottom)
    }

    /// Keeps the timer of the lights running only while a light blinks.
    unsafe fn watch_blink(&mut self) {
        let any = self.rows.iter().any(|r| r.attrs & attr::LIGHT_BLINK != 0);
        if any && !self.blinking {
            SetTimer(self.hwnd, TIMER_BLINK, BLINK_MS, None);
        } else if !any && self.blinking {
            KillTimer(self.hwnd, TIMER_BLINK);
            self.dark = false;
        }
        self.blinking = any;
    }

    /// Works out the columns, puts every button in its place and brings
    /// the scroll bar up to date.
    unsafe fn arrange(&mut self) {
        self.watch_blink();
        let (w, h) = self.client();
        let pad = self.s(8);
        let gap = self.s(6);
        let bh = self.s(26);
        let row_h = self.row_h();

        // The column of names is as wide as the longest name, within
        // reason; the column of verbs as wide as the longest row of them.
        let mut name_w = 0;
        let mut verbs_w = 0;
        let mut widths: Vec<Vec<i32>> = Vec::with_capacity(self.rows.len());
        for r in &self.rows {
            name_w = name_w.max(text_width(self.hwnd, self.font, &r.name));
            let ws: Vec<i32> = r.verbs.iter().map(|v| button_width(self.hwnd, self.font, &v.label, self.dpi)).collect();
            let total: i32 = ws.iter().sum::<i32>() + gap * (ws.len() as i32 - 1).max(0);
            verbs_w = verbs_w.max(total);
            widths.push(ws);
        }
        self.name_w = name_w.min((w * 2 / 5).max(self.s(60)));
        self.verbs_w = verbs_w;

        let content = self.rows.len() as i32 * row_h;
        self.scroll = self.scroll.clamp(0, (content - h).max(0));
        let si = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: (content - 1).max(0),
            nPage: h.max(0) as u32,
            nPos: self.scroll,
            nTrackPos: 0,
        };
        SetScrollInfo(self.hwnd, SB_VERT, &si, 1);
        // The bar may have come or gone, and taken width with it.
        let (w, _) = self.client();

        let left = (w - pad - self.verbs_w).max(0);
        for (i, r) in self.rows.iter().enumerate() {
            let y = i as i32 * row_h - self.scroll + (row_h - bh) / 2;
            let mut x = left;
            for (b, bw) in r.buttons.iter().zip(&widths[i]) {
                MoveWindow(*b, x, y, *bw, bh, 1);
                x += bw + gap;
            }
        }
        InvalidateRect(self.hwnd, null(), 0);
    }

    unsafe fn scroll_to(&mut self, pos: i32) {
        let (_, h) = self.client();
        let content = self.rows.len() as i32 * self.row_h();
        let pos = pos.clamp(0, (content - h).max(0));
        if pos != self.scroll {
            self.scroll = pos;
            self.arrange();
        }
    }

    unsafe fn on_vscroll(&mut self, code: u32) {
        let (_, h) = self.client();
        let line = self.row_h();
        match code as i32 {
            SB_LINEUP => self.scroll_to(self.scroll - line),
            SB_LINEDOWN => self.scroll_to(self.scroll + line),
            SB_PAGEUP => self.scroll_to(self.scroll - h),
            SB_PAGEDOWN => self.scroll_to(self.scroll + h),
            SB_TOP => self.scroll_to(0),
            SB_BOTTOM => self.scroll_to(i32::MAX),
            SB_THUMBTRACK | SB_THUMBPOSITION => {
                let mut si: SCROLLINFO = zeroed();
                si.cbSize = size_of::<SCROLLINFO>() as u32;
                si.fMask = SIF_TRACKPOS;
                GetScrollInfo(self.hwnd, SB_VERT, &mut si);
                self.scroll_to(si.nTrackPos);
            }
            _ => {}
        }
    }

    unsafe fn paint(&mut self) {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(self.hwnd, &mut ps);
        let (w, h) = self.client();
        let all = RECT { left: 0, top: 0, right: w, bottom: h };

        // Drawn off screen and copied in one go: a light that blinks must
        // not make the rest flicker.
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w.max(1), h.max(1));
        let old_bmp = SelectObject(mem, bmp);
        FillRect(mem, &all, GetSysColorBrush(COLOR_WINDOW));
        let old_font = SelectObject(mem, self.font);
        SetBkMode(mem, TRANSPARENT as _);
        SetTextColor(mem, GetSysColor(COLOR_WINDOWTEXT));

        let pad = self.s(8);
        let d = self.s(12);
        let row_h = self.row_h();
        let name_x = pad * 2 + d + pad;
        let text_x = name_x + self.name_w + pad * 2;
        let text_end = (w - pad - self.verbs_w - pad).max(text_x);
        let flags = DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;

        for (i, r) in self.rows.iter().enumerate() {
            let top = i as i32 * row_h - self.scroll;
            if top + row_h < 0 || top > h {
                continue;
            }
            // The light: a disc of the color the host gave. Hollow, or in
            // the dark half of a blink, only its rim. It is drawn as a
            // character so its edge is as smooth as that of the text.
            let color = rgb((r.rgb >> 16) as u8, (r.rgb >> 8) as u8, r.rgb as u8);
            let off = r.attrs & attr::LIGHT_HOLLOW != 0 || (r.attrs & attr::LIGHT_BLINK != 0 && self.dark);
            let disc = [if off { LIGHT_OFF } else { LIGHT_ON }];
            let mut light_rc = RECT { left: pad * 2 - d / 2, top, right: pad * 2 + d + d / 2, bottom: top + row_h };
            SelectObject(mem, self.font_light);
            SetTextColor(mem, color);
            DrawTextW(mem, disc.as_ptr(), 1, &mut light_rc, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
            SelectObject(mem, self.font);
            SetTextColor(mem, GetSysColor(COLOR_WINDOWTEXT));

            let mut name_rc = RECT { left: name_x, top, right: name_x + self.name_w, bottom: top + row_h };
            let name = wide(&r.name);
            DrawTextW(mem, name.as_ptr(), -1, &mut name_rc, flags);
            let mut text_rc = RECT { left: text_x, top, right: text_end, bottom: top + row_h };
            let text = wide(&r.text);
            DrawTextW(mem, text.as_ptr(), -1, &mut text_rc, flags);

            // A thin line under each row: it is a table.
            let line = RECT { left: pad, top: top + row_h - 1, right: w - pad, bottom: top + row_h };
            FillRect(mem, &line, GetSysColorBrush(COLOR_BTNFACE));
        }

        BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_font);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(self.hwnd, &ps);
    }

    /// A button of a row was pressed: the host hears which verb of which
    /// control. Nothing changes here until the host says so.
    unsafe fn pressed(&self, button: HWND) {
        for r in &self.rows {
            if let Some(i) = r.buttons.iter().position(|&b| b == button) {
                self.shared.emit(Event { kind: event::CONTROL_VERB, id: r.id, op: r.verbs[i].id, n: 0, seq: 0 });
                return;
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        let seed = Box::from_raw(cs.lpCreateParams as *mut Seed);
        let table = Box::new(Table {
            hwnd,
            hinst: cs.hInstance,
            shared: seed.shared.clone(),
            font: seed.font,
            font_light: null_mut(),
            dpi: seed.dpi,
            rows: Vec::new(),
            scroll: 0,
            dark: false,
            blinking: false,
            name_w: 0,
            verbs_w: 0,
        });
        set_state(hwnd, Box::into_raw(table));
        if let Some(t) = of(hwnd) {
            t.make_light_font();
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if msg == WM_NCDESTROY {
        if let Some(t) = take_state::<Table>(hwnd) {
            DeleteObject(t.font_light);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let Some(t) = of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_SIZE => {
            t.arrange();
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            t.paint();
            0
        }
        WM_TIMER => {
            if wparam == TIMER_BLINK {
                t.dark = !t.dark;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_COMMAND => {
            if lparam != 0 {
                t.pressed(lparam as HWND);
            }
            0
        }
        WM_VSCROLL => {
            t.on_vscroll(loword(wparam));
            0
        }
        WM_MOUSEWHEEL => {
            let notches = wheel_of(wparam) as f32 / 120.0;
            t.scroll_to(t.scroll - (notches * t.row_h() as f32) as i32);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
