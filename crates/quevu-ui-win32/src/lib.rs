//! Win32 backend of the quevu visual part.
//!
//! One window: the switches on the left (a global one and one per unit),
//! the screens on the right (a tab per log, each holding a pseudo-console)
//! and a status strip under them. Plus a menu and, if asked for, an icon
//! in the notification area to hide in.
//!
//! Everything here runs on the thread that called [`run`]. The host
//! reaches it only through `quevu_ui_core::shared`: commands arrive as a
//! posted message that makes this thread drain the queue, and what the
//! user does leaves as events.
#![cfg(windows)]

mod util;
mod view;

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;

use quevu_ui_core::console::Status;
use quevu_ui_core::shared::{event, Command, Event, Shared};
use quevu_ui_core::UnitState;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateFontW, DeleteObject, GetDC, GetSysColorBrush, GetTextMetricsW,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, COLOR_BTNFACE, COLOR_WINDOW, HFONT, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
    ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCIF_TEXT, TCITEMW,
    TCM_ADJUSTRECT, TCM_DELETEITEM, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCM_SETITEMW,
    TCN_SELCHANGE,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForWindow, SetThreadDpiAwarenessContext, SystemParametersInfoForDpi,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_F3, VK_SHIFT,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, DrawMenuBar, GetClientRect, GetCursorPos, GetMessageW, GetWindowTextW,
    IsDialogMessageW, IsWindowVisible, LoadCursorW, LoadIconW, MoveWindow, PostMessageW,
    PostQuitMessage, RegisterClassExW, SendMessageW, SetForegroundWindow, SetMenu, SetWindowPos,
    SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage, BS_PUSHBUTTON,
    CW_USEDEFAULT, HMENU, HWND_BOTTOM, HWND_TOP, IDC_ARROW, IDI_APPLICATION, MF_CHECKED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED,
    MSG, NONCLIENTMETRICSW, SIZE_MINIMIZED, SPI_GETNONCLIENTMETRICS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW,
    TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_NCDESTROY, WM_NOTIFY, WM_NULL, WM_RBUTTONUP, WM_SETFONT, WM_SIZE,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use util::{hiword, loword, scale, set_state, state, take_state, wide};

/// The host queued commands: drain them.
const WM_QV_WAKE: u32 = WM_APP + 1;
/// The console of tab `wparam` changed state: refresh the status strip.
pub(crate) const WM_QV_STATE: u32 = WM_APP + 2;
/// Something happened to the icon in the notification area.
const WM_QV_TRAY: u32 = WM_APP + 3;


const CLASS: &str = "QuevuPanelWindow";
/// The plain background of the page of switches: a window that does
/// nothing but paint itself in the color the labels on it are drawn with.
const PAGE_CLASS: &str = "QuevuPanelPage";

// Pause and play in the symbol font of the system (Segoe MDL2 Assets).
const GLYPH_PAUSE: &str = "\u{e769}";
const GLYPH_PLAY: &str = "\u{e768}";
// Chevrons, for walking through the matches of a search.
const GLYPH_UP: &str = "\u{e70e}";
const GLYPH_DOWN: &str = "\u{e70d}";

// Styles of the STATIC control.
const SS_NOPREFIX: u32 = 0x80;
const SS_ENDELLIPSIS: u32 = 0x4000;

// Command identifiers. Buttons and menu entries that do the same thing
// share one.
const ID_START_ALL: u32 = 101;
const ID_STOP_ALL: u32 = 102;
const ID_EXIT: u32 = 103;
const ID_TOGGLE: u32 = 110;
const ID_COPY: u32 = 111;
const ID_OPEN: u32 = 112;
const ID_TIME: u32 = 113;
const ID_FIND: u32 = 114;
const ID_FIND_OLDER: u32 = 115;
const ID_FIND_NEWER: u32 = 116;
const ID_HISTORY: u32 = 117;
// What a dialog-style window gets for Enter and Escape.
const ID_ENTER: u32 = 1;
const ID_ESCAPE: u32 = 2;

// Styles and messages of the EDIT control.
const ES_AUTOHSCROLL: u32 = 0x80;
const EM_SETSEL: u32 = 0xb1;
const EM_SETCUEBANNER: u32 = 0x1501;
const ID_UNIT_BASE: u32 = 1000;
const ID_MENU_BASE: u32 = 5000;

struct Unit {
    id: u32,
    name: String,
    state: UnitState,
    err: String,
    button: HWND,
    label: HWND,
}

struct Tab {
    id: u32,
    view: HWND,
    title: String,
    note: String,
    /// Output arrived while another tab was in front.
    unseen: bool,
    /// How the last step of a search went, shown in the strip.
    find_status: String,
    /// Where the next step starts if the same thing is asked again, and
    /// what "the same thing" is.
    find_resume: u64,
    last_query: String,
    last_newer: bool,
    /// A search ran out of recent output and the history was offered.
    find_offer: bool,
    /// The user allowed searches of this console into the history.
    find_history: bool,
    /// Whether the console was following the last time it was looked at.
    was_following: bool,
}

struct App {
    shared: Arc<Shared>,
    hwnd: HWND,
    hinst: HINSTANCE,
    title: String,
    dpi: u32,
    font_ui: HFONT,
    font_mono: HFONT,
    font_icon: HFONT,
    cell: (i32, i32),
    start_all: HWND,
    stop_all: HWND,
    tabs_ctl: HWND,
    page: HWND,
    strip: HWND,
    toggle: HWND,
    find_edit: HWND,
    find_older: HWND,
    find_newer: HWND,
    history: HWND,
    units: Vec<Unit>,
    tabs: Vec<Tab>,
    app_menu: HMENU,
    console_menu: HMENU,
    show_time: bool,
    menu_items: Vec<u32>,
    tray: bool,
    tray_shown: bool,
}

/// Runs the visual part on the calling thread until the host posts
/// `Command::Quit`. Returns 0 on a clean end.
pub fn run(title: &str, shared: Arc<Shared>) -> i32 {
    unsafe { run_inner(title, shared) }
}

unsafe fn run_inner(title: &str, shared: Arc<Shared>) -> i32 {
    // A library must not decide for the whole process how it looks, so
    // both settings below are scoped: the DPI awareness to this thread,
    // and the modern look of the controls to an activation context that
    // is active only while this function runs.
    SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let themed = Themed::enter();

    let icc = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TAB_CLASSES,
    };
    InitCommonControlsEx(&icc);

    let hinst = GetModuleHandleW(null());
    let class = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassExW(&wc);
    let page_class = wide(PAGE_CLASS);
    let page = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(DefWindowProcW),
        hInstance: hinst,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_WINDOW),
        lpszClassName: page_class.as_ptr(),
        ..zeroed()
    };
    RegisterClassExW(&page);
    view::register(hinst);

    let wtitle = wide(title);
    let hwnd = CreateWindowExW(
        WS_EX_CONTROLPARENT,
        class.as_ptr(),
        wtitle.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        null_mut(),
        null_mut(),
        hinst,
        null(),
    );
    if hwnd.is_null() {
        drop(themed);
        return 1;
    }

    let app = Box::new(App {
        shared: shared.clone(),
        hwnd,
        hinst,
        title: title.to_string(),
        dpi: GetDpiForWindow(hwnd).max(96),
        font_ui: null_mut(),
        font_mono: null_mut(),
        font_icon: null_mut(),
        cell: (8, 16),
        start_all: null_mut(),
        stop_all: null_mut(),
        tabs_ctl: null_mut(),
        page: null_mut(),
        strip: null_mut(),
        toggle: null_mut(),
        find_edit: null_mut(),
        find_older: null_mut(),
        find_newer: null_mut(),
        history: null_mut(),
        units: Vec::new(),
        tabs: Vec::new(),
        app_menu: null_mut(),
        console_menu: null_mut(),
        show_time: false,
        menu_items: Vec::new(),
        tray: false,
        tray_shown: false,
    });
    set_state(hwnd, Box::into_raw(app));
    if let Some(app) = state::<App>(hwnd) {
        app.build();
    }
    // A sensible first size, in the units of this monitor.
    let dpi = GetDpiForWindow(hwnd).max(96);
    SetWindowPos(hwnd, null_mut(), 0, 0, scale(1040, dpi), scale(660, dpi), SWP_NOMOVE | SWP_NOZORDER);
    ShowWindow(hwnd, SW_SHOW);

    // From here on the host can reach this thread. A handle is just a
    // number as far as posting a message goes, which is what lets the
    // waker cross threads.
    let target = hwnd as usize;
    shared.attach(Box::new(move || unsafe {
        PostMessageW(target as HWND, WM_QV_WAKE, 0, 0);
    }));
    shared.emit(Event::simple(event::READY, 0));
    // Whatever the host queued before the window existed.
    PostMessageW(hwnd, WM_QV_WAKE, 0, 0);

    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        // The keys of the search work wherever the keyboard is: in the
        // console, in the search box, on a tab. They are taken here, on
        // the way in, so no control has to know about them.
        if msg.message == WM_KEYDOWN {
            let shift = GetKeyState(VK_SHIFT as i32) < 0;
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            let id = match msg.wParam as u16 {
                VK_F3 if shift => Some(ID_FIND_NEWER),
                VK_F3 => Some(ID_FIND_OLDER),
                k if ctrl && k == b'F' as u16 => Some(ID_FIND),
                _ => None,
            };
            if let Some(id) = id {
                PostMessageW(hwnd, WM_COMMAND, id as usize, 0);
                continue;
            }
        }
        // Lets Tab walk through the switches like in a dialog.
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    shared.detach();
    drop(themed);
    0
}

/// The modern look of the common controls (version 6 of comctl32) is
/// normally asked for in the manifest of the executable. A library cannot
/// count on its host having one, so it borrows the manifest that ships
/// inside shell32.dll for exactly this purpose and activates it for the
/// current thread only.
struct Themed {
    ctx: windows_sys::Win32::Foundation::HANDLE,
    cookie: usize,
}

impl Themed {
    unsafe fn enter() -> Option<Themed> {
        let mut path = [0u16; 300];
        let n = GetSystemDirectoryW(path.as_mut_ptr(), 260) as usize;
        if n == 0 || n >= 260 {
            return None;
        }
        for (i, c) in "\\shell32.dll".encode_utf16().enumerate() {
            path[n + i] = c;
        }
        const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;
        let actctx = ACTCTXW {
            cbSize: size_of::<ACTCTXW>() as u32,
            dwFlags: ACTCTX_FLAG_RESOURCE_NAME_VALID,
            lpSource: path.as_ptr(),
            // MAKEINTRESOURCE(124)
            lpResourceName: 124usize as *const u16,
            ..zeroed()
        };
        let ctx = CreateActCtxW(&actctx);
        if ctx as isize == -1 || ctx.is_null() {
            return None;
        }
        let mut cookie = 0usize;
        if ActivateActCtx(ctx, &mut cookie) == 0 {
            ReleaseActCtx(ctx);
            return None;
        }
        Some(Themed { ctx, cookie })
    }
}

impl Drop for Themed {
    fn drop(&mut self) {
        unsafe {
            DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.ctx);
        }
    }
}

impl App {
    fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }

    unsafe fn child(&self, class: &str, text: &str, style: u32, id: u32) -> HWND {
        let class = wide(class);
        let text = wide(text);
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as usize as HMENU,
            self.hinst,
            null(),
        );
        SendMessageW(h, WM_SETFONT, self.font_ui as usize, 1);
        h
    }

    unsafe fn make_fonts(&mut self) {
        if !self.font_ui.is_null() {
            DeleteObject(self.font_ui);
        }
        if !self.font_mono.is_null() {
            DeleteObject(self.font_mono);
        }
        // The font the system uses for messages, at this monitor's size.
        let mut ncm: NONCLIENTMETRICSW = zeroed();
        ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            &mut ncm as *mut _ as *mut _,
            0,
            self.dpi,
        );
        self.font_ui = CreateFontIndirectW(&ncm.lfMessageFont);

        let face = wide("Consolas");
        // 10 points.
        let height = -(10 * self.dpi as i32 / 72);
        const FIXED_PITCH_MODERN: u32 = 1 | 0x30;
        const CLEARTYPE_QUALITY: u32 = 5;
        self.font_mono = CreateFontW(
            height, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY, FIXED_PITCH_MODERN, face.as_ptr(),
        );
        // The symbol font of the system, for the transport button of a
        // console: the pause and play marks of a tape deck.
        if !self.font_icon.is_null() {
            DeleteObject(self.font_icon);
        }
        let icons = wide("Segoe MDL2 Assets");
        self.font_icon = CreateFontW(
            -(10 * self.dpi as i32 / 72), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY, 0, icons.as_ptr(),
        );

        let dc = GetDC(self.hwnd);
        let old = SelectObject(dc, self.font_mono);
        let mut tm: TEXTMETRICW = zeroed();
        GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        ReleaseDC(self.hwnd, dc);
        self.cell = (tm.tmAveCharWidth.max(1), (tm.tmHeight + tm.tmExternalLeading).max(1));
    }

    unsafe fn build(&mut self) {
        self.make_fonts();

        let bar = CreateMenu();
        let panel = CreatePopupMenu();
        AppendMenuW(panel, MF_STRING, ID_START_ALL as usize, wide("&Start all").as_ptr());
        AppendMenuW(panel, MF_STRING, ID_STOP_ALL as usize, wide("S&top all").as_ptr());
        AppendMenuW(panel, MF_SEPARATOR, 0, null());
        AppendMenuW(panel, MF_STRING, ID_EXIT as usize, wide("E&xit").as_ptr());
        AppendMenuW(bar, MF_POPUP, panel as usize, wide("&Panel").as_ptr());
        let console = CreatePopupMenu();
        AppendMenuW(console, MF_STRING, ID_TOGGLE as usize, wide("&Hold / follow the output\tEnd").as_ptr());
        AppendMenuW(console, MF_STRING, ID_COPY as usize, wide("&Copy\tCtrl+C").as_ptr());
        AppendMenuW(console, MF_SEPARATOR, 0, null());
        AppendMenuW(console, MF_STRING, ID_FIND as usize, wide("&Find...\tCtrl+F").as_ptr());
        AppendMenuW(console, MF_STRING, ID_FIND_OLDER as usize, wide("Find &older\tF3").as_ptr());
        AppendMenuW(console, MF_STRING, ID_FIND_NEWER as usize, wide("Find &newer\tShift+F3").as_ptr());
        AppendMenuW(console, MF_SEPARATOR, 0, null());
        AppendMenuW(console, MF_STRING, ID_TIME as usize, wide("Show the &time of each line").as_ptr());
        self.console_menu = console;
        AppendMenuW(bar, MF_POPUP, console as usize, wide("&Console").as_ptr());
        SetMenu(self.hwnd, bar);

        // The tab control, the plain background of the page of switches,
        // and the switches themselves. Which one covers which is pinned
        // further down, not left to the order of creation.
        self.tabs_ctl = self.child("SysTabControl32", "", WS_CLIPSIBLINGS | WS_TABSTOP, 0);
        self.page = self.child(PAGE_CLASS, "", WS_CLIPSIBLINGS, 0);
        self.start_all = self.child("BUTTON", "Start all", BS_PUSHBUTTON as u32 | WS_TABSTOP, ID_START_ALL);
        self.stop_all = self.child("BUTTON", "Stop all", BS_PUSHBUTTON as u32 | WS_TABSTOP, ID_STOP_ALL);
        // The first tab is always there: the page of switches.
        let mut text = wide("Units");
        let mut item: TCITEMW = zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = text.as_mut_ptr();
        SendMessageW(self.tabs_ctl, TCM_INSERTITEMW, 0, &item as *const TCITEMW as LPARAM);
        SendMessageW(self.tabs_ctl, TCM_SETCURSEL, 0, 0);
        self.strip = self.child("STATIC", "", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
        self.toggle = self.child("BUTTON", GLYPH_PAUSE, BS_PUSHBUTTON as u32 | WS_TABSTOP | WS_CLIPSIBLINGS, ID_TOGGLE);
        SendMessageW(self.toggle, WM_SETFONT, self.font_icon as usize, 1);
        EnableWindow(self.toggle, 0);
        // The box a search is typed in. It needs a border of its own,
        // hence not through child().
        let edit_class = wide("EDIT");
        self.find_edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            edit_class.as_ptr(),
            null(),
            WS_CHILD | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            0,
            0,
            self.hwnd,
            null_mut(),
            self.hinst,
            null(),
        );
        SendMessageW(self.find_edit, WM_SETFONT, self.font_ui as usize, 1);
        let cue = wide("Find in this log");
        SendMessageW(self.find_edit, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
        // The same two steps the keys take, for the mouse: up is older,
        // down is newer, as the text itself runs.
        self.find_older = self.child("BUTTON", GLYPH_UP, BS_PUSHBUTTON as u32, ID_FIND_OLDER);
        self.find_newer = self.child("BUTTON", GLYPH_DOWN, BS_PUSHBUTTON as u32, ID_FIND_NEWER);
        for h in [self.find_older, self.find_newer] {
            SendMessageW(h, WM_SETFONT, self.font_icon as usize, 1);
            ShowWindow(h, SW_HIDE);
        }
        // The one way into the history. It shows up only when there is a
        // history to offer, and nothing goes there until it is pressed.
        self.history = self.child("BUTTON", "History", BS_PUSHBUTTON as u32, ID_HISTORY);
        ShowWindow(self.history, SW_HIDE);
        // Pin the stacking order instead of trusting the order of
        // creation: the tab control at the very bottom, the background of
        // the page right above it, everything else over both.
        let keep = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        SetWindowPos(self.page, HWND_BOTTOM, 0, 0, 0, 0, keep);
        SetWindowPos(self.tabs_ctl, HWND_BOTTOM, 0, 0, 0, 0, keep);
        self.layout();
    }

    unsafe fn layout(&mut self) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let pad = self.s(8);
        let bh = self.s(28);

        // One row of tabs across the whole window: the switches are the
        // first tab, the screens are the rest.
        let strip_h = self.s(36);
        let tx = pad;
        let tw = (w - pad * 2).max(0);
        let th = (h - strip_h - pad).max(0);
        MoveWindow(self.tabs_ctl, tx, pad, tw, th, 1);
        let mut area = RECT { left: 0, top: 0, right: tw, bottom: th };
        // Asks the tab control how much of itself is left for content.
        SendMessageW(self.tabs_ctl, TCM_ADJUSTRECT, 0, &mut area as *mut RECT as LPARAM);
        for t in &self.tabs {
            MoveWindow(
                t.view,
                tx + area.left,
                pad + area.top,
                (area.right - area.left).max(0),
                (area.bottom - area.top).max(0),
                1,
            );
        }

        // The page of switches, laid out inside the first tab: the global
        // one on top, then a row per unit with room for its whole state.
        MoveWindow(
            self.page,
            tx + area.left,
            pad + area.top,
            (area.right - area.left).max(0),
            (area.bottom - area.top).max(0),
            1,
        );
        let (ox, oy) = (tx + area.left + pad * 2, pad + area.top + pad * 2);
        let gw = self.s(96);
        MoveWindow(self.start_all, ox, oy, gw, bh, 1);
        MoveWindow(self.stop_all, ox + gw + pad, oy, gw, bh, 1);
        let uw = self.s(72);
        let lw = (area.right - area.left - pad * 5 - uw).max(0);
        let mut y = oy + bh + pad * 2;
        for u in &self.units {
            MoveWindow(u.button, ox, y, uw, self.s(26), 1);
            MoveWindow(u.label, ox + uw + pad, y + self.s(5), lw, self.s(18), 1);
            y += self.s(34);
        }

        // The transport button sits in the row of the tabs, at its right
        // end: on the consoles and away from the switches, because it
        // holds the view, not the unit.
        let side = (area.top - self.s(4)).max(self.s(18));
        MoveWindow(self.toggle, tx + tw - side - self.s(2), pad, side, side, 1);
        SetWindowPos(self.toggle, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);

        let sy = pad + th + (strip_h - bh) / 2;
        let fb = self.s(26);
        let fw = self.s(260).min(tw / 3);
        let right = tx + tw;
        MoveWindow(self.find_newer, right - fb, sy + self.s(1), fb, fb, 1);
        MoveWindow(self.find_older, right - fb * 2 - self.s(2), sy + self.s(1), fb, fb, 1);
        let hw = self.s(76);
        let used = fw + fb * 2 + self.s(8);
        MoveWindow(self.find_edit, right - used, sy + self.s(2), fw, self.s(24), 1);
        MoveWindow(self.history, right - used - hw - pad, sy + self.s(1), hw, fb, 1);
        let taken = used + hw + pad * 3;
        MoveWindow(self.strip, tx + pad / 2, sy + self.s(5), (tw - taken).max(0), self.s(18), 1);
        let _ = w;
    }

    /// The console of the selected tab, if the selected tab is one. Tab 0
    /// is the page of switches, so console `i` lives in tab `i + 1`.
    fn current(&self) -> Option<usize> {
        let i = unsafe { SendMessageW(self.tabs_ctl, TCM_GETCURSEL, 0, 0) };
        (i >= 1 && (i as usize) <= self.tabs.len()).then(|| i as usize - 1)
    }

    /// Shows what the selected tab holds, the switches or one console,
    /// and hides everything else.
    unsafe fn show_current(&mut self) {
        let cur = self.current();
        let switches = if cur.is_none() { SW_SHOW } else { SW_HIDE };
        ShowWindow(self.page, switches);
        ShowWindow(self.start_all, switches);
        ShowWindow(self.stop_all, switches);
        for u in &self.units {
            ShowWindow(u.button, switches);
            ShowWindow(u.label, switches);
        }
        // The transport button belongs to the consoles.
        ShowWindow(self.toggle, if cur.is_some() { SW_SHOW } else { SW_HIDE });
        for h in [self.find_edit, self.find_older, self.find_newer] {
            ShowWindow(h, if cur.is_some() { SW_SHOW } else { SW_HIDE });
        }
        if let Some(i) = cur {
            if self.tabs[i].unseen {
                self.tabs[i].unseen = false;
                self.retitle(i);
            }
        }
        for (i, t) in self.tabs.iter().enumerate() {
            if Some(i) == cur {
                SetWindowPos(t.view, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                ShowWindow(t.view, SW_SHOW);
            } else {
                ShowWindow(t.view, SW_HIDE);
            }
        }
        self.refresh_strip();
    }

    /// Writes the title of a console tab, with a mark if it holds output
    /// nobody has looked at.
    unsafe fn retitle(&self, i: usize) {
        let t = &self.tabs[i];
        let label = if t.unseen { format!("{} \u{2022}", t.title) } else { t.title.clone() };
        let mut text = wide(&label);
        let mut item: TCITEMW = zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = text.as_mut_ptr();
        SendMessageW(self.tabs_ctl, TCM_SETITEMW, i + 1, &item as *const TCITEMW as LPARAM);
    }

    unsafe fn refresh_strip(&mut self) {
        let Some(i) = self.current() else {
            ShowWindow(self.history, SW_HIDE);
            // On the page of switches the strip sums the units up.
            let mut counts = [0usize; 5];
            for u in &self.units {
                counts[u.state as usize] += 1;
            }
            let mut parts = Vec::new();
            for state in [
                UnitState::Running,
                UnitState::Starting,
                UnitState::Stopping,
                UnitState::Stopped,
                UnitState::Failed,
            ] {
                let n = counts[state as usize];
                if n > 0 {
                    parts.push(format!("{n} {}", state.label()));
                }
            }
            SetWindowTextW(self.strip, wide(&parts.join(", ")).as_ptr());
            EnableWindow(self.toggle, 0);
            return;
        };
        let status = view::of(self.tabs[i].view).map(|v| v.status()).unwrap_or(Status::Waiting);
        let following = status == Status::Following;
        if following && !self.tabs[i].was_following {
            // Back on the live output: the search is over, and so is any
            // leave to look into the history. Only on the way back: a
            // search made while following has things to say too.
            self.tabs[i].find_status.clear();
            self.tabs[i].find_resume = 0;
            self.tabs[i].find_offer = false;
            self.tabs[i].find_history = false;
        }
        self.tabs[i].was_following = following;
        let scroll_offer = view::of(self.tabs[i].view).map(|v| v.history_offered()).unwrap_or(false);
        let offer = scroll_offer || self.tabs[i].find_offer;
        ShowWindow(self.history, if offer { SW_SHOW } else { SW_HIDE });
        let tab = &self.tabs[i];
        // The button shows what pressing it does, as on a tape deck. The
        // text says in words that only the view is held: the application
        // and its log go on.
        let (text, button) = match status {
            Status::Waiting => ("Waiting for output...", GLYPH_PAUSE),
            Status::Following => ("Following the live output", GLYPH_PAUSE),
            Status::Paused => ("View held (the application keeps running). Press play or End to follow the live output", GLYPH_PLAY),
            Status::PausedFull => ("View held: it fell behind and stopped receiving (the application keeps running). Play jumps to the end", GLYPH_PLAY),
        };
        // While a search is going on, the strip tells how it went.
        let mut line = if !tab.find_status.is_empty() {
            tab.find_status.clone()
        } else if scroll_offer {
            "This is the oldest recent output. There is a history: press History to load it".to_string()
        } else {
            text.to_string()
        };
        if tab.find_status.is_empty() && !tab.note.is_empty() {
            line.push_str("   |   ");
            line.push_str(&tab.note);
        }
        SetWindowTextW(self.strip, wide(&line).as_ptr());
        SetWindowTextW(self.toggle, wide(button).as_ptr());
        EnableWindow(self.toggle, 1);
    }

    unsafe fn refresh_unit(&self, u: &Unit) {
        let mut text = format!("{}  -  {}", u.name, u.state.label());
        if !u.err.is_empty() {
            text.push_str(": ");
            text.push_str(&u.err);
        }
        SetWindowTextW(u.label, wide(&text).as_ptr());
        let action = if u.state.offers_stop() { "Stop" } else { "Start" };
        SetWindowTextW(u.button, wide(action).as_ptr());
        EnableWindow(u.button, if u.state.is_busy() { 0 } else { 1 });
    }

    unsafe fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::UnitAdd { id, name } => {
                let cmd_id = ID_UNIT_BASE + self.units.len() as u32;
                let button = self.child("BUTTON", "Start", BS_PUSHBUTTON as u32 | WS_TABSTOP, cmd_id);
                let label = self.child("STATIC", "", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
                let u = Unit { id, name, state: UnitState::Stopped, err: String::new(), button, label };
                self.refresh_unit(&u);
                self.units.push(u);
                self.layout();
                self.show_current();
            }
            Command::UnitState { id, state, err } => {
                if let Some(i) = self.units.iter().position(|u| u.id == id) {
                    self.units[i].state = state;
                    self.units[i].err = err;
                    self.refresh_unit(&self.units[i]);
                    if self.current().is_none() {
                        self.refresh_strip();
                    }
                }
            }
            Command::ConsoleAdd { id, title } => {
                let view = view::create(
                    self.hwnd,
                    self.hinst,
                    id,
                    self.shared.clone(),
                    self.font_mono,
                    self.cell.0,
                    self.cell.1,
                );
                let mut text = wide(&title);
                let mut item: TCITEMW = zeroed();
                item.mask = TCIF_TEXT;
                item.pszText = text.as_mut_ptr();
                // After the page of switches and the consoles already there.
                let at = self.tabs.len() + 1;
                SendMessageW(self.tabs_ctl, TCM_INSERTITEMW, at, &item as *const TCITEMW as LPARAM);
                self.tabs.push(Tab { id, view, title,
                    note: String::new(),
                    unseen: false,
                    find_status: String::new(),
                    find_resume: 0,
                    last_query: String::new(),
                    last_newer: false,
                    find_offer: false,
                    find_history: false,
                    was_following: false,
                });
                if self.show_time {
                    if let Some(v) = view::of(view) {
                        v.set_show_time(true);
                    }
                }
                self.layout();
                self.show_current();
                // The console asks for its first page as soon as it exists.
                if let Some(v) = view::of(view) {
                    v.follow();
                }
            }
            Command::Deliver { tab, op, full, history, lines } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == tab) {
                    // New output on a tab that is not in front leaves a
                    // mark on its title.
                    let fresh = op == quevu_ui_core::op::LIVE && !lines.is_empty();
                    if let Some(v) = view::of(self.tabs[i].view) {
                        v.deliver(op, full, history, lines);
                    }
                    if fresh && self.current() != Some(i) && !self.tabs[i].unseen {
                        self.tabs[i].unseen = true;
                        self.retitle(i);
                    }
                }
            }
            Command::ConsoleNote { tab, text } => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.note = text;
                }
                self.refresh_strip();
            }
            Command::Goto { tab, lines, mark, history } => {
                if let Some(t) = self.tabs.iter().find(|t| t.id == tab) {
                    if let Some(v) = view::of(t.view) {
                        v.goto(lines, mark, history);
                    }
                }
            }
            Command::FindStatus { tab, text, resume, history } => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    t.find_status = text;
                    t.find_resume = resume;
                    t.find_offer = history;
                }
                self.refresh_strip();
            }
            Command::TabRemove { id } => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    let t = self.tabs.remove(i);
                    DestroyWindow(t.view);
                    SendMessageW(self.tabs_ctl, TCM_DELETEITEM, i + 1, 0);
                    // Falls back on the neighbour, or on the switches.
                    SendMessageW(self.tabs_ctl, TCM_SETCURSEL, (i + 1).min(self.tabs.len()), 0);
                    self.layout();
                    self.show_current();
                }
            }
            Command::MenuAdd { id, label } => {
                if self.app_menu.is_null() {
                    self.app_menu = CreatePopupMenu();
                    let bar = windows_sys::Win32::UI::WindowsAndMessaging::GetMenu(self.hwnd);
                    AppendMenuW(bar, MF_POPUP, self.app_menu as usize, wide("&Application").as_ptr());
                }
                let cmd_id = ID_MENU_BASE + self.menu_items.len() as u32;
                AppendMenuW(self.app_menu, MF_STRING, cmd_id as usize, wide(&label).as_ptr());
                self.menu_items.push(id);
                DrawMenuBar(self.hwnd);
            }
            Command::Tray { enabled } => {
                self.tray = enabled;
                self.tray_icon(enabled);
            }
            Command::Show { visible } => {
                if visible {
                    self.restore();
                } else {
                    ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            Command::Quit => {
                DestroyWindow(self.hwnd);
            }
        }
    }

    unsafe fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = 1;
        nid
    }

    unsafe fn tray_icon(&mut self, show: bool) {
        let mut nid = self.tray_data();
        if show && !self.tray_shown {
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_QV_TRAY;
            nid.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
            for (i, c) in self.title.encode_utf16().take(nid.szTip.len() - 1).enumerate() {
                nid.szTip[i] = c;
            }
            self.tray_shown = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
        } else if !show && self.tray_shown {
            Shell_NotifyIconW(NIM_DELETE, &nid);
            self.tray_shown = false;
        }
    }

    unsafe fn restore(&self) {
        ShowWindow(self.hwnd, SW_SHOW);
        ShowWindow(self.hwnd, SW_RESTORE);
        SetForegroundWindow(self.hwnd);
    }

    unsafe fn tray_menu(&self) {
        let menu = CreatePopupMenu();
        AppendMenuW(menu, MF_STRING, ID_OPEN as usize, wide("Open").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, ID_START_ALL as usize, wide("Start all").as_ptr());
        AppendMenuW(menu, MF_STRING, ID_STOP_ALL as usize, wide("Stop all").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, ID_EXIT as usize, wide("Exit").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Without these two calls a menu opened from the notification
        // area does not close when the user clicks elsewhere.
        SetForegroundWindow(self.hwnd);
        TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, self.hwnd, null());
        PostMessageW(self.hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }

    /// Asks the host for one step of a search in the console in front.
    /// A search never runs here, and never runs to the end of the log by
    /// itself: each step is bounded, and going on is the user's choice.
    unsafe fn find(&mut self, newer: bool) {
        let Some(i) = self.current() else { return };
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(self.find_edit, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let text = String::from_utf16_lossy(&buf[..n]);
        if text.trim().is_empty() {
            SetFocus(self.find_edit);
            return;
        }
        let tab = &mut self.tabs[i];
        // The same question again after a step that ran out of budget
        // goes on from where that step stopped.
        let from = if tab.find_resume != 0 && tab.last_query == text && tab.last_newer == newer {
            tab.find_resume
        } else {
            view::of(tab.view).map(|v| v.find_from(newer)).unwrap_or(0)
        };
        tab.last_query = text.clone();
        tab.last_newer = newer;
        tab.find_resume = 0;
        tab.find_status = format!("Looking for \"{text}\"...");
        tab.find_offer = false;
        let id = tab.id;
        // The history is searched only if the user opened it here, by
        // scrolling into it or by accepting it for a search.
        let history = tab.find_history || view::of(tab.view).map(|v| v.in_history()).unwrap_or(false);
        let op = newer as u32 | if history { 2 } else { 0 };
        self.shared.set_find_text(id, text);
        self.shared.emit(Event { kind: event::CONSOLE_FIND, id, op, n: 0, seq: from });
        self.refresh_strip();
    }

    unsafe fn command(&mut self, id: u32) {
        match id {
            ID_FIND => {
                if self.current().is_some() {
                    SetFocus(self.find_edit);
                    SendMessageW(self.find_edit, EM_SETSEL, 0, -1);
                }
            }
            ID_HISTORY => {
                // The user asks for the history: of the search that just
                // ran out, or of the output on screen.
                let Some(i) = self.current() else { return };
                if self.tabs[i].find_offer {
                    self.tabs[i].find_history = true;
                    self.tabs[i].find_offer = false;
                    let newer = self.tabs[i].last_newer;
                    self.find(newer);
                } else if let Some(v) = view::of(self.tabs[i].view) {
                    v.request_history();
                }
                SetFocus(self.tabs[i].view);
            }
            ID_FIND_OLDER | ID_FIND_NEWER => {
                self.find(id == ID_FIND_NEWER);
                // A click leaves the keyboard on the button; it goes back
                // to the console, where the next key is expected.
                let focus = GetFocus();
                if focus == self.find_older || focus == self.find_newer {
                    if let Some(i) = self.current() {
                        SetFocus(self.tabs[i].view);
                    }
                }
            }
            // Enter and Escape reach the window as commands. They mean
            // something only while the search box has the keyboard.
            ID_ENTER if GetFocus() == self.find_edit => {
                self.find(GetKeyState(VK_SHIFT as i32) < 0);
            }
            ID_ESCAPE if GetFocus() == self.find_edit => {
                if let Some(i) = self.current() {
                    SetFocus(self.tabs[i].view);
                }
            }
            ID_START_ALL => self.shared.emit(Event::simple(event::START_ALL, 0)),
            ID_STOP_ALL => self.shared.emit(Event::simple(event::STOP_ALL, 0)),
            ID_EXIT => self.shared.emit(Event::simple(event::CLOSE, 0)),
            ID_OPEN => self.restore(),
            ID_TIME => {
                self.show_time = !self.show_time;
                let mark = if self.show_time { MF_CHECKED } else { MF_UNCHECKED };
                CheckMenuItem(self.console_menu, ID_TIME, mark);
                let views: Vec<HWND> = self.tabs.iter().map(|t| t.view).collect();
                for h in views {
                    if let Some(v) = view::of(h) {
                        v.set_show_time(self.show_time);
                    }
                }
            }
            ID_TOGGLE | ID_COPY => {
                let Some(i) = self.current() else { return };
                let hview = self.tabs[i].view;
                if let Some(v) = view::of(hview) {
                    if id == ID_COPY {
                        v.copy();
                    } else if v.status() == Status::Following {
                        v.pause();
                    } else {
                        v.follow();
                    }
                }
                SetFocus(hview);
            }
            id if id >= ID_MENU_BASE => {
                if let Some(&item) = self.menu_items.get((id - ID_MENU_BASE) as usize) {
                    self.shared.emit(Event::simple(event::MENU, item));
                }
            }
            id if id >= ID_UNIT_BASE => {
                if let Some(u) = self.units.get((id - ID_UNIT_BASE) as usize) {
                    let kind = if u.state.offers_stop() { event::UNIT_STOP } else { event::UNIT_START };
                    self.shared.emit(Event::simple(kind, u.id));
                }
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCDESTROY {
        if let Some(app) = take_state::<App>(hwnd) {
            DeleteObject(app.font_ui);
            DeleteObject(app.font_mono);
            DeleteObject(app.font_icon);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let Some(app) = state::<App>(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        WM_QV_WAKE => {
            for cmd in app.shared.drain() {
                // A quit destroys the window, and with it `app`.
                let quit = matches!(cmd, Command::Quit);
                app.apply(cmd);
                if quit {
                    break;
                }
            }
            0
        }
        WM_QV_STATE => {
            if let Some(i) = app.current() {
                if app.tabs[i].id == wparam as u32 {
                    app.refresh_strip();
                }
            }
            0
        }
        WM_QV_TRAY => {
            match loword(lparam as usize) {
                WM_LBUTTONDBLCLK => app.restore(),
                WM_RBUTTONUP => app.tray_menu(),
                _ => {}
            }
            0
        }
        WM_SIZE => {
            if wparam as u32 == SIZE_MINIMIZED && app.tray {
                // Minimizing with a tray icon means "get out of the way".
                ShowWindow(hwnd, SW_HIDE);
            } else {
                app.layout();
            }
            0
        }
        WM_DPICHANGED => {
            app.dpi = hiword(wparam).max(96);
            app.make_fonts();
            for h in [app.start_all, app.stop_all, app.tabs_ctl, app.strip, app.find_edit, app.history] {
                SendMessageW(h, WM_SETFONT, app.font_ui as usize, 1);
            }
            for h in [app.toggle, app.find_older, app.find_newer] {
                SendMessageW(h, WM_SETFONT, app.font_icon as usize, 1);
            }
            for u in &app.units {
                SendMessageW(u.button, WM_SETFONT, app.font_ui as usize, 1);
                SendMessageW(u.label, WM_SETFONT, app.font_ui as usize, 1);
            }
            let (font, cell) = (app.font_mono, app.cell);
            let views: Vec<HWND> = app.tabs.iter().map(|t| t.view).collect();
            for h in views {
                if let Some(v) = view::of(h) {
                    v.set_font(font, cell.0, cell.1);
                }
            }
            // The system suggests where the window should go at the new
            // scale.
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            app.layout();
            0
        }
        WM_COMMAND => {
            app.command(loword(wparam));
            0
        }
        WM_NOTIFY => {
            let hdr = &*(lparam as *const NMHDR);
            if hdr.hwndFrom == app.tabs_ctl && hdr.code == TCN_SELCHANGE {
                app.show_current();
                if let Some(i) = app.current() {
                    SetFocus(app.tabs[i].view);
                }
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            // A label takes the color of what it sits on: the status
            // strip is on the window, the unit labels on a tab page.
            SetBkMode(wparam as _, TRANSPARENT as _);
            let on = if lparam as HWND == app.strip { COLOR_BTNFACE } else { COLOR_WINDOW };
            // The state of a unit is told by color too.
            if let Some(u) = app.units.iter().find(|u| u.label == lparam as HWND) {
                let color = match u.state {
                    UnitState::Running => util::rgb(0x10, 0x7c, 0x10),
                    UnitState::Failed => util::rgb(0xc4, 0x2b, 0x1c),
                    UnitState::Starting | UnitState::Stopping => util::rgb(0x9d, 0x5d, 0x00),
                    UnitState::Stopped => util::rgb(0x60, 0x60, 0x60),
                };
                SetTextColor(wparam as _, color);
            }
            GetSysColorBrush(on) as LRESULT
        }
        WM_CLOSE => {
            if app.tray && IsWindowVisible(hwnd) != 0 {
                // With a tray icon the cross hides; Exit is in the menu.
                ShowWindow(hwnd, SW_HIDE);
            } else {
                // Closing is the application's decision: it stops in
                // order and then tells this window to go.
                app.shared.emit(Event::simple(event::CLOSE, 0));
            }
            0
        }
        WM_DESTROY => {
            app.tray_icon(false);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
