//! The pseudo-console as a Win32 control of its own.
//!
//! It is deliberately small: a grid of fixed-width cells drawn straight
//! from the model. Everything about following, pausing and moving by
//! position lives in `quevu_ui_core::console`; this file only draws lines
//! and turns mouse, wheel, keys and scroll bars into calls on the model.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;

use quevu_ui_core::console::{Console, Line, Request, Status};
use quevu_ui_core::shared::{Event, Shared};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, ExtTextOutW, FillRect, InvalidateRect, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, HDC, HFONT, OPAQUE, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN, VK_END, VK_HOME,
    VK_NEXT, VK_PRIOR, VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetParent, GetScrollInfo, LoadCursorW,
    PostMessageW, RegisterClassExW, CREATESTRUCTW, CS_DBLCLKS, DLGC_WANTARROWS,
    DLGC_WANTCHARS, IDC_IBEAM, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN,
    SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_DISABLENOSCROLL,
    SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS, WM_ERASEBKGND, WM_GETDLGCODE, WM_HSCROLL,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_VSCROLL, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_HSCROLL, WS_TABSTOP, WS_VSCROLL,
};

use crate::util::{loword, rgb, set_state, state, take_state, wheel_of, wide, x_of, y_of};
use crate::WM_QV_STATE;

const CLASS: &str = "QuevuConsoleView";

/// How many lines the view keeps, and how many it asks for at a time.
const MAX_LINES: usize = 3000;
const PAGE: u32 = 300;

const PAD: i32 = 4;
const CF_UNICODETEXT: u32 = 13;

const BG: u32 = rgb(0x1e, 0x1e, 0x1e);
const FG: u32 = rgb(0xd4, 0xd4, 0xd4);
const NOTE_FG: u32 = rgb(0x8a, 0x8a, 0x8a);
const SEL_BG: u32 = rgb(0x26, 0x4f, 0x78);
const SEL_FG: u32 = rgb(0xff, 0xff, 0xff);

/// A place in the text: a line, by its stable address, and a column.
type Pos = (i64, i32);

pub(crate) struct View {
    hwnd: HWND,
    tab: u32,
    shared: Arc<Shared>,
    model: Console,
    font: HFONT,
    /// Width and height of a cell, in pixels.
    cw: i32,
    ch: i32,
    /// First visible column, and the longest line in sight.
    hpos: i32,
    max_cols: i32,
    /// The selection runs from where the mouse went down to where it is.
    anchor: Option<Pos>,
    caret: Pos,
    dragging: bool,
}

/// What the creator of a view hands over through CreateWindowEx.
struct Seed {
    tab: u32,
    shared: Arc<Shared>,
    font: HFONT,
    cw: i32,
    ch: i32,
}

pub(crate) unsafe fn register(hinst: HINSTANCE) {
    let class = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        hCursor: LoadCursorW(null_mut(), IDC_IBEAM),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassExW(&wc);
}

pub(crate) unsafe fn create(
    parent: HWND,
    hinst: HINSTANCE,
    tab: u32,
    shared: Arc<Shared>,
    font: HFONT,
    cw: i32,
    ch: i32,
) -> HWND {
    let class = wide(CLASS);
    let seed = Box::into_raw(Box::new(Seed { tab, shared, font, cw, ch }));
    CreateWindowExW(
        0,
        class.as_ptr(),
        null(),
        WS_CHILD | WS_VSCROLL | WS_HSCROLL | WS_CLIPSIBLINGS | WS_TABSTOP,
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

/// The view behind a window handle. Only valid on the UI thread.
pub(crate) unsafe fn of<'a>(hwnd: HWND) -> Option<&'a mut View> {
    state::<View>(hwnd)
}

impl View {
    pub(crate) fn status(&self) -> Status {
        self.model.status()
    }

    fn emit(&self, reqs: impl IntoIterator<Item = Request>) {
        for r in reqs {
            self.shared.emit(Event::request(self.tab, r));
        }
    }

    /// Starts the console, or takes it out of a pause: the last page,
    /// then live output.
    pub(crate) unsafe fn follow(&mut self) {
        let r = self.model.follow();
        self.emit([r]);
        self.changed();
    }

    /// Holds the text still.
    pub(crate) unsafe fn pause(&mut self) {
        let r = self.model.pause();
        self.emit(r);
        self.changed();
    }

    /// Something came down the data lane.
    pub(crate) unsafe fn deliver(&mut self, op: u32, full: bool, lines: Vec<Line>) {
        if op == quevu_ui_core::op::FOLLOW {
            // The window starts over: addresses mean something else now.
            self.anchor = None;
            self.hpos = 0;
        }
        self.model.deliver(op, full, lines);
        self.changed();
    }

    pub(crate) unsafe fn set_font(&mut self, font: HFONT, cw: i32, ch: i32) {
        self.font = font;
        self.cw = cw.max(1);
        self.ch = ch.max(1);
        self.resized();
    }

    unsafe fn client(&self) -> (i32, i32) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        (rc.right, rc.bottom)
    }

    unsafe fn cols(&self) -> i32 {
        ((self.client().0 - PAD) / self.cw).max(1)
    }

    unsafe fn resized(&mut self) {
        let (_, h) = self.client();
        self.model.set_rows((h / self.ch).max(1) as usize);
        self.changed();
    }

    /// The model changed: scroll bars, pixels and the status strip of the
    /// window follow.
    unsafe fn changed(&mut self) {
        self.sync_scroll();
        InvalidateRect(self.hwnd, null(), 0);
        // Posted, not sent: the window reads the state when it gets to
        // it, and nothing here runs inside another window procedure.
        PostMessageW(GetParent(self.hwnd), WM_QV_STATE, self.tab as usize, 0);
    }

    unsafe fn sync_scroll(&mut self) {
        let lines = self.model.lines();
        let top = self.model.top();
        let rows = self.model.rows();
        self.max_cols = lines
            .iter()
            .skip(top)
            .take(rows + 1)
            .map(|l| l.text.encode_utf16().count() as i32)
            .max()
            .unwrap_or(0);
        let cols = self.cols();
        self.hpos = self.hpos.clamp(0, (self.max_cols - cols + 1).max(0));

        let mut si = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL,
            nMin: 0,
            nMax: (lines.len() as i32 - 1).max(0),
            nPage: rows as u32,
            nPos: top as i32,
            nTrackPos: 0,
        };
        SetScrollInfo(self.hwnd, SB_VERT, &si, 1);
        si.nMax = self.max_cols.max(0);
        si.nPage = cols as u32;
        si.nPos = self.hpos;
        SetScrollInfo(self.hwnd, SB_HORZ, &si, 1);
    }

    unsafe fn scroll_by(&mut self, delta: isize) {
        let reqs = self.model.scroll_by(delta);
        self.emit(reqs);
        self.changed();
    }

    unsafe fn scroll_to(&mut self, top: usize) {
        let reqs = self.model.scroll_to(top);
        self.emit(reqs);
        self.changed();
    }

    unsafe fn hscroll_to(&mut self, pos: i32) {
        self.hpos = pos.max(0);
        self.changed();
    }

    /// The place in the text under a point of the client area.
    unsafe fn hit(&self, x: i32, y: i32) -> Pos {
        let row = y.div_euclid(self.ch) as i64;
        let line = self.model.origin() + self.model.top() as i64 + row;
        let col = self.hpos + (x - PAD + self.cw / 2).div_euclid(self.cw);
        (line, col.max(0))
    }

    /// The selection with its ends in reading order.
    fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        let b = self.caret;
        if a == b {
            return None;
        }
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The columns of a line that fall inside the selection.
    fn selected_cols(sel: Option<(Pos, Pos)>, addr: i64, len: i32) -> (i32, i32) {
        let Some(((l0, c0), (l1, c1))) = sel else {
            return (0, 0);
        };
        if addr < l0 || addr > l1 {
            return (0, 0);
        }
        let from = if addr == l0 { c0.min(len) } else { 0 };
        let to = if addr == l1 { c1.min(len) } else { len };
        (from, to.max(from))
    }

    unsafe fn select_all(&mut self) {
        let lines = self.model.lines();
        if lines.is_empty() {
            return;
        }
        self.pause();
        let lines = self.model.lines();
        let last = lines.len() as i64 - 1;
        let last_len = lines[last as usize].text.encode_utf16().count() as i32;
        self.anchor = Some((self.model.origin(), 0));
        self.caret = (self.model.origin() + last, last_len);
        self.changed();
    }

    /// Puts the selected text on the clipboard, one line per chunk.
    pub(crate) unsafe fn copy(&self) {
        let Some(sel) = self.selection() else {
            return;
        };
        let origin = self.model.origin();
        let mut out: Vec<u16> = Vec::new();
        for (i, line) in self.model.lines().iter().enumerate() {
            let addr = origin + i as i64;
            if addr < (sel.0).0 || addr > (sel.1).0 {
                continue;
            }
            let text: Vec<u16> = line.text.encode_utf16().collect();
            let (from, to) = Self::selected_cols(Some(sel), addr, text.len() as i32);
            out.extend_from_slice(&text[from as usize..to as usize]);
            if addr < (sel.1).0 {
                out.extend_from_slice(&[13, 10]);
            }
        }
        out.push(0);
        if OpenClipboard(self.hwnd) == 0 {
            return;
        }
        EmptyClipboard();
        let bytes = out.len() * 2;
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if !h.is_null() {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(out.as_ptr(), p, out.len());
                GlobalUnlock(h);
                // The clipboard owns the block from here on.
                SetClipboardData(CF_UNICODETEXT, h as _);
            }
        }
        CloseClipboard();
    }

    unsafe fn draw_run(&self, dc: HDC, y: i32, text: &[u16], from: i32, to: i32, fg: u32, bg: u32) {
        // Only what is in sight is handed to GDI.
        let from = from.max(self.hpos);
        let to = to.min(self.hpos + self.cols() + 1).min(text.len() as i32);
        if to <= from {
            return;
        }
        SetTextColor(dc, fg);
        SetBkColor(dc, bg);
        let x = PAD + (from - self.hpos) * self.cw;
        ExtTextOutW(dc, x, y, 0, null(), text[from as usize..].as_ptr(), (to - from) as u32, null());
    }

    unsafe fn paint(&mut self) {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(self.hwnd, &mut ps);
        let (w, h) = self.client();
        let rc = RECT { left: 0, top: 0, right: w, bottom: h };

        // Drawn off screen and copied in one go, so nothing flickers
        // while output streams in.
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w.max(1), h.max(1));
        let old_bmp = SelectObject(mem, bmp);
        let brush = CreateSolidBrush(BG);
        FillRect(mem, &rc, brush);
        DeleteObject(brush);
        let old_font = SelectObject(mem, self.font);
        SetBkMode(mem, OPAQUE as _);

        let sel = self.selection();
        let origin = self.model.origin();
        let top = self.model.top();
        for (row, line) in self.model.lines().iter().skip(top).take(self.model.rows() + 1).enumerate() {
            let y = row as i32 * self.ch;
            let text: Vec<u16> = line.text.encode_utf16().collect();
            let len = text.len() as i32;
            let base = if line.is_note() { NOTE_FG } else { FG };
            let (from, to) = Self::selected_cols(sel, origin + (top + row) as i64, len);
            // The line is cut wherever the color or the selection changes
            // and each piece is drawn in one go. The selection wins over
            // the color of the text.
            let mut cuts: Vec<i32> = vec![0, len, from, to];
            for s in &line.spans {
                cuts.push(s.start as i32);
                cuts.push((s.start + s.len) as i32);
            }
            cuts.retain(|c| (0..=len).contains(c));
            cuts.sort_unstable();
            cuts.dedup();
            for piece in cuts.windows(2) {
                let (a, b) = (piece[0], piece[1]);
                if from < to && a >= from && b <= to {
                    self.draw_run(mem, y, &text, a, b, SEL_FG, SEL_BG);
                    continue;
                }
                let fg = line
                    .spans
                    .iter()
                    .find(|s| s.start as i32 <= a && a < (s.start + s.len) as i32)
                    .map(|s| rgb((s.rgb >> 16) as u8, (s.rgb >> 8) as u8, s.rgb as u8))
                    .unwrap_or(base);
                self.draw_run(mem, y, &text, a, b, fg, BG);
            }
        }

        BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_font);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(self.hwnd, &ps);
    }

    unsafe fn on_vscroll(&mut self, code: u32) {
        let page = self.model.rows().max(2) as isize - 1;
        match code as i32 {
            SB_LINEUP => self.scroll_by(-1),
            SB_LINEDOWN => self.scroll_by(1),
            SB_PAGEUP => self.scroll_by(-page),
            SB_PAGEDOWN => self.scroll_by(page),
            SB_TOP => self.scroll_to(0),
            SB_BOTTOM => self.scroll_to(usize::MAX),
            SB_THUMBTRACK | SB_THUMBPOSITION => {
                let mut si: SCROLLINFO = zeroed();
                si.cbSize = size_of::<SCROLLINFO>() as u32;
                si.fMask = SIF_TRACKPOS;
                GetScrollInfo(self.hwnd, SB_VERT, &mut si);
                self.scroll_to(si.nTrackPos.max(0) as usize);
            }
            _ => {}
        }
    }

    unsafe fn on_hscroll(&mut self, code: u32) {
        let page = (self.cols() - 1).max(1);
        match code as i32 {
            SB_LINEUP => self.hscroll_to(self.hpos - 1),
            SB_LINEDOWN => self.hscroll_to(self.hpos + 1),
            SB_PAGEUP => self.hscroll_to(self.hpos - page),
            SB_PAGEDOWN => self.hscroll_to(self.hpos + page),
            SB_THUMBTRACK | SB_THUMBPOSITION => {
                let mut si: SCROLLINFO = zeroed();
                si.cbSize = size_of::<SCROLLINFO>() as u32;
                si.fMask = SIF_TRACKPOS;
                GetScrollInfo(self.hwnd, SB_HORZ, &mut si);
                self.hscroll_to(si.nTrackPos);
            }
            _ => {}
        }
    }

    unsafe fn on_key(&mut self, vk: u32) -> bool {
        let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
        let page = self.model.rows().max(2) as isize - 1;
        match vk as u16 {
            VK_UP => self.scroll_by(-1),
            VK_DOWN => self.scroll_by(1),
            VK_PRIOR => self.scroll_by(-page),
            VK_NEXT => self.scroll_by(page),
            VK_HOME => self.scroll_to(0),
            // End is "take me back to the live output".
            VK_END => self.follow(),
            k if ctrl && k == b'C' as u16 => self.copy(),
            k if ctrl && k == b'A' as u16 => self.select_all(),
            _ => return false,
        }
        true
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        let seed = Box::from_raw(cs.lpCreateParams as *mut Seed);
        let view = Box::new(View {
            hwnd,
            tab: seed.tab,
            shared: seed.shared.clone(),
            model: Console::new(MAX_LINES, PAGE),
            font: seed.font,
            cw: seed.cw.max(1),
            ch: seed.ch.max(1),
            hpos: 0,
            max_cols: 0,
            anchor: None,
            caret: (0, 0),
            dragging: false,
        });
        set_state(hwnd, Box::into_raw(view));
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if msg == WM_NCDESTROY {
        drop(take_state::<View>(hwnd));
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let Some(v) = of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_SIZE => {
            v.resized();
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            v.paint();
            0
        }
        WM_GETDLGCODE => (DLGC_WANTARROWS | DLGC_WANTCHARS) as LRESULT,
        WM_MOUSEWHEEL => {
            let notches = wheel_of(wparam) as f32 / 120.0;
            if GetKeyState(VK_SHIFT as i32) < 0 {
                v.hscroll_to(v.hpos - (notches * 8.0) as i32);
            } else {
                // Wheel up is positive, and means older output.
                v.scroll_by(-(notches * 3.0).round() as isize);
            }
            0
        }
        WM_VSCROLL => {
            v.on_vscroll(loword(wparam));
            0
        }
        WM_HSCROLL => {
            v.on_hscroll(loword(wparam));
            0
        }
        WM_KEYDOWN => {
            if v.on_key(wparam as u32) {
                0
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            SetCapture(hwnd);
            // A selection needs the text to hold still.
            v.pause();
            let p = v.hit(x_of(lparam), y_of(lparam));
            v.anchor = Some(p);
            v.caret = p;
            v.dragging = true;
            v.changed();
            0
        }
        WM_MOUSEMOVE => {
            if v.dragging {
                let (x, y) = (x_of(lparam), y_of(lparam));
                let (_, h) = v.client();
                // Dragging past an edge walks the text along.
                if y < 0 {
                    v.scroll_by(-1);
                } else if y >= h {
                    v.scroll_by(1);
                }
                v.caret = v.hit(x, y.clamp(0, (h - 1).max(0)));
                v.changed();
            }
            0
        }
        WM_LBUTTONUP => {
            if v.dragging {
                v.dragging = false;
                ReleaseCapture();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
