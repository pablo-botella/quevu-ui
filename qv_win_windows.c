/*
 * qv_win - the window.
 *
 * One window with a row of tabs, and nothing else of its own. Every tab
 * is opened by the host, which gives it its name and its weight, and is a
 * whole page: what it shows and, under it, a bar with its tools and its
 * status. There are two kinds:
 *
 * - a source tab holds a pseudo-console over a log, with the search and
 *   the button that holds the view;
 * - a control tab holds a table of controls, each with a light, a name,
 *   the text of its state and the buttons of its verbs.
 *
 * The menu bar and the menu of the icon in the notification area are the
 * host's too. The window has no word of any language and decides nothing:
 * what the user does leaves as an event, and the window goes away only
 * when the host says so.
 *
 * Everything here runs on the thread that called qv_win_run. The host
 * reaches it only through the two lanes of qv_shared: commands arrive as
 * a posted message that makes this thread drain the queue, and what the
 * user does leaves as events.
 */
#include <stdlib.h>
#include <string.h>

#include "qv_win.h"

#ifndef WM_DPICHANGED
#define WM_DPICHANGED 0x02E0
#endif

#define MAIN_CLASS L"QuevuWindow"
/*
 * What a tab holds: a window of its own that carries the content of the
 * tab and its bar, and passes what happens on them up to the main window.
 */
#define PAGE_CLASS L"QuevuPage"

/* Pause and play in the symbol font of the system (Segoe MDL2 Assets). */
#define GLYPH_PAUSE L"\xe769"
#define GLYPH_PLAY L"\xe768"
/* Chevrons, for walking through the matches of a search. */
#define GLYPH_UP L"\xe70e"
#define GLYPH_DOWN L"\xe70d"
/* "More": opens the rest of what a search can be given. */
#define GLYPH_MORE L"\xe712"
/* The three lines that open a menu. */
#define GLYPH_MENU L"\xe700"

/*
 * Command identifiers of a source tab. Its buttons, the entries of its
 * menu and the keys that do the same thing share one.
 */
#define ID_TOGGLE 110
#define ID_COPY 111
#define ID_TIME 113
#define ID_FIND 114
#define ID_FIND_OLDER 115
#define ID_FIND_NEWER 116
#define ID_HISTORY 117
#define ID_MORE 118
#define ID_TABMENU 119
/* Every button of the bar of a control tab: which one, its handle tells. */
#define ID_TAB_VERB 120
/* What a dialog-style window gets for Enter and Escape. */
#define ID_ENTER 1
#define ID_ESCAPE 2
/* The entries of the host's menus count up from here. */
#define ID_MENU_BASE 5000

/*
 * What a source tab holds besides its console: its bar, and where its
 * search stands.
 */
typedef struct source {
    HWND view, strip, toggle, menu;
    HWND find_edit, find_older, find_newer, find_more;
    /* The rest of the search: a range of time, in a row that folds away. */
    int  range_open;
    HWND from_label, from_pick, to_label, to_pick;
    HWND history;
    int  show_time;       /* each line shows when it entered the log     */
    WCHAR   *find_status; /* how the last step of a search went          */
    uint64_t find_resume; /* where the next step starts, if asked again  */
    WCHAR   *last_query;  /* and what "again" means                      */
    int      last_newer;
    int      find_offer;   /* a search ran out of recent output          */
    int      find_history; /* the user allowed searches into the history */
    int64_t  last_since, last_until;
    int      was_following;
} source;

/* What a control tab holds: the table, and the verbs of the tab itself. */
typedef struct controls {
    HWND     table, strip;
    qv_verbs verbs;
    HWND    *buttons;
} controls;

typedef struct tab {
    uint32_t id;
    WCHAR   *title;
    int32_t  weight;
    HWND     page;
    WCHAR   *note;   /* what the host has to say in the status strip     */
    int      unseen; /* output arrived while another tab was in front    */
    int      is_source;
    source   s;
    controls c;
} tab;

/* An entry of one of the host's menus. */
typedef struct menu_item {
    uint32_t id, parent;
    WCHAR   *label;
    uint32_t attrs;
    HMENU    popup; /* the menu this entry opens, if it opens one        */
    UINT     cmd;   /* the number Windows sends back when it is chosen   */
} menu_item;

typedef struct app {
    HWND      hwnd;
    HINSTANCE hinst;
    WCHAR    *title;
    UINT      dpi;
    HFONT     font_ui, font_mono, font_icon;
    int       cw, ch;
    HWND      tabs_ctl;
    tab      *tabs; /* in the order they are shown */
    size_t    ntabs, tabs_cap;
    uint32_t  front; /* the tab in front, by its identifier; 0: none */
    WCHAR    *texts[QV_TEXT_COUNT];
    HMENU     bar, tray_menu;
    menu_item *items;
    size_t    nitems, items_cap;
    UINT      next_cmd;
    int       tray, tray_shown;
} app;

/* ---- helpers ---- */

int qv_text_width(HWND hwnd, HFONT font, const WCHAR *text)
{
    SIZE    size = {0, 0};
    HDC     dc;
    HGDIOBJ old;
    int     n = (int)qv_wlen(text);

    if (n == 0)
        return 0;
    dc = GetDC(hwnd);
    old = SelectObject(dc, font);
    GetTextExtentPoint32W(dc, text, n, &size);
    SelectObject(dc, old);
    ReleaseDC(hwnd, dc);
    return size.cx;
}

int qv_button_width(HWND hwnd, HFONT font, const WCHAR *label, UINT dpi)
{
    /* Never less than a button is expected to be. */
    int w = qv_text_width(hwnd, font, label) + qv_scale(24, dpi);
    int least = qv_scale(64, dpi);
    return w > least ? w : least;
}

HWND qv_button(HWND parent, HINSTANCE hinst, HFONT font, const WCHAR *label, int disabled, UINT id)
{
    DWORD style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON | (disabled ? WS_DISABLED : 0);
    HWND  h = CreateWindowExW(0, L"BUTTON", label, style, 0, 0, 0, 0, parent, (HMENU)(UINT_PTR)id, hinst, NULL);
    SendMessageW(h, WM_SETFONT, (WPARAM)font, 1);
    return h;
}

static const WCHAR *ws(const WCHAR *s)
{
    return s ? s : L"";
}

static int wempty(const WCHAR *s)
{
    return !s || !*s;
}

/* Replaces an owned string with another the caller hands over. */
static void wset(WCHAR **dst, WCHAR *owned)
{
    free(*dst);
    *dst = owned;
}

/* Takes the text out of a command. */
static WCHAR *take_text(qv_cmd *c)
{
    WCHAR *t = c->text ? c->text : qv_wdup(L"");
    c->text = NULL;
    return t;
}

static WCHAR *wjoin(const WCHAR *a, const WCHAR *b, const WCHAR *c)
{
    size_t la = qv_wlen(a), lb = qv_wlen(b), lc = qv_wlen(c);
    WCHAR *out = qv_alloc((la + lb + lc + 1) * sizeof *out);
    memcpy(out, a, la * sizeof *out);
    memcpy(out + la, b, lb * sizeof *out);
    memcpy(out + la + lb, c, lc * sizeof *out);
    return out;
}

static const WCHAR *text_of(const app *a, uint32_t id)
{
    return id < QV_TEXT_COUNT ? ws(a->texts[id]) : L"";
}

static int sc(const app *a, int v)
{
    return qv_scale(v, a->dpi);
}

/* A child of parent in the font of the user interface. */
static HWND child(const app *a, HWND parent, const WCHAR *class, const WCHAR *text, DWORD style, UINT id)
{
    HWND h = CreateWindowExW(0, class, text, WS_CHILD | WS_VISIBLE | style, 0, 0, 0, 0, parent,
                             (HMENU)(UINT_PTR)id, a->hinst, NULL);
    SendMessageW(h, WM_SETFONT, (WPARAM)a->font_ui, 1);
    return h;
}

/* A button of a bar that shows a symbol instead of words. */
static HWND glyph(const app *a, HWND parent, const WCHAR *symbol, UINT id)
{
    HWND h = child(a, parent, L"BUTTON", symbol, BS_PUSHBUTTON | WS_TABSTOP, id);
    SendMessageW(h, WM_SETFONT, (WPARAM)a->font_icon, 1);
    return h;
}

/* The window a tab holds, hidden until the tab is in front. */
static HWND new_page(const app *a)
{
    return CreateWindowExW(WS_EX_CONTROLPARENT, PAGE_CLASS, NULL, WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                           0, 0, 0, 0, a->hwnd, NULL, a->hinst, NULL);
}

static void make_fonts(app *a)
{
    NONCLIENTMETRICSW ncm = {0};
    TEXTMETRICW tm;
    HDC     dc;
    HGDIOBJ old;
    int     height = -(10 * (int)a->dpi / 72); /* 10 points */

    if (a->font_ui)
        DeleteObject(a->font_ui);
    if (a->font_mono)
        DeleteObject(a->font_mono);
    if (a->font_icon)
        DeleteObject(a->font_icon);
    /* The font the system uses for messages, at this monitor's size. */
    ncm.cbSize = sizeof ncm;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &ncm, 0, a->dpi);
    a->font_ui = CreateFontIndirectW(&ncm.lfMessageFont);
    a->font_mono = CreateFontW(height, 0, 0, 0, FW_NORMAL, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY,
                               FIXED_PITCH | FF_MODERN, L"Consolas");
    /*
     * The symbol font of the system, for the buttons of a bar that need
     * no words: the pause and play marks of a tape deck, the chevrons of
     * a search.
     */
    a->font_icon = CreateFontW(height, 0, 0, 0, FW_NORMAL, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0,
                               L"Segoe MDL2 Assets");

    dc = GetDC(a->hwnd);
    old = SelectObject(dc, a->font_mono);
    GetTextMetricsW(dc, &tm);
    SelectObject(dc, old);
    ReleaseDC(a->hwnd, dc);
    a->cw = tm.tmAveCharWidth > 1 ? tm.tmAveCharWidth : 1;
    a->ch = tm.tmHeight + tm.tmExternalLeading > 1 ? tm.tmHeight + tm.tmExternalLeading : 1;
}

/* The position of a tab in the row, by its identifier; -1 if not there. */
static int tab_index(const app *a, uint32_t id)
{
    size_t i;
    for (i = 0; i < a->ntabs; i++)
        if (a->tabs[i].id == id)
            return (int)i;
    return -1;
}

/* The tab in front, if there is any. */
static int current(const app *a)
{
    return a->front ? tab_index(a, a->front) : -1;
}

/* The source tab at a position, if that is what is there. */
static source *source_at(app *a, int i)
{
    if (i < 0 || (size_t)i >= a->ntabs || !a->tabs[i].is_source)
        return NULL;
    return &a->tabs[i].s;
}

/* The table that holds a control. */
static qv_table *table_of(app *a, uint32_t control)
{
    size_t i;
    for (i = 0; i < a->ntabs; i++)
        if (!a->tabs[i].is_source) {
            qv_table *t = qv_table_of(a->tabs[i].c.table);
            if (t && qv_table_has(t, control))
                return t;
        }
    return NULL;
}

static void set_tab_text(const app *a, UINT msg, size_t i, const WCHAR *text)
{
    TCITEMW item = {0};
    item.mask = TCIF_TEXT;
    item.pszText = (WCHAR *)text;
    SendMessageW(a->tabs_ctl, msg, i, (LPARAM)&item);
}

/*
 * Writes the title of a tab, with a mark if it holds output nobody has
 * looked at.
 */
static void retitle(const app *a, size_t i)
{
    const tab *t = &a->tabs[i];
    if (t->unseen) {
        WCHAR *label = wjoin(t->title, L" \x2022", L"");
        set_tab_text(a, TCM_SETITEMW, i, label);
        free(label);
    } else {
        set_tab_text(a, TCM_SETITEMW, i, t->title);
    }
}

/* Makes the row of tabs agree with which tab is in front. */
static void sync_front(const app *a)
{
    SendMessageW(a->tabs_ctl, TCM_SETCURSEL, (WPARAM)(LONG_PTR)current(a), 0);
}

/*
 * Brings the status strip of a tab up to date. Each tab has its own, so
 * it is kept right whether the tab is in front or not.
 */
static void refresh_strip(app *a, int i)
{
    tab     *t;
    source  *s;
    qv_view *v;
    int      status, following, scroll_offer;
    const WCHAR *text, *symbol;
    WCHAR   *line;

    if (i < 0 || (size_t)i >= a->ntabs)
        return;
    t = &a->tabs[i];
    if (!t->is_source) {
        /* A table of controls says only what the host has to say. */
        SetWindowTextW(t->c.strip, ws(t->note));
        return;
    }
    s = &t->s;
    v = qv_view_of(s->view);
    status = v ? qv_view_status(v) : QV_WAITING;
    following = status == QV_FOLLOWING;
    if (following && !s->was_following) {
        /*
         * Back on the live output: the search is over, and so is any
         * leave to look into the history. Only on the way back: a search
         * made while following has things to say too.
         */
        wset(&s->find_status, NULL);
        s->find_resume = 0;
        s->find_offer = 0;
        s->find_history = 0;
    }
    s->was_following = following;
    scroll_offer = v ? qv_view_history_offered(v) : 0;
    ShowWindow(s->history, scroll_offer || s->find_offer ? SW_SHOW : SW_HIDE);
    /*
     * The button shows what pressing it does, as on a tape deck. The
     * text says in the host's words what state the view is in.
     */
    switch (status) {
    case QV_FOLLOWING:
        text = text_of(a, QUEVU_TEXT_FOLLOWING), symbol = GLYPH_PAUSE;
        break;
    case QV_PAUSED:
        text = text_of(a, QUEVU_TEXT_PAUSED), symbol = GLYPH_PLAY;
        break;
    case QV_PAUSED_FULL:
        text = text_of(a, QUEVU_TEXT_PAUSED_FULL), symbol = GLYPH_PLAY;
        break;
    default:
        text = text_of(a, QUEVU_TEXT_WAITING), symbol = GLYPH_PAUSE;
        break;
    }
    /* While a search is going on, the strip tells how it went. */
    if (!wempty(s->find_status))
        line = qv_wdup(s->find_status);
    else {
        const WCHAR *state = scroll_offer ? text_of(a, QUEVU_TEXT_HISTORY_OFFER) : text;
        if (wempty(t->note))
            line = qv_wdup(state);
        else
            line = wjoin(state, wempty(state) ? L"" : L"   |   ", t->note);
    }
    SetWindowTextW(s->strip, line);
    free(line);
    SetWindowTextW(s->toggle, symbol);
    EnableWindow(s->toggle, TRUE);
}

/* Shows the page of the tab in front and hides the others. */
static void show_current(app *a)
{
    int    cur = current(a);
    size_t i;

    if (cur >= 0 && a->tabs[cur].unseen) {
        a->tabs[cur].unseen = 0;
        retitle(a, (size_t)cur);
    }
    for (i = 0; i < a->ntabs; i++) {
        if ((int)i == cur) {
            SetWindowPos(a->tabs[i].page, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            ShowWindow(a->tabs[i].page, SW_SHOW);
        } else {
            ShowWindow(a->tabs[i].page, SW_HIDE);
        }
    }
    if (cur >= 0)
        refresh_strip(a, cur);
}

/* Lays out what a tab holds: its content on top and its bar under it. */
static void layout_page(app *a, size_t i, int w, int h)
{
    tab *t = &a->tabs[i];
    int  pad = sc(a, 8), bh = sc(a, 28), fb = sc(a, 26), bar_h = sc(a, 36), gap = sc(a, 2);

    if (t->is_source) {
        source *s = &t->s;
        int  range_h = s->range_open ? sc(a, 34) : 0;
        int  top = h - bar_h - range_h > 0 ? h - bar_h - range_h : 0;
        int  right = w - sc(a, 4), ry = top + sc(a, 6), pw = sc(a, 196), ph = sc(a, 24);
        int  sy, x, fw, hw, k;
        HWND right_side[2], finders[3];

        MoveWindow(s->view, 0, 0, w, top, TRUE);

        /*
         * The folding row sits between the console and the bar, under
         * the search box it belongs to.
         */
        MoveWindow(s->to_pick, right - pw, ry, pw, ph, TRUE);
        MoveWindow(s->to_label, right - pw - sc(a, 28), ry + sc(a, 3), sc(a, 24), sc(a, 18), TRUE);
        MoveWindow(s->from_pick, right - pw * 2 - sc(a, 44), ry, pw, ph, TRUE);
        MoveWindow(s->from_label, right - pw * 2 - sc(a, 86), ry + sc(a, 3), sc(a, 38), sc(a, 18), TRUE);

        /*
         * The bar, from its right end: the menu of the tab, the button
         * that holds the view, the search, the way into the history;
         * what is left is the status strip.
         */
        sy = top + range_h + (bar_h - bh) / 2;
        x = right;
        right_side[0] = s->menu, right_side[1] = s->toggle;
        for (k = 0; k < 2; k++) {
            x -= fb;
            MoveWindow(right_side[k], x, sy + sc(a, 1), fb, fb, TRUE);
            x -= gap;
        }
        x -= pad - gap;
        finders[0] = s->find_more, finders[1] = s->find_newer, finders[2] = s->find_older;
        for (k = 0; k < 3; k++) {
            x -= fb;
            MoveWindow(finders[k], x, sy + sc(a, 1), fb, fb, TRUE);
            x -= gap;
        }
        fw = sc(a, 260) < w / 3 ? sc(a, 260) : w / 3;
        x -= fw;
        MoveWindow(s->find_edit, x, sy + sc(a, 2), fw, sc(a, 24), TRUE);
        hw = qv_button_width(a->hwnd, a->font_ui, text_of(a, QUEVU_TEXT_HISTORY), a->dpi);
        if (hw < sc(a, 76))
            hw = sc(a, 76);
        x -= pad + hw;
        MoveWindow(s->history, x, sy + sc(a, 1), hw, fb, TRUE);
        x -= pad;
        MoveWindow(s->strip, sc(a, 6), sy + sc(a, 5), x - sc(a, 6) > 0 ? x - sc(a, 6) : 0, sc(a, 18), TRUE);
    } else {
        controls *c = &t->c;
        int      top, sy, x, total = 0, strip_w;
        uint32_t k;

        /*
         * A control tab with nothing to say and no verbs of its own has
         * no bar: the table takes it all.
         */
        if (c->verbs.n == 0 && wempty(t->note))
            bar_h = 0;
        top = h - bar_h > 0 ? h - bar_h : 0;
        MoveWindow(c->table, 0, 0, w, top, TRUE);
        sy = top + (bar_h - bh) / 2;
        for (k = 0; k < c->verbs.n; k++)
            total += qv_button_width(a->hwnd, a->font_ui, c->verbs.v[k].label, a->dpi) + (k ? sc(a, 6) : 0);
        x = w - sc(a, 4) - total;
        strip_w = x - pad - sc(a, 6) > 0 ? x - pad - sc(a, 6) : 0;
        for (k = 0; k < c->verbs.n; k++) {
            int bw = qv_button_width(a->hwnd, a->font_ui, c->verbs.v[k].label, a->dpi);
            MoveWindow(c->buttons[k], x, sy + sc(a, 1), bw, bar_h > 0 ? fb : 0, TRUE);
            x += bw + sc(a, 6);
        }
        MoveWindow(c->strip, sc(a, 6), sy + sc(a, 5), strip_w, bar_h > 0 ? sc(a, 18) : 0, TRUE);
    }
}

/* Lays out the page of one tab again, at the size it already has. */
static void relayout_page(app *a, size_t i)
{
    RECT rc;
    GetClientRect(a->tabs[i].page, &rc);
    layout_page(a, i, rc.right, rc.bottom);
}

static void layout(app *a)
{
    RECT   rc, area;
    int    w, h, pad = sc(a, 8), tw, th, pw, ph;
    size_t i;

    GetClientRect(a->hwnd, &rc);
    w = rc.right, h = rc.bottom;
    /* With no tab there is nothing to show, not even an empty row. */
    ShowWindow(a->tabs_ctl, a->ntabs ? SW_SHOW : SW_HIDE);

    /*
     * One row of tabs across the whole window, and under it the page of
     * the tab in front.
     */
    tw = w - pad * 2 > 0 ? w - pad * 2 : 0;
    th = h - pad * 2 > 0 ? h - pad * 2 : 0;
    MoveWindow(a->tabs_ctl, pad, pad, tw, th, TRUE);
    area.left = area.top = 0;
    area.right = tw;
    area.bottom = th;
    /* Asks the tab control how much of itself is left for content. */
    SendMessageW(a->tabs_ctl, TCM_ADJUSTRECT, FALSE, (LPARAM)&area);
    pw = area.right - area.left > 0 ? area.right - area.left : 0;
    ph = area.bottom - area.top > 0 ? area.bottom - area.top : 0;
    for (i = 0; i < a->ntabs; i++) {
        MoveWindow(a->tabs[i].page, pad + area.left, pad + area.top, pw, ph, TRUE);
        layout_page(a, i, pw, ph);
    }
    /* The pages go over the tab control, never under it. */
    SetWindowPos(a->tabs_ctl, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}

/*
 * Puts a new tab in the row where its weight says, and in front if it is
 * the first one. Returns where it went.
 */
static size_t insert_tab(app *a, tab t)
{
    int32_t *weights = qv_alloc(a->ntabs * sizeof *weights);
    size_t   i, at;

    for (i = 0; i < a->ntabs; i++)
        weights[i] = a->tabs[i].weight;
    at = qv_tab_insert_at(weights, a->ntabs, t.weight);
    free(weights);

    set_tab_text(a, TCM_INSERTITEMW, at, t.title);
    if (!a->front)
        a->front = t.id;
    if (a->ntabs == a->tabs_cap) {
        a->tabs_cap = a->tabs_cap ? a->tabs_cap * 2 : 8;
        a->tabs = qv_grow(a->tabs, a->tabs_cap * sizeof *a->tabs);
    }
    memmove(a->tabs + at + 1, a->tabs + at, (a->ntabs - at) * sizeof *a->tabs);
    a->tabs[at] = t;
    a->ntabs++;
    sync_front(a);
    layout(a);
    show_current(a);
    return at;
}

/* What the empty search box says. */
static void set_cue(const app *a, HWND edit)
{
    SendMessageW(edit, EM_SETCUEBANNER, TRUE, (LPARAM)text_of(a, QUEVU_TEXT_FIND_CUE));
}

static void add_source(app *a, uint32_t id, WCHAR *title, int32_t weight)
{
    tab     t = {0};
    source *s = &t.s;
    HWND    page = new_page(a), picks[2], folded[4];
    int     k;
    qv_view *v;

    t.id = id;
    t.title = title;
    t.weight = weight;
    t.page = page;
    t.is_source = 1;

    s->view = qv_view_create(page, a->hinst, id, a->font_mono, a->cw, a->ch);
    ShowWindow(s->view, SW_SHOW);
    s->strip = child(a, page, L"STATIC", L"", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
    /*
     * The one way into the history. It shows up only when there is a
     * history to offer, and nothing goes there until it is pressed.
     */
    s->history = child(a, page, L"BUTTON", text_of(a, QUEVU_TEXT_HISTORY), BS_PUSHBUTTON | WS_TABSTOP, ID_HISTORY);
    ShowWindow(s->history, SW_HIDE);
    /* The box a search is typed in. It needs a border of its own. */
    s->find_edit = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", NULL, WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
                                   0, 0, 0, 0, page, NULL, a->hinst, NULL);
    SendMessageW(s->find_edit, WM_SETFONT, (WPARAM)a->font_ui, 1);
    /*
     * The same two steps the keys take, for the mouse: up is older, down
     * is newer, as the text itself runs.
     */
    s->find_older = glyph(a, page, GLYPH_UP, ID_FIND_OLDER);
    s->find_newer = glyph(a, page, GLYPH_DOWN, ID_FIND_NEWER);
    /*
     * What else a search can be given lives in a row that folds away:
     * for now, a range of time. Each end has a box to tick; unticked,
     * that side has no bound.
     */
    s->find_more = glyph(a, page, GLYPH_MORE, ID_MORE);
    s->from_label = child(a, page, L"STATIC", text_of(a, QUEVU_TEXT_FROM), SS_NOPREFIX, 0);
    s->from_pick = child(a, page, DATETIMEPICK_CLASSW, L"", DTS_SHOWNONE | WS_TABSTOP, 0);
    s->to_label = child(a, page, L"STATIC", text_of(a, QUEVU_TEXT_TO), SS_NOPREFIX, 0);
    s->to_pick = child(a, page, DATETIMEPICK_CLASSW, L"", DTS_SHOWNONE | WS_TABSTOP, 0);
    picks[0] = s->from_pick, picks[1] = s->to_pick;
    for (k = 0; k < 2; k++) {
        SendMessageW(picks[k], DTM_SETFORMATW, 0, (LPARAM)L"yyyy'-'MM'-'dd  HH':'mm':'ss");
        /* Unticked to begin with: no bound. */
        SendMessageW(picks[k], DTM_SETSYSTEMTIME, GDT_NONE, 0);
    }
    folded[0] = s->from_label, folded[1] = s->from_pick, folded[2] = s->to_label, folded[3] = s->to_pick;
    for (k = 0; k < 4; k++)
        ShowWindow(folded[k], SW_HIDE);
    /*
     * The button that holds the view shows what pressing it does, as on
     * a tape deck; next to it, the one that opens the menu of the tab.
     */
    s->toggle = glyph(a, page, GLYPH_PAUSE, ID_TOGGLE);
    EnableWindow(s->toggle, FALSE);
    s->menu = glyph(a, page, GLYPH_MENU, ID_TABMENU);
    set_cue(a, s->find_edit);

    v = qv_view_of(s->view);
    insert_tab(a, t);
    /* The console asks for its first page as soon as it exists. */
    if (v)
        qv_view_follow(v);
}

static void add_controls(app *a, uint32_t id, WCHAR *title, int32_t weight)
{
    tab t = {0};
    t.id = id;
    t.title = title;
    t.weight = weight;
    t.page = new_page(a);
    t.c.table = qv_table_create(t.page, a->hinst, a->font_ui, a->dpi);
    t.c.strip = child(a, t.page, L"STATIC", L"", SS_ENDELLIPSIS | SS_NOPREFIX, 0);
    insert_tab(a, t);
}

/* Frees what a tab owns besides its windows. */
static void free_tab(tab *t)
{
    free(t->title);
    free(t->note);
    free(t->s.find_status);
    free(t->s.last_query);
    qv_verbs_free(&t->c.verbs);
    free(t->c.buttons);
}

/* ---- menus ---- */

static menu_item *find_item(app *a, uint32_t id)
{
    size_t i;
    for (i = 0; i < a->nitems; i++)
        if (a->items[i].id == id)
            return &a->items[i];
    return NULL;
}

/*
 * The handle of the menu the entries under parent hang from, made the
 * first time something hangs from it.
 */
static HMENU menu_handle(app *a, uint32_t parent)
{
    menu_item *m;
    if (parent == QUEVU_MENU_BAR) {
        if (!a->bar)
            a->bar = CreateMenu();
        return a->bar;
    }
    if (parent == QUEVU_MENU_TRAY) {
        if (!a->tray_menu)
            a->tray_menu = CreatePopupMenu();
        return a->tray_menu;
    }
    m = find_item(a, parent);
    return m ? m->popup : NULL;
}

static int has_entries(const app *a, uint32_t parent)
{
    size_t i;
    for (i = 0; i < a->nitems; i++)
        if (a->items[i].parent == parent)
            return 1;
    return 0;
}

/* Writes a menu again from the entries the host has given for it. */
static void rebuild_menu(app *a, uint32_t parent)
{
    HMENU  h = menu_handle(a, parent);
    size_t i;

    if (!h)
        return;
    /* Taking an entry out does not destroy the menu it opens. */
    while (RemoveMenu(h, 0, MF_BYPOSITION))
        ;
    for (i = 0; i < a->nitems; i++) {
        const menu_item *m = &a->items[i];
        UINT flags = MF_STRING;
        if (m->parent != parent)
            continue;
        if (m->attrs & QUEVU_MENU_SEPARATOR) {
            AppendMenuW(h, MF_SEPARATOR, 0, NULL);
            continue;
        }
        if (m->attrs & QUEVU_MENU_CHECKED)
            flags |= MF_CHECKED;
        if (m->attrs & QUEVU_MENU_DISABLED)
            flags |= MF_GRAYED;
        if (m->popup)
            AppendMenuW(h, flags | MF_POPUP, (UINT_PTR)m->popup, m->label);
        else
            AppendMenuW(h, flags, m->cmd, m->label);
    }
    /* A bar with nothing on it is not shown at all. */
    if (parent == QUEVU_MENU_BAR)
        SetMenu(a->hwnd, has_entries(a, parent) ? a->bar : NULL);
    DrawMenuBar(a->hwnd);
    layout(a);
}

static void menu_add(app *a, qv_cmd *c)
{
    menu_item *m;
    if (a->nitems == a->items_cap) {
        a->items_cap = a->items_cap ? a->items_cap * 2 : 16;
        a->items = qv_grow(a->items, a->items_cap * sizeof *a->items);
    }
    m = &a->items[a->nitems++];
    m->id = c->id;
    m->parent = c->tab;
    m->label = take_text(c);
    m->attrs = c->b;
    m->popup = (c->b & QUEVU_MENU_SUBMENU) ? CreatePopupMenu() : NULL;
    m->cmd = a->next_cmd++;
    rebuild_menu(a, m->parent);
}

/* Takes an entry out of the list, with everything that hangs from it. */
static void drop_items_under(app *a, uint32_t parent)
{
    size_t k = 0;
    while (k < a->nitems) {
        if (a->items[k].parent == parent) {
            uint32_t id = a->items[k].id;
            free(a->items[k].label);
            memmove(&a->items[k], &a->items[k + 1], (a->nitems - k - 1) * sizeof *a->items);
            a->nitems--;
            /* The menus it opened are destroyed with the one above. */
            drop_items_under(a, id);
            k = 0;
        } else {
            k++;
        }
    }
}

static void menu_remove(app *a, uint32_t id)
{
    menu_item *m = find_item(a, id), gone;
    if (!m)
        return;
    gone = *m;
    memmove(m, m + 1, (size_t)(&a->items[a->nitems] - (m + 1)) * sizeof *m);
    a->nitems--;
    drop_items_under(a, id);
    rebuild_menu(a, gone.parent);
    /* Now that nothing points at it. */
    if (gone.popup)
        DestroyMenu(gone.popup);
    free(gone.label);
}

/* ---- the notification area ---- */

static void tray_icon(app *a, int show)
{
    NOTIFYICONDATAW nid = {0};
    nid.cbSize = sizeof nid;
    nid.hWnd = a->hwnd;
    nid.uID = 1;
    if (show && !a->tray_shown) {
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_QV_TRAY;
        nid.hIcon = LoadIconW(NULL, (LPCWSTR)IDI_APPLICATION);
        lstrcpynW(nid.szTip, ws(a->title), (int)(sizeof nid.szTip / sizeof nid.szTip[0]));
        a->tray_shown = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
    } else if (!show && a->tray_shown) {
        Shell_NotifyIconW(NIM_DELETE, &nid);
        a->tray_shown = 0;
    }
}

static void restore(const app *a)
{
    ShowWindow(a->hwnd, SW_SHOW);
    ShowWindow(a->hwnd, SW_RESTORE);
    SetForegroundWindow(a->hwnd);
}

/*
 * Opens the menu the host gave the icon in the notification area, if it
 * gave one. What is chosen arrives as any other menu entry.
 */
static void tray_popup(const app *a)
{
    POINT pt = {0, 0};
    if (!a->tray_menu || !has_entries(a, QUEVU_MENU_TRAY))
        return;
    GetCursorPos(&pt);
    /*
     * Without these two calls a menu opened from the notification area
     * does not close when the user clicks elsewhere.
     */
    SetForegroundWindow(a->hwnd);
    TrackPopupMenu(a->tray_menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, a->hwnd, NULL);
    PostMessageW(a->hwnd, WM_NULL, 0, 0);
}

/* ---- what a source tab is asked ---- */

/*
 * The time a picker holds, as nanoseconds since the Unix epoch, or zero
 * if its box is not ticked.
 */
static int64_t picked(HWND h)
{
    SYSTEMTIME local, utc;
    FILETIME   ft;
    ULONGLONG  ticks;

    if (SendMessageW(h, DTM_GETSYSTEMTIME, 0, (LPARAM)&local) != GDT_VALID)
        return 0;
    /*
     * The user thinks in the time of the machine; chunks are stamped in
     * universal time.
     */
    if (!TzSpecificLocalTimeToSystemTime(NULL, &local, &utc) || !SystemTimeToFileTime(&utc, &ft))
        return 0;
    /* Windows counts in units of 100 ns from 1601. */
    ticks = ((ULONGLONG)ft.dwHighDateTime << 32) | ft.dwLowDateTime;
    return ((int64_t)ticks - 116444736000000000LL) * 100;
}

/* The text with its first "%s" replaced by what is being looked for. */
static WCHAR *looking_for(const WCHAR *format, const WCHAR *text)
{
    const WCHAR *p = format;
    WCHAR *head, *out;
    size_t n;

    while (*p && !(p[0] == L'%' && p[1] == L's'))
        p++;
    if (!*p)
        return qv_wdup(format);
    n = (size_t)(p - format);
    head = qv_alloc((n + 1) * sizeof *head);
    memcpy(head, format, n * sizeof *head);
    out = wjoin(head, text, p + 2);
    free(head);
    return out;
}

/*
 * Asks the host for one step of a search in a console. A search never
 * runs here, and never runs to the end of the log by itself: each step
 * is bounded, and going on is the user's choice.
 */
static void find(app *a, int i, int newer)
{
    source  *s = source_at(a, i);
    uint32_t id;
    WCHAR    buf[512];
    int      n, k, blank = 1, same, history;
    int64_t  since = 0, until = 0;
    uint64_t from;
    qv_view *v;

    if (!s)
        return;
    id = a->tabs[i].id;
    n = GetWindowTextW(s->find_edit, buf, 512);
    if (n < 0)
        n = 0;
    buf[n] = 0;
    for (k = 0; k < n; k++)
        if (buf[k] != L' ' && buf[k] != L'\t' && buf[k] != L'\r' && buf[k] != L'\n')
            blank = 0;
    if (blank) {
        SetFocus(s->find_edit);
        return;
    }
    /*
     * The range counts only while its row is open: a bound nobody can
     * see must not narrow a search behind the user's back.
     */
    if (s->range_open) {
        since = picked(s->from_pick);
        until = picked(s->to_pick);
        /* The picker counts in seconds: the whole second is inside. */
        if (until != 0)
            until += 999999999;
    }
    /*
     * The same question again after a step that ran out of budget goes
     * on from where that step stopped.
     */
    same = s->last_query && qv_wsame(s->last_query, buf) && s->last_newer == newer && s->last_since == since &&
           s->last_until == until;
    s->last_since = since;
    s->last_until = until;
    v = qv_view_of(s->view);
    if (s->find_resume != 0 && same)
        from = s->find_resume;
    else
        from = v ? qv_view_find_from(v, newer) : 0;
    wset(&s->last_query, qv_wdup(buf));
    s->last_newer = newer;
    s->find_resume = 0;
    wset(&s->find_status, looking_for(text_of(a, QUEVU_TEXT_LOOKING), buf));
    s->find_offer = 0;
    /*
     * The history is searched only if the user opened it here, by
     * scrolling into it or by accepting it for a search.
     */
    history = s->find_history || (v && qv_view_in_history(v));
    qv_set_find(id, qv_wide_to_utf8(buf, (size_t)n, NULL), since, until);
    qv_emit(QUEVU_EV_CONSOLE_FIND, id, (newer ? 1u : 0u) | (history ? 2u : 0u), 0, from);
    refresh_strip(a, i);
}

static void source_command(app *a, int i, UINT id);

/*
 * Opens the menu of a source tab over its button and does what the user
 * picks. The words are the host's; an entry it gave no words to is left
 * out.
 */
static void tab_popup(app *a, int i)
{
    static const struct {
        uint32_t text;
        UINT     id;
        int      group;
    } entries[] = {
        {QUEVU_TEXT_MENU_HOLD, ID_TOGGLE, 0},      {QUEVU_TEXT_MENU_COPY, ID_COPY, 0},
        {QUEVU_TEXT_MENU_FIND, ID_FIND, 1},        {QUEVU_TEXT_MENU_FIND_OLDER, ID_FIND_OLDER, 1},
        {QUEVU_TEXT_MENU_FIND_NEWER, ID_FIND_NEWER, 1}, {QUEVU_TEXT_MENU_TIME, ID_TIME, 2},
    };
    source  *s = source_at(a, i);
    uint32_t tab_id;
    HMENU    popup;
    RECT     rc;
    int      any = 0, last_group = -1, choice = 0;
    size_t   k;

    if (!s)
        return;
    tab_id = a->tabs[i].id;
    popup = CreatePopupMenu();
    for (k = 0; k < sizeof entries / sizeof entries[0]; k++) {
        const WCHAR *label = text_of(a, entries[k].text);
        UINT flags = MF_STRING;
        if (wempty(label))
            continue;
        if (any && entries[k].group != last_group)
            AppendMenuW(popup, MF_SEPARATOR, 0, NULL);
        last_group = entries[k].group;
        if (entries[k].id == ID_TIME && s->show_time)
            flags |= MF_CHECKED;
        AppendMenuW(popup, flags, entries[k].id, label);
        any = 1;
    }
    GetWindowRect(s->menu, &rc);
    if (any)
        choice = (int)TrackPopupMenu(popup, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTALIGN | TPM_BOTTOMALIGN, rc.right,
                                     rc.top, 0, a->hwnd, NULL);
    DestroyMenu(popup);
    /*
     * The window went on living while the menu was open: the tab may
     * have moved in the row, or be gone.
     */
    if (choice > 0) {
        int now = tab_index(a, tab_id);
        if (now >= 0)
            source_command(a, now, (UINT)choice);
    }
}

/*
 * Something asked of the source tab at i: by a button of its bar, an
 * entry of its menu or a key.
 */
static void source_command(app *a, int i, UINT id)
{
    source  *s = source_at(a, i);
    qv_view *v;
    HWND     focus;

    if (!s)
        return;
    v = qv_view_of(s->view);
    switch (id) {
    case ID_TABMENU:
        tab_popup(a, i);
        break;
    case ID_FIND:
        SetFocus(s->find_edit);
        SendMessageW(s->find_edit, EM_SETSEL, 0, -1);
        break;
    case ID_MORE: {
        /* Folds the rest of the search out, or away. */
        int  open = !s->range_open;
        HWND view = s->view, from_pick = s->from_pick;
        s->range_open = open;
        ShowWindow(s->from_label, open ? SW_SHOW : SW_HIDE);
        ShowWindow(s->from_pick, open ? SW_SHOW : SW_HIDE);
        ShowWindow(s->to_label, open ? SW_SHOW : SW_HIDE);
        ShowWindow(s->to_pick, open ? SW_SHOW : SW_HIDE);
        relayout_page(a, (size_t)i);
        SetFocus(open ? from_pick : view);
        break;
    }
    case ID_HISTORY: {
        /*
         * The user asks for the history: of the search that just ran
         * out, or of the output on screen.
         */
        HWND view = s->view;
        if (s->find_offer) {
            s->find_history = 1;
            s->find_offer = 0;
            find(a, i, s->last_newer);
        } else if (v) {
            qv_view_request_history(v);
        }
        SetFocus(view);
        break;
    }
    case ID_FIND_OLDER:
    case ID_FIND_NEWER:
        find(a, i, id == ID_FIND_NEWER);
        /*
         * A click leaves the keyboard on the button; it goes back to the
         * console, where the next key is expected.
         */
        focus = GetFocus();
        if (focus == s->find_older || focus == s->find_newer)
            SetFocus(s->view);
        break;
    /*
     * Enter and Escape reach the window as commands. They mean something
     * only while the search box has the keyboard.
     */
    case ID_ENTER:
        if (GetFocus() == s->find_edit)
            find(a, i, GetKeyState(VK_SHIFT) < 0);
        break;
    case ID_ESCAPE:
        if (GetFocus() == s->find_edit)
            SetFocus(s->view);
        break;
    case ID_TIME:
        s->show_time = !s->show_time;
        if (v)
            qv_view_set_show_time(v, s->show_time);
        break;
    case ID_TOGGLE:
    case ID_COPY: {
        HWND view = s->view;
        if (v) {
            if (id == ID_COPY)
                qv_view_copy(v);
            else if (qv_view_status(v) == QV_FOLLOWING)
                qv_view_pause(v);
            else
                qv_view_follow(v);
        }
        SetFocus(view);
        break;
    }
    default:
        break;
    }
}

/* A control on a page said something. */
static void page_command(app *a, int i, UINT id, HWND ctl)
{
    tab *t = &a->tabs[i];
    uint32_t k;

    if (t->is_source) {
        source_command(a, i, id);
        return;
    }
    /*
     * A verb of the tab: the host hears which one, and nothing changes
     * here.
     */
    for (k = 0; k < t->c.verbs.n; k++)
        if (t->c.buttons[k] == ctl) {
            qv_emit(QUEVU_EV_TAB_VERB, t->id, t->c.verbs.v[k].id, 0, 0);
            return;
        }
}

/*
 * A command with no control behind it: an entry of one of the host's
 * menus, or a key that belongs to the tab in front.
 */
static void command(app *a, UINT id)
{
    if (id >= ID_MENU_BASE) {
        size_t i;
        for (i = 0; i < a->nitems; i++)
            if (a->items[i].cmd == id) {
                qv_emit(QUEVU_EV_MENU, a->items[i].id, 0, 0, 0);
                return;
            }
        return;
    }
    if (current(a) >= 0)
        source_command(a, current(a), id);
}

/* ---- commands of the host ---- */

/*
 * A text changed: whatever shows it on a tab that already exists is
 * written again.
 */
static void apply_texts(app *a)
{
    size_t i;
    for (i = 0; i < a->ntabs; i++) {
        if (a->tabs[i].is_source) {
            source *s = &a->tabs[i].s;
            set_cue(a, s->find_edit);
            SetWindowTextW(s->history, text_of(a, QUEVU_TEXT_HISTORY));
            SetWindowTextW(s->from_label, text_of(a, QUEVU_TEXT_FROM));
            SetWindowTextW(s->to_label, text_of(a, QUEVU_TEXT_TO));
            relayout_page(a, i);
        }
        refresh_strip(a, (int)i);
    }
}

static void tab_verbs(app *a, qv_cmd *c)
{
    int       i = tab_index(a, c->id);
    controls *ct;
    uint32_t  k;

    if (i < 0 || a->tabs[i].is_source)
        return;
    ct = &a->tabs[i].c;
    if (!qv_verbs_equal(&ct->verbs, &c->verbs)) {
        for (k = 0; k < ct->verbs.n; k++)
            DestroyWindow(ct->buttons[k]);
        free(ct->buttons);
        qv_verbs_free(&ct->verbs);
        ct->verbs = c->verbs;
        c->verbs.v = NULL;
        c->verbs.n = 0;
        ct->buttons = qv_alloc(ct->verbs.n * sizeof *ct->buttons);
        for (k = 0; k < ct->verbs.n; k++)
            ct->buttons[k] = qv_button(a->tabs[i].page, a->hinst, a->font_ui, ct->verbs.v[k].label,
                                       (ct->verbs.v[k].attrs & QUEVU_VERB_DISABLED) != 0, ID_TAB_VERB);
    }
    relayout_page(a, (size_t)i);
}

static void tab_remove(app *a, uint32_t id)
{
    int i = tab_index(a, id);
    tab gone;

    if (i < 0)
        return;
    gone = a->tabs[i];
    memmove(&a->tabs[i], &a->tabs[i + 1], (a->ntabs - (size_t)i - 1) * sizeof *a->tabs);
    a->ntabs--;
    /* The page takes everything on it along. */
    DestroyWindow(gone.page);
    free_tab(&gone);
    SendMessageW(a->tabs_ctl, TCM_DELETEITEM, (WPARAM)i, 0);
    if (a->front == id) {
        /* The neighbour comes to the front, if there is one. */
        int next = qv_tab_front_after_remove((size_t)i, a->ntabs);
        a->front = next >= 0 ? a->tabs[next].id : 0;
    }
    sync_front(a);
    layout(a);
    show_current(a);
}

/* Does what a command says. Returns 1 if the window is gone with it. */
static int apply(app *a, qv_cmd *c)
{
    int       i;
    source   *s;
    qv_view  *v;
    qv_table *t;
    menu_item *m;

    switch (c->kind) {
    case QV_CONSOLE_ADD:
        add_source(a, c->id, take_text(c), c->weight);
        break;
    case QV_CONTROLS_ADD:
        add_controls(a, c->id, take_text(c), c->weight);
        break;
    case QV_TAB_TITLE:
        if ((i = tab_index(a, c->id)) >= 0) {
            wset(&a->tabs[i].title, take_text(c));
            retitle(a, (size_t)i);
            /* A longer name can make the row of tabs taller. */
            layout(a);
        }
        break;
    case QV_TAB_SELECT:
        if (tab_index(a, c->id) >= 0) {
            a->front = c->id;
            sync_front(a);
            show_current(a);
        }
        break;
    case QV_TAB_REMOVE:
        tab_remove(a, c->id);
        break;
    case QV_TAB_NOTE:
        if ((i = tab_index(a, c->id)) >= 0) {
            wset(&a->tabs[i].note, take_text(c));
            /* The bar of a control tab comes and goes with it. */
            relayout_page(a, (size_t)i);
            refresh_strip(a, i);
        }
        break;
    case QV_TAB_VERBS:
        tab_verbs(a, c);
        break;
    case QV_CONTROL_ADD:
        i = tab_index(a, c->tab);
        if (i >= 0 && !a->tabs[i].is_source && (t = qv_table_of(a->tabs[i].c.table)) != NULL)
            qv_table_add(t, c->id, take_text(c));
        break;
    case QV_CONTROL_STATE:
        if ((t = table_of(a, c->id)) != NULL)
            qv_table_set(t, c->id, c->a, c->b, take_text(c), &c->verbs);
        break;
    case QV_CONTROL_REMOVE:
        if ((t = table_of(a, c->id)) != NULL)
            qv_table_remove(t, c->id);
        break;
    case QV_DELIVER: {
        /*
         * New output on a tab that is not in front leaves a mark on its
         * title.
         */
        int fresh = c->a == QUEVU_OP_LIVE && c->nlines > 0;
        i = tab_index(a, c->id);
        if (i < 0)
            break;
        s = source_at(a, i);
        if (s && (v = qv_view_of(s->view)) != NULL) {
            qv_view_deliver(v, c->a, c->full, c->history, c->lines, c->nlines);
            c->nlines = 0;
        }
        if (fresh && current(a) != i && !a->tabs[i].unseen) {
            a->tabs[i].unseen = 1;
            retitle(a, (size_t)i);
        }
        break;
    }
    case QV_CONSOLE_TIME:
        if ((s = source_at(a, tab_index(a, c->id))) != NULL) {
            s->show_time = c->full;
            if ((v = qv_view_of(s->view)) != NULL)
                qv_view_set_show_time(v, c->full);
        }
        break;
    case QV_GOTO:
        if ((s = source_at(a, tab_index(a, c->id))) != NULL && (v = qv_view_of(s->view)) != NULL) {
            qv_view_goto(v, c->lines, c->nlines, c->seq, c->history);
            c->nlines = 0;
        }
        break;
    case QV_FIND_STATUS:
        i = tab_index(a, c->id);
        if ((s = source_at(a, i)) != NULL) {
            wset(&s->find_status, take_text(c));
            s->find_resume = c->seq;
            s->find_offer = c->history;
            refresh_strip(a, i);
        }
        break;
    case QV_MENU_ADD:
        menu_add(a, c);
        break;
    case QV_MENU_SET:
        if ((m = find_item(a, c->id)) != NULL) {
            wset(&m->label, take_text(c));
            /*
             * Whether an entry opens a menu is settled when it is added;
             * the rest can change.
             */
            m->attrs = (c->b & ~(uint32_t)QUEVU_MENU_SUBMENU) | (m->attrs & QUEVU_MENU_SUBMENU);
            rebuild_menu(a, m->parent);
        }
        break;
    case QV_MENU_REMOVE:
        menu_remove(a, c->id);
        break;
    case QV_TEXT:
        if (c->id < QV_TEXT_COUNT) {
            wset(&a->texts[c->id], take_text(c));
            apply_texts(a);
        }
        break;
    case QV_TRAY:
        a->tray = c->full;
        tray_icon(a, c->full);
        break;
    case QV_SHOW:
        if (c->full)
            restore(a);
        else
            ShowWindow(a->hwnd, SW_HIDE);
        break;
    case QV_QUIT:
        /* The window goes, and a with it. */
        DestroyWindow(a->hwnd);
        return 1;
    default:
        break;
    }
    return 0;
}

/* The new scale of the monitor reaches everything that has a font. */
static void rescale(app *a)
{
    size_t i;
    make_fonts(a);
    SendMessageW(a->tabs_ctl, WM_SETFONT, (WPARAM)a->font_ui, 1);
    for (i = 0; i < a->ntabs; i++) {
        tab *t = &a->tabs[i];
        if (t->is_source) {
            source  *s = &t->s;
            HWND     ui[7] = {s->strip, s->find_edit, s->history, s->from_label, s->from_pick, s->to_label, s->to_pick};
            HWND     icon[5] = {s->toggle, s->menu, s->find_older, s->find_newer, s->find_more};
            qv_view *v = qv_view_of(s->view);
            int      k;
            for (k = 0; k < 7; k++)
                SendMessageW(ui[k], WM_SETFONT, (WPARAM)a->font_ui, 1);
            for (k = 0; k < 5; k++)
                SendMessageW(icon[k], WM_SETFONT, (WPARAM)a->font_icon, 1);
            if (v)
                qv_view_set_font(v, a->font_mono, a->cw, a->ch);
        } else {
            qv_table *tb = qv_table_of(t->c.table);
            uint32_t  k;
            SendMessageW(t->c.strip, WM_SETFONT, (WPARAM)a->font_ui, 1);
            for (k = 0; k < t->c.verbs.n; k++)
                SendMessageW(t->c.buttons[k], WM_SETFONT, (WPARAM)a->font_ui, 1);
            if (tb)
                qv_table_set_font(tb, a->font_ui, a->dpi);
        }
    }
}

/* ---- window procedures ---- */

/*
 * The window a tab holds does nothing by itself: what its controls say
 * goes up to the main window, which knows the tabs.
 */
static LRESULT CALLBACK page_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    switch (msg) {
    case WM_COMMAND:
    case WM_NOTIFY:
    case WM_CTLCOLORSTATIC:
        return SendMessageW(GetParent(hwnd), msg, wparam, lparam);
    case WM_QV_STATE:
        PostMessageW(GetParent(hwnd), msg, wparam, lparam);
        return 0;
    default:
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
}

static LRESULT CALLBACK main_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    app *a = (app *)GetWindowLongPtrW(hwnd, GWLP_USERDATA);

    if (msg == WM_NCDESTROY) {
        if (a) {
            size_t i;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DeleteObject(a->font_ui);
            DeleteObject(a->font_mono);
            DeleteObject(a->font_icon);
            /* A menu that is not on the window is not destroyed with it. */
            if (a->tray_menu)
                DestroyMenu(a->tray_menu);
            if (a->bar && !has_entries(a, QUEVU_MENU_BAR))
                DestroyMenu(a->bar);
            for (i = 0; i < a->ntabs; i++)
                free_tab(&a->tabs[i]);
            free(a->tabs);
            for (i = 0; i < a->nitems; i++)
                free(a->items[i].label);
            free(a->items);
            for (i = 0; i < QV_TEXT_COUNT; i++)
                free(a->texts[i]);
            free(a->title);
            free(a);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if (!a)
        return DefWindowProcW(hwnd, msg, wparam, lparam);

    switch (msg) {
    case WM_QV_WAKE: {
        qv_cmd *c = qv_drain();
        int     gone = 0;
        while (c) {
            qv_cmd *next = c->next;
            /* A quit destroys the window: what came after it is dropped. */
            if (!gone)
                gone = apply(a, c);
            qv_cmd_free(c);
            c = next;
        }
        return 0;
    }
    case WM_QV_STATE:
        refresh_strip(a, tab_index(a, (uint32_t)wparam));
        return 0;
    case WM_QV_TRAY:
        if (LOWORD(lparam) == WM_LBUTTONDBLCLK)
            restore(a);
        else if (LOWORD(lparam) == WM_RBUTTONUP)
            tray_popup(a);
        return 0;
    case WM_SIZE:
        /* Minimizing with a tray icon means "get out of the way". */
        if (wparam == SIZE_MINIMIZED && a->tray)
            ShowWindow(hwnd, SW_HIDE);
        else
            layout(a);
        return 0;
    case WM_DPICHANGED: {
        /* The system suggests where the window should go at the new scale. */
        const RECT *r = (const RECT *)lparam;
        a->dpi = HIWORD(wparam) > 96 ? HIWORD(wparam) : 96;
        rescale(a);
        SetWindowPos(hwnd, NULL, r->left, r->top, r->right - r->left, r->bottom - r->top, SWP_NOZORDER | SWP_NOACTIVATE);
        layout(a);
        return 0;
    }
    case WM_COMMAND: {
        UINT id = LOWORD(wparam);
        if (lparam != 0) {
            /*
             * A control: a click on a button of the bar of a tab. What a
             * search box or a picker says in passing is not a command.
             */
            HWND ctl = (HWND)lparam, page = GetParent(ctl);
            if (HIWORD(wparam) == 0 && id != 0) {
                size_t i;
                for (i = 0; i < a->ntabs; i++)
                    if (a->tabs[i].page == page) {
                        page_command(a, (int)i, id, ctl);
                        break;
                    }
            }
        } else {
            command(a, id);
        }
        return 0;
    }
    case WM_NOTIFY: {
        const NMHDR *hdr = (const NMHDR *)lparam;
        if (hdr->hwndFrom == a->tabs_ctl && hdr->code == TCN_SELCHANGE) {
            LRESULT sel = SendMessageW(a->tabs_ctl, TCM_GETCURSEL, 0, 0);
            if (sel >= 0 && (size_t)sel < a->ntabs) {
                source *s;
                a->front = a->tabs[sel].id;
                show_current(a);
                if ((s = source_at(a, (int)sel)) != NULL)
                    SetFocus(s->view);
            }
        }
        return 0;
    }
    case WM_CTLCOLORSTATIC:
        /* A label takes the color of the bar it sits on. */
        SetBkMode((HDC)wparam, TRANSPARENT);
        return (LRESULT)GetSysColorBrush(COLOR_BTNFACE);
    case WM_CLOSE:
        /*
         * Closing is the host's decision: it hears the request and, when
         * it sees fit, tells this window to go.
         */
        qv_emit(QUEVU_EV_CLOSE, 0, 0, 0, 0);
        return 0;
    case WM_DESTROY:
        tray_icon(a, 0);
        PostQuitMessage(0);
        return 0;
    default:
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
}

/*
 * The modern look of the common controls (version 6 of comctl32) is
 * normally asked for in the manifest of the executable. This code cannot
 * count on the program it is built into having one, so it borrows the
 * manifest that ships inside shell32.dll for exactly this purpose and
 * activates it for the current thread only.
 */
static HANDLE themed_enter(ULONG_PTR *cookie)
{
    WCHAR    path[300];
    ACTCTXW  ctx = {0};
    HANDLE   h;
    UINT     n = GetSystemDirectoryW(path, 260);

    if (n == 0 || n >= 260)
        return INVALID_HANDLE_VALUE;
    lstrcpyW(path + n, L"\\shell32.dll");
    ctx.cbSize = sizeof ctx;
    ctx.dwFlags = ACTCTX_FLAG_RESOURCE_NAME_VALID;
    ctx.lpSource = path;
    ctx.lpResourceName = MAKEINTRESOURCEW(124);
    h = CreateActCtxW(&ctx);
    if (h == INVALID_HANDLE_VALUE || h == NULL)
        return INVALID_HANDLE_VALUE;
    if (!ActivateActCtx(h, cookie)) {
        ReleaseActCtx(h);
        return INVALID_HANDLE_VALUE;
    }
    return h;
}

static void themed_leave(HANDLE h, ULONG_PTR cookie)
{
    if (h == INVALID_HANDLE_VALUE)
        return;
    DeactivateActCtx(0, cookie);
    ReleaseActCtx(h);
}

int qv_win_run(const uint16_t *title)
{
    INITCOMMONCONTROLSEX icc;
    WNDCLASSEXW wc = {0};
    HINSTANCE   hinst = GetModuleHandleW(NULL);
    ULONG_PTR   cookie = 0;
    HANDLE      themed;
    HWND        hwnd;
    app        *a;
    qv_cmd     *c;
    UINT        dpi;
    MSG         msg;

    /*
     * This code must not decide for the whole process how it looks, so
     * both settings below are scoped: the DPI awareness to this thread,
     * and the modern look of the controls to an activation context that
     * is active only while this function runs.
     */
    SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    themed = themed_enter(&cookie);

    icc.dwSize = sizeof icc;
    icc.dwICC = ICC_TAB_CLASSES | ICC_DATE_CLASSES;
    InitCommonControlsEx(&icc);

    wc.cbSize = sizeof wc;
    wc.lpfnWndProc = main_proc;
    wc.hInstance = hinst;
    wc.hIcon = LoadIconW(NULL, (LPCWSTR)IDI_APPLICATION);
    wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
    wc.hbrBackground = GetSysColorBrush(COLOR_BTNFACE);
    wc.lpszClassName = MAIN_CLASS;
    RegisterClassExW(&wc);
    wc.lpfnWndProc = page_proc;
    wc.hIcon = NULL;
    wc.lpszClassName = PAGE_CLASS;
    RegisterClassExW(&wc);
    qv_view_register(hinst);
    qv_table_register(hinst);

    hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, MAIN_CLASS, title, WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                           CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, NULL, NULL, hinst, NULL);
    if (!hwnd) {
        themed_leave(themed, cookie);
        return 1;
    }

    a = qv_alloc(sizeof *a);
    a->hwnd = hwnd;
    a->hinst = hinst;
    a->title = qv_wdup(title);
    a->dpi = GetDpiForWindow(hwnd);
    if (a->dpi < 96)
        a->dpi = 96;
    a->cw = 8, a->ch = 16;
    a->next_cmd = ID_MENU_BASE;
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (LONG_PTR)a);
    /* What is applied below already has things to tell the host. */
    qv_begin();

    /*
     * The window starts with the row of tabs and nothing in it: no tab,
     * no menu. Both are the host's to add.
     */
    make_fonts(a);
    a->tabs_ctl = CreateWindowExW(0, WC_TABCONTROLW, NULL, WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP, 0, 0, 0, 0, hwnd,
                                  NULL, hinst, NULL);
    SendMessageW(a->tabs_ctl, WM_SETFONT, (WPARAM)a->font_ui, 1);
    layout(a);

    /*
     * Whatever the host queued before the window existed is applied
     * before the window is shown: it opens with its tabs and its menu,
     * not empty and then filling up.
     */
    c = qv_drain();
    while (c) {
        qv_cmd *next = c->next;
        /*
         * A quit asked for while no window was open does nothing: it
         * must not close a window that came afterwards.
         */
        if (c->kind != QV_QUIT)
            apply(a, c);
        qv_cmd_free(c);
        c = next;
    }
    /* A sensible first size, in the units of this monitor. */
    dpi = GetDpiForWindow(hwnd);
    if (dpi < 96)
        dpi = 96;
    SetWindowPos(hwnd, NULL, 0, 0, qv_scale(1040, dpi), qv_scale(660, dpi), SWP_NOMOVE | SWP_NOZORDER);
    ShowWindow(hwnd, SW_SHOW);

    /* From here on the host can reach this thread. */
    qv_attach(hwnd);
    qv_emit(QUEVU_EV_READY, 0, 0, 0, 0);
    /* Whatever the host queued in the meantime. */
    PostMessageW(hwnd, WM_QV_WAKE, 0, 0);

    while (GetMessageW(&msg, NULL, 0, 0) > 0) {
        /*
         * The keys of the search work wherever the keyboard is: in the
         * console, in the search box, on a tab. They are taken here, on
         * the way in, so no control has to know about them.
         */
        if (msg.message == WM_KEYDOWN) {
            int  shift = GetKeyState(VK_SHIFT) < 0, ctrl = GetKeyState(VK_CONTROL) < 0;
            UINT id = 0;
            if (msg.wParam == VK_F3)
                id = shift ? ID_FIND_NEWER : ID_FIND_OLDER;
            else if (ctrl && msg.wParam == 'F')
                id = ID_FIND;
            if (id) {
                PostMessageW(hwnd, WM_COMMAND, id, 0);
                continue;
            }
        }
        /* Lets Tab walk through the buttons like in a dialog. */
        if (!IsDialogMessageW(hwnd, &msg)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    qv_detach();
    themed_leave(themed, cookie);
    return 0;
}
