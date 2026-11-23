//! Win32 backend of the quevu visual part.
//!
//! One window with a row of tabs, and nothing else of its own. Every tab
//! is opened by the host, which gives it its name and its weight, and is a
//! whole page: what it shows and, under it, a bar with its tools and its
//! status. There are two kinds:
//!
//! - a source tab holds a pseudo-console over a log, with the search and
//!   the button that holds the view;
//! - a control tab holds a table of controls, each with a light, a name,
//!   the text of its state and the buttons of its verbs.
//!
//! The menu bar and the menu of the icon in the notification area are the
//! host's too. The window has no word of any language and decides nothing:
//! what the user does leaves as an event, and the window goes away only
//! when the host says so.
//!
//! Everything here runs on the thread that called [`run`]. The host
//! reaches it only through `quevu_ui_core::shared`: commands arrive as a
//! posted message that makes this thread drain the queue, and what the
//! user does leaves as events.
#![cfg(windows)]

mod table;
mod util;
mod view;

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;

use quevu_ui_core::console::Status;
use quevu_ui_core::shared::{event, Command, Event, FindParams, Shared, Verb};
use quevu_ui_core::texts::{self, Texts};
use quevu_ui_core::{attr, menu, tabs};

use windows_sys::Win32::Foundation::{
    FILETIME, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateFontW, DeleteObject, GetDC, GetSysColorBrush, GetTextMetricsW,
    ReleaseDC, SelectObject, SetBkMode, COLOR_BTNFACE, HFONT, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
    ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
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
    AppendMenuW, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, DrawMenuBar, GetClientRect, GetCursorPos, GetMessageW,
    GetParent, GetWindowRect, GetWindowTextW, IsDialogMessageW, LoadCursorW, LoadIconW,
    MoveWindow, PostMessageW, PostQuitMessage, RegisterClassExW, RemoveMenu, SendMessageW,
    SetForegroundWindow, SetMenu, SetWindowPos, SetWindowTextW, ShowWindow, TrackPopupMenu,
    TranslateMessage, BS_PUSHBUTTON, CW_USEDEFAULT, HMENU, HWND_BOTTOM, HWND_TOP, IDC_ARROW,
    IDI_APPLICATION, MF_BYPOSITION, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    MSG, NONCLIENTMETRICSW, SIZE_MINIMIZED, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_NCDESTROY,
    WM_NOTIFY, WM_NULL, WM_RBUTTONUP, WM_SETFONT, WM_SIZE, WNDCLASSEXW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE,
};

use util::{button, button_width, hiword, loword, scale, set_state, state, take_state, wide};

/// The host queued commands: drain them.
const WM_QV_WAKE: u32 = WM_APP + 1;
/// The console of tab `wparam` changed state: refresh its status strip.
pub(crate) const WM_QV_STATE: u32 = WM_APP + 2;
/// Something happened to the icon in the notification area.
const WM_QV_TRAY: u32 = WM_APP + 3;

const CLASS: &str = "QuevuPanelWindow";
/// What a tab holds: a window of its own that carries the content of the
/// tab and its bar, and passes what happens on them up to the main window.
const PAGE_CLASS: &str = "QuevuPanelPage";

// Pause and play in the symbol font of the system (Segoe MDL2 Assets).
const GLYPH_PAUSE: &str = "\u{e769}";
const GLYPH_PLAY: &str = "\u{e768}";
// Chevrons, for walking through the matches of a search.
const GLYPH_UP: &str = "\u{e70e}";
const GLYPH_DOWN: &str = "\u{e70d}";
// "More": opens the rest of what a search can be given.
const GLYPH_MORE: &str = "\u{e712}";
// The three lines that open a menu.
const GLYPH_MENU: &str = "\u{e700}";

// Styles of the STATIC control.
const SS_NOPREFIX: u32 = 0x80;
const SS_ENDELLIPSIS: u32 = 0x4000;

// Command identifiers of a source tab. Its buttons, the entries of its
// menu and the keys that do the same thing share one.
const ID_TOGGLE: u32 = 110;
const ID_COPY: u32 = 111;
const ID_TIME: u32 = 113;
const ID_FIND: u32 = 114;
const ID_FIND_OLDER: u32 = 115;
const ID_FIND_NEWER: u32 = 116;
const ID_HISTORY: u32 = 117;
const ID_MORE: u32 = 118;
const ID_TABMENU: u32 = 119;
/// Every button of the bar of a control tab: which one is told by its
/// handle.
const ID_TAB_VERB: u32 = 120;

// The date and time picker control.
const DTS_SHOWNONE: u32 = 0x2;
const DTM_GETSYSTEMTIME: u32 = 0x1001;
const DTM_SETSYSTEMTIME: u32 = 0x1002;
const DTM_SETFORMATW: u32 = 0x1032;
const GDT_VALID: isize = 0;
const GDT_NONE: usize = 1;
const ICC_DATE_CLASSES: u32 = 0x100;
// What a dialog-style window gets for Enter and Escape.
const ID_ENTER: u32 = 1;
const ID_ESCAPE: u32 = 2;

// Styles and messages of the EDIT control.
const ES_AUTOHSCROLL: u32 = 0x80;
const EM_SETSEL: u32 = 0xb1;
const EM_SETCUEBANNER: u32 = 0x1501;
/// The entries of the host's menus count up from here.
const ID_MENU_BASE: u32 = 5000;

/// What a source tab holds besides its console: its bar, and where its
/// search stands.
struct Source {
    view: HWND,
    strip: HWND,
    toggle: HWND,
    menu: HWND,
    find_edit: HWND,
    find_older: HWND,
    find_newer: HWND,
    find_more: HWND,
    /// The rest of the search: a range of time, in a row that folds away.
    range_open: bool,
    from_label: HWND,
    from_pick: HWND,
    to_label: HWND,
    to_pick: HWND,
    history: HWND,
    /// Whether each line shows when it entered the log.
    show_time: bool,
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
    /// The time range of the last search, to tell "the same thing".
    last_since: i64,
    last_until: i64,
    /// Whether the console was following the last time it was looked at.
    was_following: bool,
}

/// What a control tab holds: the table, and the verbs of the tab itself.
struct Controls {
    table: HWND,
    strip: HWND,
    verbs: Vec<Verb>,
    buttons: Vec<HWND>,
}

enum Kind {
    Source(Box<Source>),
    Controls(Controls),
}

struct Tab {
    id: u32,
    title: String,
    weight: i32,
    page: HWND,
    /// What the host has to say in the status strip.
    note: String,
    /// Output arrived while another tab was in front.
    unseen: bool,
    kind: Kind,
}

/// An entry of one of the host's menus.
struct MenuItem {
    id: u32,
    parent: u32,
    label: String,
    attrs: u32,
    /// The menu this entry opens, if it opens one.
    popup: HMENU,
    /// The number Windows sends back when the entry is chosen.
    cmd: u32,
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
    tabs_ctl: HWND,
    /// The tabs, in the order they are shown.
    tabs: Vec<Tab>,
    /// The tab in front, by its identifier.
    front: Option<u32>,
    texts: Texts,
    bar: HMENU,
    tray_menu: HMENU,
    items: Vec<MenuItem>,
    next_cmd: u32,
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
        dwICC: ICC_TAB_CLASSES | ICC_DATE_CLASSES,
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
        lpfnWndProc: Some(page_proc),
        hInstance: hinst,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
        lpszClassName: page_class.as_ptr(),
        ..zeroed()
    };
    RegisterClassExW(&page);
    view::register(hinst);
    table::register(hinst);

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
        tabs_ctl: null_mut(),
        tabs: Vec::new(),
        front: None,
        texts: Texts::new(),
        bar: null_mut(),
        tray_menu: null_mut(),
        items: Vec::new(),
        next_cmd: ID_MENU_BASE,
        tray: false,
        tray_shown: false,
    });
    set_state(hwnd, Box::into_raw(app));
    // What is applied below already has things to tell the host.
    shared.begin();
    if let Some(app) = state::<App>(hwnd) {
        app.build();
        // Whatever the host queued before the window existed is applied
        // before the window is shown: it opens with its tabs and its menu,
        // not empty and then filling up.
        for cmd in shared.drain() {
            // A quit asked for while no window was open does nothing: it
            // must not close a window that came afterwards.
            if !matches!(cmd, Command::Quit) {
                app.apply(cmd);
            }
        }
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
    // Whatever the host queued in the meantime.
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
        // Lets Tab walk through the buttons like in a dialog.
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

/// The time a picker holds, as nanoseconds since the Unix epoch, or zero
/// if its box is not ticked.
unsafe fn picked(h: HWND) -> i64 {
    let mut local: SYSTEMTIME = zeroed();
    if SendMessageW(h, DTM_GETSYSTEMTIME, 0, &mut local as *mut SYSTEMTIME as LPARAM) != GDT_VALID {
        return 0;
    }
    // The user thinks in the time of the machine; chunks are stamped in
    // universal time.
    let mut utc: SYSTEMTIME = zeroed();
    let mut ft: FILETIME = zeroed();
    if TzSpecificLocalTimeToSystemTime(null(), &local, &mut utc) == 0 || SystemTimeToFileTime(&utc, &mut ft) == 0 {
        return 0;
    }
    // Windows counts in units of 100 ns from 1601.
    let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    (ticks as i64 - 116_444_736_000_000_000) * 100
}


impl App {
    fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }

    /// A child of `parent` in the font of the user interface.
    unsafe fn child(&self, parent: HWND, class: &str, text: &str, style: u32, id: u32) -> HWND {
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
            parent,
            id as usize as HMENU,
            self.hinst,
            null(),
        );
        SendMessageW(h, WM_SETFONT, self.font_ui as usize, 1);
        h
    }

    /// A button of a bar that shows a symbol instead of words.
    unsafe fn glyph(&self, parent: HWND, glyph: &str, id: u32) -> HWND {
        let h = self.child(parent, "BUTTON", glyph, BS_PUSHBUTTON as u32 | WS_TABSTOP, id);
        SendMessageW(h, WM_SETFONT, self.font_icon as usize, 1);
        h
    }

    /// The window a tab holds, hidden until the tab is in front.
    unsafe fn page(&self) -> HWND {
        let class = wide(PAGE_CLASS);
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            self.hwnd,
            null_mut(),
            self.hinst,
            null(),
        )
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
        // The symbol font of the system, for the buttons of a bar that
        // need no words: the pause and play marks of a tape deck, the
        // chevrons of a search.
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

    /// The window starts with the row of tabs and nothing in it: no tab,
    /// no menu. Both are the host's to add.
    unsafe fn build(&mut self) {
        self.make_fonts();
        let class = wide("SysTabControl32");
        self.tabs_ctl = CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP,
            0,
            0,
            0,
            0,
            self.hwnd,
            null_mut(),
            self.hinst,
            null(),
        );
        SendMessageW(self.tabs_ctl, WM_SETFONT, self.font_ui as usize, 1);
        self.layout();
    }

    /// The position of a tab in the row, by its identifier.
    fn index(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The tab in front, if there is any.
    fn current(&self) -> Option<usize> {
        self.front.and_then(|id| self.index(id))
    }

    /// The source tab at a position, if that is what is there.
    fn source(&mut self, i: usize) -> Option<&mut Source> {
        match self.tabs.get_mut(i).map(|t| &mut t.kind) {
            Some(Kind::Source(s)) => Some(s),
            _ => None,
        }
    }

    /// The control tab that holds a control.
    unsafe fn table_of(&self, control: u32) -> Option<HWND> {
        self.tabs.iter().find_map(|t| match &t.kind {
            Kind::Controls(c) if table::of(c.table).is_some_and(|tb| tb.has(control)) => Some(c.table),
            _ => None,
        })
    }

    /// Puts a new tab in the row where its weight says, and in front if it
    /// is the first one.
    unsafe fn insert(&mut self, tab: Tab) {
        let weights: Vec<i32> = self.tabs.iter().map(|t| t.weight).collect();
        let at = tabs::insert_at(&weights, tab.weight);
        let mut text = wide(&tab.title);
        let mut item: TCITEMW = zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = text.as_mut_ptr();
        SendMessageW(self.tabs_ctl, TCM_INSERTITEMW, at, &item as *const TCITEMW as LPARAM);
        if self.front.is_none() {
            self.front = Some(tab.id);
        }
        self.tabs.insert(at, tab);
        self.sync_front();
        self.layout();
        self.show_current();
    }

    /// Makes the row of tabs agree with which tab is in front.
    unsafe fn sync_front(&self) {
        let sel = self.current().map(|i| i as isize).unwrap_or(-1);
        SendMessageW(self.tabs_ctl, TCM_SETCURSEL, sel as usize, 0);
    }

    unsafe fn add_source(&mut self, id: u32, title: String, weight: i32) {
        let page = self.page();
        let view = view::create(page, self.hinst, id, self.shared.clone(), self.font_mono, self.cell.0, self.cell.1);
        ShowWindow(view, SW_SHOW);
        let strip = self.child(page, "STATIC", "", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
        // The one way into the history. It shows up only when there is a
        // history to offer, and nothing goes there until it is pressed.
        let history = self.child(page, "BUTTON", self.texts.get(texts::HISTORY), BS_PUSHBUTTON as u32 | WS_TABSTOP, ID_HISTORY);
        ShowWindow(history, SW_HIDE);
        // The box a search is typed in. It needs a border of its own,
        // hence not through child().
        let edit_class = wide("EDIT");
        let find_edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            edit_class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            0,
            0,
            page,
            null_mut(),
            self.hinst,
            null(),
        );
        SendMessageW(find_edit, WM_SETFONT, self.font_ui as usize, 1);
        // The same two steps the keys take, for the mouse: up is older,
        // down is newer, as the text itself runs.
        let find_older = self.glyph(page, GLYPH_UP, ID_FIND_OLDER);
        let find_newer = self.glyph(page, GLYPH_DOWN, ID_FIND_NEWER);
        // What else a search can be given lives in a row that folds away:
        // for now, a range of time. Each end has a box to tick; unticked,
        // that side has no bound.
        let find_more = self.glyph(page, GLYPH_MORE, ID_MORE);
        let from_label = self.child(page, "STATIC", self.texts.get(texts::FROM), SS_NOPREFIX, 0);
        let from_pick = self.child(page, "SysDateTimePick32", "", DTS_SHOWNONE | WS_TABSTOP, 0);
        let to_label = self.child(page, "STATIC", self.texts.get(texts::TO), SS_NOPREFIX, 0);
        let to_pick = self.child(page, "SysDateTimePick32", "", DTS_SHOWNONE | WS_TABSTOP, 0);
        let format = wide("yyyy'-'MM'-'dd  HH':'mm':'ss");
        for h in [from_pick, to_pick] {
            SendMessageW(h, DTM_SETFORMATW, 0, format.as_ptr() as LPARAM);
            // Unticked to begin with: no bound.
            SendMessageW(h, DTM_SETSYSTEMTIME, GDT_NONE, 0);
        }
        for h in [from_label, from_pick, to_label, to_pick] {
            ShowWindow(h, SW_HIDE);
        }
        // The button that holds the view shows what pressing it does, as
        // on a tape deck; next to it, the one that opens the menu of the
        // tab.
        let toggle = self.glyph(page, GLYPH_PAUSE, ID_TOGGLE);
        EnableWindow(toggle, 0);
        let menu = self.glyph(page, GLYPH_MENU, ID_TABMENU);

        let source = Source {
            view,
            strip,
            toggle,
            menu,
            find_edit,
            find_older,
            find_newer,
            find_more,
            range_open: false,
            from_label,
            from_pick,
            to_label,
            to_pick,
            history,
            show_time: false,
            find_status: String::new(),
            find_resume: 0,
            last_query: String::new(),
            last_newer: false,
            find_offer: false,
            find_history: false,
            last_since: 0,
            last_until: 0,
            was_following: false,
        };
        self.set_cue(find_edit);
        self.insert(Tab { id, title, weight, page, note: String::new(), unseen: false, kind: Kind::Source(Box::new(source)) });
        // The console asks for its first page as soon as it exists.
        if let Some(v) = view::of(view) {
            v.follow();
        }
    }

    unsafe fn add_controls(&mut self, id: u32, title: String, weight: i32) {
        let page = self.page();
        let table = table::create(page, self.hinst, self.shared.clone(), self.font_ui, self.dpi);
        let strip = self.child(page, "STATIC", "", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
        let controls = Controls { table, strip, verbs: Vec::new(), buttons: Vec::new() };
        self.insert(Tab { id, title, weight, page, note: String::new(), unseen: false, kind: Kind::Controls(controls) });
    }

    /// What the empty search box says.
    unsafe fn set_cue(&self, edit: HWND) {
        let cue = wide(self.texts.get(texts::FIND_CUE));
        SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
    }

    unsafe fn layout(&mut self) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let pad = self.s(8);
        // With no tab there is nothing to show, not even an empty row.
        ShowWindow(self.tabs_ctl, if self.tabs.is_empty() { SW_HIDE } else { SW_SHOW });

        // One row of tabs across the whole window, and under it the page
        // of the tab in front.
        let tw = (w - pad * 2).max(0);
        let th = (h - pad * 2).max(0);
        MoveWindow(self.tabs_ctl, pad, pad, tw, th, 1);
        let mut area = RECT { left: 0, top: 0, right: tw, bottom: th };
        // Asks the tab control how much of itself is left for content.
        SendMessageW(self.tabs_ctl, TCM_ADJUSTRECT, 0, &mut area as *mut RECT as LPARAM);
        let (pw, ph) = ((area.right - area.left).max(0), (area.bottom - area.top).max(0));
        for i in 0..self.tabs.len() {
            MoveWindow(self.tabs[i].page, pad + area.left, pad + area.top, pw, ph, 1);
            self.layout_page(i, pw, ph);
        }
        // The pages go over the tab control, never under it.
        SetWindowPos(self.tabs_ctl, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }

    /// Lays out the page of one tab again, at the size it already has.
    unsafe fn relayout_page(&mut self, i: usize) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.tabs[i].page, &mut rc);
        self.layout_page(i, rc.right, rc.bottom);
    }

    /// Lays out what a tab holds: its content on top and its bar under it.
    unsafe fn layout_page(&mut self, i: usize, w: i32, h: i32) {
        let pad = self.s(8);
        let bh = self.s(28);
        let fb = self.s(26);
        let bar_h = self.s(36);
        let gap = self.s(2);
        match &self.tabs[i].kind {
            Kind::Source(s) => {
                let range_h = if s.range_open { self.s(34) } else { 0 };
                let top = (h - bar_h - range_h).max(0);
                MoveWindow(s.view, 0, 0, w, top, 1);

                // The folding row sits between the console and the bar,
                // under the search box it belongs to.
                let right = w - self.s(4);
                let ry = top + self.s(6);
                let pw = self.s(196);
                let ph = self.s(24);
                MoveWindow(s.to_pick, right - pw, ry, pw, ph, 1);
                MoveWindow(s.to_label, right - pw - self.s(28), ry + self.s(3), self.s(24), self.s(18), 1);
                MoveWindow(s.from_pick, right - pw * 2 - self.s(44), ry, pw, ph, 1);
                MoveWindow(s.from_label, right - pw * 2 - self.s(86), ry + self.s(3), self.s(38), self.s(18), 1);

                // The bar, from its right end: the menu of the tab, the
                // button that holds the view, the search, the way into the
                // history; what is left is the status strip.
                let sy = top + range_h + (bar_h - bh) / 2;
                let mut x = right;
                for b in [s.menu, s.toggle] {
                    x -= fb;
                    MoveWindow(b, x, sy + self.s(1), fb, fb, 1);
                    x -= gap;
                }
                x -= pad - gap;
                for b in [s.find_more, s.find_newer, s.find_older] {
                    x -= fb;
                    MoveWindow(b, x, sy + self.s(1), fb, fb, 1);
                    x -= gap;
                }
                let fw = self.s(260).min(w / 3);
                x -= fw;
                MoveWindow(s.find_edit, x, sy + self.s(2), fw, self.s(24), 1);
                let hw = button_width(self.hwnd, self.font_ui, self.texts.get(texts::HISTORY), self.dpi).max(self.s(76));
                x -= pad + hw;
                MoveWindow(s.history, x, sy + self.s(1), hw, fb, 1);
                x -= pad;
                MoveWindow(s.strip, self.s(6), sy + self.s(5), (x - self.s(6)).max(0), self.s(18), 1);
            }
            Kind::Controls(c) => {
                // A control tab with nothing to say and no verbs of its
                // own has no bar: the table takes it all.
                let bar_h = if c.verbs.is_empty() && self.tabs[i].note.is_empty() { 0 } else { bar_h };
                let top = (h - bar_h).max(0);
                MoveWindow(c.table, 0, 0, w, top, 1);
                let sy = top + (bar_h - bh) / 2;
                let widths: Vec<i32> = c.verbs.iter().map(|v| button_width(self.hwnd, self.font_ui, &v.label, self.dpi)).collect();
                let total: i32 = widths.iter().sum::<i32>() + self.s(6) * (widths.len() as i32 - 1).max(0);
                let mut x = w - self.s(4) - total;
                let strip_w = (x - pad - self.s(6)).max(0);
                for (b, bw) in c.buttons.iter().zip(&widths) {
                    MoveWindow(*b, x, sy + self.s(1), *bw, if bar_h > 0 { fb } else { 0 }, 1);
                    x += bw + self.s(6);
                }
                MoveWindow(c.strip, self.s(6), sy + self.s(5), strip_w, if bar_h > 0 { self.s(18) } else { 0 }, 1);
            }
        }
    }

    /// Shows the page of the tab in front and hides the others.
    unsafe fn show_current(&mut self) {
        let cur = self.current();
        if let Some(i) = cur {
            if self.tabs[i].unseen {
                self.tabs[i].unseen = false;
                self.retitle(i);
            }
        }
        for (i, t) in self.tabs.iter().enumerate() {
            if Some(i) == cur {
                SetWindowPos(t.page, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                ShowWindow(t.page, SW_SHOW);
            } else {
                ShowWindow(t.page, SW_HIDE);
            }
        }
        if let Some(i) = cur {
            self.refresh_strip(i);
        }
    }

    /// Writes the title of a tab, with a mark if it holds output nobody
    /// has looked at.
    unsafe fn retitle(&self, i: usize) {
        let t = &self.tabs[i];
        let label = if t.unseen { format!("{} \u{2022}", t.title) } else { t.title.clone() };
        let mut text = wide(&label);
        let mut item: TCITEMW = zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = text.as_mut_ptr();
        SendMessageW(self.tabs_ctl, TCM_SETITEMW, i, &item as *const TCITEMW as LPARAM);
    }

    /// Brings the status strip of a tab up to date. Each tab has its own,
    /// so it is kept right whether the tab is in front or not.
    unsafe fn refresh_strip(&mut self, i: usize) {
        let note = self.tabs[i].note.clone();
        let (waiting, following, paused, paused_full, offer_text) = (
            self.texts.get(texts::WAITING).to_string(),
            self.texts.get(texts::FOLLOWING).to_string(),
            self.texts.get(texts::PAUSED).to_string(),
            self.texts.get(texts::PAUSED_FULL).to_string(),
            self.texts.get(texts::HISTORY_OFFER).to_string(),
        );
        let s = match &mut self.tabs[i].kind {
            Kind::Controls(c) => {
                // A table of controls says only what the host has to say.
                SetWindowTextW(c.strip, wide(&note).as_ptr());
                return;
            }
            Kind::Source(s) => s,
        };
        let status = view::of(s.view).map(|v| v.status()).unwrap_or(Status::Waiting);
        let following_now = status == Status::Following;
        if following_now && !s.was_following {
            // Back on the live output: the search is over, and so is any
            // leave to look into the history. Only on the way back: a
            // search made while following has things to say too.
            s.find_status.clear();
            s.find_resume = 0;
            s.find_offer = false;
            s.find_history = false;
        }
        s.was_following = following_now;
        let scroll_offer = view::of(s.view).map(|v| v.history_offered()).unwrap_or(false);
        let offer = scroll_offer || s.find_offer;
        ShowWindow(s.history, if offer { SW_SHOW } else { SW_HIDE });
        // The button shows what pressing it does, as on a tape deck. The
        // text says in the host's words what state the view is in.
        let (text, glyph) = match status {
            Status::Waiting => (waiting, GLYPH_PAUSE),
            Status::Following => (following, GLYPH_PAUSE),
            Status::Paused => (paused, GLYPH_PLAY),
            Status::PausedFull => (paused_full, GLYPH_PLAY),
        };
        // While a search is going on, the strip tells how it went.
        let mut line = if !s.find_status.is_empty() {
            s.find_status.clone()
        } else if scroll_offer {
            offer_text
        } else {
            text
        };
        if s.find_status.is_empty() && !note.is_empty() {
            if !line.is_empty() {
                line.push_str("   |   ");
            }
            line.push_str(&note);
        }
        SetWindowTextW(s.strip, wide(&line).as_ptr());
        SetWindowTextW(s.toggle, wide(glyph).as_ptr());
        EnableWindow(s.toggle, 1);
    }

    /// The handle of the menu the entries under `parent` hang from, made
    /// the first time something hangs from it.
    unsafe fn menu_handle(&mut self, parent: u32) -> HMENU {
        match parent {
            menu::BAR => {
                if self.bar.is_null() {
                    self.bar = CreateMenu();
                }
                self.bar
            }
            menu::TRAY => {
                if self.tray_menu.is_null() {
                    self.tray_menu = CreatePopupMenu();
                }
                self.tray_menu
            }
            id => self.items.iter().find(|m| m.id == id).map(|m| m.popup).unwrap_or(null_mut()),
        }
    }

    /// Writes a menu again from the entries the host has given for it.
    unsafe fn rebuild_menu(&mut self, parent: u32) {
        let h = self.menu_handle(parent);
        if h.is_null() {
            return;
        }
        // Taking an entry out does not destroy the menu it opens.
        while RemoveMenu(h, 0, MF_BYPOSITION) != 0 {}
        let mut any = false;
        for m in self.items.iter().filter(|m| m.parent == parent) {
            any = true;
            if m.attrs & attr::MENU_SEPARATOR != 0 {
                AppendMenuW(h, MF_SEPARATOR, 0, null());
                continue;
            }
            let mut flags = MF_STRING;
            if m.attrs & attr::MENU_CHECKED != 0 {
                flags |= MF_CHECKED;
            }
            if m.attrs & attr::MENU_DISABLED != 0 {
                flags |= MF_GRAYED;
            }
            let label = wide(&m.label);
            if m.popup.is_null() {
                AppendMenuW(h, flags, m.cmd as usize, label.as_ptr());
            } else {
                AppendMenuW(h, flags | MF_POPUP, m.popup as usize, label.as_ptr());
            }
        }
        if parent == menu::BAR {
            // A bar with nothing on it is not shown at all.
            SetMenu(self.hwnd, if any { self.bar } else { null_mut() });
        }
        DrawMenuBar(self.hwnd);
        self.layout();
    }

    unsafe fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::ConsoleAdd { id, title, weight } => self.add_source(id, title, weight),
            Command::ControlsAdd { id, title, weight } => self.add_controls(id, title, weight),
            Command::TabTitle { id, title } => {
                if let Some(i) = self.index(id) {
                    self.tabs[i].title = title;
                    self.retitle(i);
                    // A longer name can make the row of tabs taller.
                    self.layout();
                }
            }
            Command::TabSelect { id } => {
                if self.index(id).is_some() {
                    self.front = Some(id);
                    self.sync_front();
                    self.show_current();
                }
            }
            Command::TabRemove { id } => {
                if let Some(i) = self.index(id) {
                    let t = self.tabs.remove(i);
                    // The page takes everything on it along.
                    DestroyWindow(t.page);
                    SendMessageW(self.tabs_ctl, TCM_DELETEITEM, i, 0);
                    if self.front == Some(id) {
                        // The neighbour comes to the front, if there is one.
                        self.front = tabs::front_after_remove(i, self.tabs.len()).map(|j| self.tabs[j].id);
                    }
                    self.sync_front();
                    self.layout();
                    self.show_current();
                }
            }
            Command::TabNote { tab, text } => {
                if let Some(i) = self.index(tab) {
                    self.tabs[i].note = text;
                    // The bar of a control tab comes and goes with it.
                    self.relayout_page(i);
                    self.refresh_strip(i);
                }
            }
            Command::TabVerbs { tab, verbs } => {
                let Some(i) = self.index(tab) else { return };
                let (page, hinst, font) = (self.tabs[i].page, self.hinst, self.font_ui);
                if let Kind::Controls(c) = &mut self.tabs[i].kind {
                    if c.verbs != verbs {
                        for b in c.buttons.drain(..) {
                            DestroyWindow(b);
                        }
                        for v in &verbs {
                            c.buttons.push(button(page, hinst, font, &v.label, v.attrs & attr::VERB_DISABLED != 0, ID_TAB_VERB));
                        }
                        c.verbs = verbs;
                    }
                    self.relayout_page(i);
                }
            }
            Command::ControlAdd { id, tab, name } => {
                if let Some(Kind::Controls(c)) = self.index(tab).map(|i| &self.tabs[i].kind) {
                    if let Some(t) = table::of(c.table) {
                        t.add(id, name);
                    }
                }
            }
            Command::ControlState { id, rgb, attrs, text, verbs } => {
                if let Some(t) = self.table_of(id).and_then(|h| table::of(h)) {
                    t.set(id, rgb, attrs, text, verbs);
                }
            }
            Command::ControlRemove { id } => {
                if let Some(t) = self.table_of(id).and_then(|h| table::of(h)) {
                    t.remove(id);
                }
            }
            Command::Deliver { tab, op, full, history, lines } => {
                let Some(i) = self.index(tab) else { return };
                // New output on a tab that is not in front leaves a mark
                // on its title.
                let fresh = op == quevu_ui_core::op::LIVE && !lines.is_empty();
                if let Some(v) = self.source(i).and_then(|s| view::of(s.view)) {
                    v.deliver(op, full, history, lines);
                }
                if fresh && self.current() != Some(i) && !self.tabs[i].unseen {
                    self.tabs[i].unseen = true;
                    self.retitle(i);
                }
            }
            Command::Goto { tab, lines, mark, history } => {
                if let Some(v) = self.index(tab).and_then(|i| self.source(i)).and_then(|s| view::of(s.view)) {
                    v.goto(lines, mark, history);
                }
            }
            Command::FindStatus { tab, text, resume, history } => {
                let Some(i) = self.index(tab) else { return };
                if let Some(s) = self.source(i) {
                    s.find_status = text;
                    s.find_resume = resume;
                    s.find_offer = history;
                    self.refresh_strip(i);
                }
            }
            Command::MenuAdd { id, parent, label, attrs } => {
                let popup = if attrs & attr::MENU_SUBMENU != 0 { CreatePopupMenu() } else { null_mut() };
                let cmd = self.next_cmd;
                self.next_cmd += 1;
                self.items.push(MenuItem { id, parent, label, attrs, popup, cmd });
                self.rebuild_menu(parent);
            }
            Command::MenuSet { id, label, attrs } => {
                if let Some(m) = self.items.iter_mut().find(|m| m.id == id) {
                    m.label = label;
                    // Whether an entry opens a menu is settled when it is
                    // added; the rest can change.
                    m.attrs = (attrs & !attr::MENU_SUBMENU) | (m.attrs & attr::MENU_SUBMENU);
                    let parent = m.parent;
                    self.rebuild_menu(parent);
                }
            }
            Command::MenuRemove { id } => {
                let Some(pos) = self.items.iter().position(|m| m.id == id) else { return };
                let gone = self.items.remove(pos);
                // Everything that hung from it goes with it.
                let mut dead = vec![id];
                while let Some(p) = dead.pop() {
                    let mut k = 0;
                    while k < self.items.len() {
                        if self.items[k].parent == p {
                            dead.push(self.items.remove(k).id);
                        } else {
                            k += 1;
                        }
                    }
                }
                self.rebuild_menu(gone.parent);
                if !gone.popup.is_null() {
                    // Now that nothing points at it. The menus it opened
                    // in turn are destroyed with it.
                    DestroyMenu(gone.popup);
                }
            }
            Command::Text { id, text } => {
                self.texts.set(id, text);
                self.apply_texts();
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

    /// A text changed: whatever shows it on a tab that already exists is
    /// written again.
    unsafe fn apply_texts(&mut self) {
        for i in 0..self.tabs.len() {
            if let Kind::Source(s) = &self.tabs[i].kind {
                let (edit, history, from, to) = (s.find_edit, s.history, s.from_label, s.to_label);
                self.set_cue(edit);
                SetWindowTextW(history, wide(self.texts.get(texts::HISTORY)).as_ptr());
                SetWindowTextW(from, wide(self.texts.get(texts::FROM)).as_ptr());
                SetWindowTextW(to, wide(self.texts.get(texts::TO)).as_ptr());
                self.relayout_page(i);
            }
            self.refresh_strip(i);
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

    /// Opens the menu the host gave the icon in the notification area, if
    /// it gave one. What is chosen arrives as any other menu entry.
    unsafe fn tray_popup(&self) {
        if self.tray_menu.is_null() || !self.items.iter().any(|m| m.parent == menu::TRAY) {
            return;
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Without these two calls a menu opened from the notification
        // area does not close when the user clicks elsewhere.
        SetForegroundWindow(self.hwnd);
        TrackPopupMenu(self.tray_menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, self.hwnd, null());
        PostMessageW(self.hwnd, WM_NULL, 0, 0);
    }

    /// Opens the menu of a source tab over its button and does what the
    /// user picks. The words are the host's; an entry it gave no words to
    /// is left out.
    unsafe fn tab_popup(&mut self, i: usize) {
        let Some((button, show_time)) = self.source(i).map(|s| (s.menu, s.show_time)) else { return };
        let popup = CreatePopupMenu();
        let mut pending_gap = false;
        let mut any = false;
        for group in [
            &[(texts::MENU_HOLD, ID_TOGGLE), (texts::MENU_COPY, ID_COPY)][..],
            &[(texts::MENU_FIND, ID_FIND), (texts::MENU_FIND_OLDER, ID_FIND_OLDER), (texts::MENU_FIND_NEWER, ID_FIND_NEWER)][..],
            &[(texts::MENU_TIME, ID_TIME)][..],
        ] {
            for &(text, id) in group {
                let label = self.texts.get(text);
                if label.is_empty() {
                    continue;
                }
                if pending_gap && any {
                    AppendMenuW(popup, MF_SEPARATOR, 0, null());
                }
                pending_gap = false;
                let mut flags = MF_STRING;
                if id == ID_TIME && show_time {
                    flags |= MF_CHECKED;
                }
                AppendMenuW(popup, flags, id as usize, wide(label).as_ptr());
                any = true;
            }
            pending_gap = true;
        }
        let mut rc: RECT = zeroed();
        GetWindowRect(button, &mut rc);
        let picked = if any {
            TrackPopupMenu(popup, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTALIGN | TPM_BOTTOMALIGN, rc.right, rc.top, 0, self.hwnd, null())
        } else {
            0
        };
        DestroyMenu(popup);
        if picked > 0 {
            self.source_command(i, picked as u32);
        }
    }

    /// Asks the host for one step of a search in a console. A search
    /// never runs here, and never runs to the end of the log by itself:
    /// each step is bounded, and going on is the user's choice.
    unsafe fn find(&mut self, i: usize, newer: bool) {
        let id = self.tabs[i].id;
        let looking = self.texts.get(texts::LOOKING).to_string();
        let Some(s) = self.source(i) else { return };
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(s.find_edit, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let text = String::from_utf16_lossy(&buf[..n]);
        if text.trim().is_empty() {
            SetFocus(s.find_edit);
            return;
        }
        // The range counts only while its row is open: a bound nobody
        // can see must not narrow a search behind the user's back.
        let (since, until) = if s.range_open {
            let until = picked(s.to_pick);
            // The picker counts in seconds: the whole second is inside.
            (picked(s.from_pick), if until != 0 { until + 999_999_999 } else { 0 })
        } else {
            (0, 0)
        };
        // The same question again after a step that ran out of budget
        // goes on from where that step stopped.
        let same = s.last_query == text && s.last_newer == newer && s.last_since == since && s.last_until == until;
        s.last_since = since;
        s.last_until = until;
        let from = if s.find_resume != 0 && same {
            s.find_resume
        } else {
            view::of(s.view).map(|v| v.find_from(newer)).unwrap_or(0)
        };
        s.last_query = text.clone();
        s.last_newer = newer;
        s.find_resume = 0;
        s.find_status = looking.replacen("%s", &text, 1);
        s.find_offer = false;
        // The history is searched only if the user opened it here, by
        // scrolling into it or by accepting it for a search.
        let history = s.find_history || view::of(s.view).map(|v| v.in_history()).unwrap_or(false);
        let op = newer as u32 | if history { 2 } else { 0 };
        self.shared.set_find(id, FindParams { text, since_ns: since, until_ns: until });
        self.shared.emit(Event { kind: event::CONSOLE_FIND, id, op, n: 0, seq: from });
        self.refresh_strip(i);
    }

    /// Something asked of the source tab at `i`: by a button of its bar,
    /// an entry of its menu or a key.
    unsafe fn source_command(&mut self, i: usize, id: u32) {
        let Some(s) = self.source(i) else { return };
        let (view, find_edit, find_older, find_newer, from_pick) = (s.view, s.find_edit, s.find_older, s.find_newer, s.from_pick);
        match id {
            ID_TABMENU => self.tab_popup(i),
            ID_FIND => {
                SetFocus(find_edit);
                SendMessageW(find_edit, EM_SETSEL, 0, -1);
            }
            ID_MORE => {
                // Folds the rest of the search out, or away.
                let open = !s.range_open;
                s.range_open = open;
                for h in [s.from_label, s.from_pick, s.to_label, s.to_pick] {
                    ShowWindow(h, if open { SW_SHOW } else { SW_HIDE });
                }
                self.relayout_page(i);
                SetFocus(if open { from_pick } else { view });
            }
            ID_HISTORY => {
                // The user asks for the history: of the search that just
                // ran out, or of the output on screen.
                if s.find_offer {
                    s.find_history = true;
                    s.find_offer = false;
                    let newer = s.last_newer;
                    self.find(i, newer);
                } else if let Some(v) = view::of(view) {
                    v.request_history();
                }
                SetFocus(view);
            }
            ID_FIND_OLDER | ID_FIND_NEWER => {
                self.find(i, id == ID_FIND_NEWER);
                // A click leaves the keyboard on the button; it goes back
                // to the console, where the next key is expected.
                let focus = GetFocus();
                if focus == find_older || focus == find_newer {
                    SetFocus(view);
                }
            }
            // Enter and Escape reach the window as commands. They mean
            // something only while the search box has the keyboard.
            ID_ENTER if GetFocus() == find_edit => {
                self.find(i, GetKeyState(VK_SHIFT as i32) < 0);
            }
            ID_ESCAPE if GetFocus() == find_edit => {
                SetFocus(view);
            }
            ID_TIME => {
                s.show_time = !s.show_time;
                let on = s.show_time;
                if let Some(v) = view::of(view) {
                    v.set_show_time(on);
                }
            }
            ID_TOGGLE | ID_COPY => {
                if let Some(v) = view::of(view) {
                    if id == ID_COPY {
                        v.copy();
                    } else if v.status() == Status::Following {
                        v.pause();
                    } else {
                        v.follow();
                    }
                }
                SetFocus(view);
            }
            _ => {}
        }
    }

    /// A control on a page said something.
    unsafe fn page_command(&mut self, i: usize, id: u32, ctl: HWND) {
        let tab = self.tabs[i].id;
        match &self.tabs[i].kind {
            Kind::Source(_) => self.source_command(i, id),
            Kind::Controls(c) => {
                // A verb of the tab: the host hears which one, and
                // nothing changes here.
                if let Some(k) = c.buttons.iter().position(|&b| b == ctl) {
                    self.shared.emit(Event { kind: event::TAB_VERB, id: tab, op: c.verbs[k].id, n: 0, seq: 0 });
                }
            }
        }
    }

    /// A command with no control behind it: an entry of one of the host's
    /// menus, or a key that belongs to the tab in front.
    unsafe fn command(&mut self, id: u32) {
        if id >= ID_MENU_BASE {
            if let Some(m) = self.items.iter().find(|m| m.cmd == id) {
                self.shared.emit(Event::simple(event::MENU, m.id));
            }
            return;
        }
        if let Some(i) = self.current() {
            self.source_command(i, id);
        }
    }

    /// The new scale of the monitor reaches everything that has a font.
    unsafe fn rescale(&mut self) {
        self.make_fonts();
        SendMessageW(self.tabs_ctl, WM_SETFONT, self.font_ui as usize, 1);
        let (ui, icon, mono, cell, dpi) = (self.font_ui, self.font_icon, self.font_mono, self.cell, self.dpi);
        for t in &self.tabs {
            match &t.kind {
                Kind::Source(s) => {
                    for h in [s.strip, s.find_edit, s.history, s.from_label, s.from_pick, s.to_label, s.to_pick] {
                        SendMessageW(h, WM_SETFONT, ui as usize, 1);
                    }
                    for h in [s.toggle, s.menu, s.find_older, s.find_newer, s.find_more] {
                        SendMessageW(h, WM_SETFONT, icon as usize, 1);
                    }
                    if let Some(v) = view::of(s.view) {
                        v.set_font(mono, cell.0, cell.1);
                    }
                }
                Kind::Controls(c) => {
                    SendMessageW(c.strip, WM_SETFONT, ui as usize, 1);
                    for &b in &c.buttons {
                        SendMessageW(b, WM_SETFONT, ui as usize, 1);
                    }
                    if let Some(tb) = table::of(c.table) {
                        tb.set_font(ui, dpi);
                    }
                }
            }
        }
    }
}

/// The window a tab holds does nothing by itself: what its controls say
/// goes up to the main window, which knows the tabs.
unsafe extern "system" fn page_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND | WM_NOTIFY | WM_CTLCOLORSTATIC => SendMessageW(GetParent(hwnd), msg, wparam, lparam),
        WM_QV_STATE => {
            PostMessageW(GetParent(hwnd), msg, wparam, lparam);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCDESTROY {
        if let Some(app) = take_state::<App>(hwnd) {
            DeleteObject(app.font_ui);
            DeleteObject(app.font_mono);
            DeleteObject(app.font_icon);
            // A menu that is not on the window is not destroyed with it.
            if !app.tray_menu.is_null() {
                DestroyMenu(app.tray_menu);
            }
            if !app.bar.is_null() && !app.items.iter().any(|m| m.parent == menu::BAR) {
                DestroyMenu(app.bar);
            }
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
            if let Some(i) = app.index(wparam as u32) {
                app.refresh_strip(i);
            }
            0
        }
        WM_QV_TRAY => {
            match loword(lparam as usize) {
                WM_LBUTTONDBLCLK => app.restore(),
                WM_RBUTTONUP => app.tray_popup(),
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
            app.rescale();
            // The system suggests where the window should go at the new
            // scale.
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            app.layout();
            0
        }
        WM_COMMAND => {
            let id = loword(wparam);
            if lparam != 0 {
                // A control: a click on a button of the bar of a tab.
                // What a search box or a picker says in passing is not a
                // command.
                let ctl = lparam as HWND;
                let page = GetParent(ctl);
                if hiword(wparam) == 0 && id != 0 {
                    if let Some(i) = app.tabs.iter().position(|t| t.page == page) {
                        app.page_command(i, id, ctl);
                    }
                }
            } else {
                app.command(id);
            }
            0
        }
        WM_NOTIFY => {
            let hdr = &*(lparam as *const NMHDR);
            if hdr.hwndFrom == app.tabs_ctl && hdr.code == TCN_SELCHANGE {
                let sel = SendMessageW(app.tabs_ctl, TCM_GETCURSEL, 0, 0);
                if sel >= 0 && (sel as usize) < app.tabs.len() {
                    app.front = Some(app.tabs[sel as usize].id);
                    app.show_current();
                    if let Some(view) = app.source(sel as usize).map(|s| s.view) {
                        SetFocus(view);
                    }
                }
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            // A label takes the color of the bar it sits on.
            SetBkMode(wparam as _, TRANSPARENT as _);
            GetSysColorBrush(COLOR_BTNFACE) as LRESULT
        }
        WM_CLOSE => {
            // Closing is the host's decision: it hears the request and,
            // when it sees fit, tells this window to go.
            app.shared.emit(Event::simple(event::CLOSE, 0));
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
