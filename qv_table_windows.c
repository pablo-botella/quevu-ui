/*
 * qv_table - the table of a control tab, as a Win32 control of its own.
 *
 * One row per control, in columns that line up: a light, the name, the
 * text of its state and the buttons of its verbs. The table knows nothing
 * of what any of it means. The host says what color the light is, what
 * the text says and which verbs there are; a button pressed goes back to
 * the host as an event and changes nothing here.
 */
#include <stdlib.h>
#include <string.h>

#include "qv_win.h"

#define TABLE_CLASS L"QuevuTable"

/* How often a light that blinks changes, in milliseconds. */
#define BLINK_MS 500
#define TIMER_BLINK 1

/*
 * A light that is on and one that is off, in the symbol font of the
 * system: a black circle and a white one.
 */
#define LIGHT_ON 0x25cf
#define LIGHT_OFF 0x25cb

typedef struct row {
    uint32_t id;
    WCHAR   *name;
    uint32_t rgb; /* as the host gives it: 0x00RRGGBB */
    uint32_t attrs;
    WCHAR   *text;
    qv_verbs verbs;
    HWND    *buttons; /* one per verb */
} row;

struct qv_table {
    HWND      hwnd;
    HINSTANCE hinst;
    HFONT     font;
    HFONT     font_light; /* the lights are drawn in it; the table's own */
    UINT      dpi;
    row      *rows;
    size_t    nrows, cap;
    int       scroll;   /* how far down the table is scrolled, in pixels */
    int       dark;     /* the half of the time a blinking light is off  */
    int       blinking;
    int       name_w;   /* width of the column of names                  */
    int       verbs_w;  /* width of the column of verbs                  */
};

/* What the creator of a table hands over through CreateWindowEx. */
typedef struct seed {
    HFONT font;
    UINT  dpi;
} seed;

static LRESULT CALLBACK table_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);
static void arrange(qv_table *t);

void qv_table_register(HINSTANCE hinst)
{
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    wc.lpfnWndProc = table_proc;
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
    wc.lpszClassName = TABLE_CLASS;
    RegisterClassExW(&wc);
}

HWND qv_table_create(HWND parent, HINSTANCE hinst, HFONT font, UINT dpi)
{
    seed *s = qv_alloc(sizeof *s);
    s->font = font;
    s->dpi = dpi;
    return CreateWindowExW(WS_EX_CONTROLPARENT, TABLE_CLASS, NULL,
                           WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                           0, 0, 0, 0, parent, NULL, hinst, s);
}

qv_table *qv_table_of(HWND hwnd)
{
    return (qv_table *)GetWindowLongPtrW(hwnd, GWLP_USERDATA);
}

static int sc(const qv_table *t, int v)
{
    return qv_scale(v, t->dpi);
}

static int row_h(const qv_table *t)
{
    return sc(t, 34);
}

static int find_row(const qv_table *t, uint32_t id)
{
    size_t i;
    for (i = 0; i < t->nrows; i++)
        if (t->rows[i].id == id)
            return (int)i;
    return -1;
}

int qv_table_has(const qv_table *t, uint32_t id)
{
    return find_row(t, id) >= 0;
}

static void client(const qv_table *t, int *w, int *h)
{
    RECT rc;
    GetClientRect(t->hwnd, &rc);
    *w = rc.right;
    *h = rc.bottom;
}

static void destroy_buttons(row *r)
{
    uint32_t i;
    for (i = 0; i < r->verbs.n; i++)
        DestroyWindow(r->buttons[i]);
    free(r->buttons);
    r->buttons = NULL;
}

/* Keeps the timer of the lights running only while a light blinks. */
static void watch_blink(qv_table *t)
{
    size_t i;
    int    any = 0;
    for (i = 0; i < t->nrows; i++)
        if (t->rows[i].attrs & QUEVU_LIGHT_BLINK)
            any = 1;
    if (any && !t->blinking) {
        SetTimer(t->hwnd, TIMER_BLINK, BLINK_MS, NULL);
    } else if (!any && t->blinking) {
        KillTimer(t->hwnd, TIMER_BLINK);
        t->dark = 0;
    }
    t->blinking = any;
}

/*
 * A new row, at the end. It shows its name and nothing else until the
 * host says what state it is in.
 */
void qv_table_add(qv_table *t, uint32_t id, WCHAR *name)
{
    row *r;
    if (t->nrows == t->cap) {
        t->cap = t->cap ? t->cap * 2 : 16;
        t->rows = qv_grow(t->rows, t->cap * sizeof *t->rows);
    }
    r = &t->rows[t->nrows++];
    memset(r, 0, sizeof *r);
    r->id = id;
    r->name = name;
    r->attrs = QUEVU_LIGHT_HOLLOW;
    r->text = qv_alloc(sizeof *r->text);
    arrange(t);
}

void qv_table_remove(qv_table *t, uint32_t id)
{
    int  i = find_row(t, id);
    row *r;
    if (i < 0)
        return;
    r = &t->rows[i];
    destroy_buttons(r);
    qv_verbs_free(&r->verbs);
    free(r->name);
    free(r->text);
    memmove(r, r + 1, (t->nrows - (size_t)i - 1) * sizeof *r);
    t->nrows--;
    arrange(t);
}

/*
 * A control changed. The buttons are made again only when the verbs are
 * not the ones already there: a control that only changes its text keeps
 * its buttons, and whichever of them has the keyboard.
 */
void qv_table_set(qv_table *t, uint32_t id, uint32_t rgb, uint32_t attrs, WCHAR *text, qv_verbs *verbs)
{
    int  i = find_row(t, id);
    row *r;
    uint32_t k;

    if (i < 0) {
        free(text);
        qv_verbs_free(verbs);
        return;
    }
    r = &t->rows[i];
    r->rgb = rgb;
    r->attrs = attrs;
    free(r->text);
    r->text = text;
    if (qv_verbs_equal(&r->verbs, verbs)) {
        qv_verbs_free(verbs);
        watch_blink(t);
        InvalidateRect(t->hwnd, NULL, FALSE);
        return;
    }
    destroy_buttons(r);
    qv_verbs_free(&r->verbs);
    r->verbs = *verbs;
    verbs->v = NULL;
    verbs->n = 0;
    r->buttons = qv_alloc(r->verbs.n * sizeof *r->buttons);
    for (k = 0; k < r->verbs.n; k++)
        r->buttons[k] = qv_button(t->hwnd, t->hinst, t->font, r->verbs.v[k].label,
                                  (r->verbs.v[k].attrs & QUEVU_VERB_DISABLED) != 0, 0);
    arrange(t);
}

/* Makes the font of the lights for the scale the table is at. */
static void make_light_font(qv_table *t)
{
    if (t->font_light)
        DeleteObject(t->font_light);
    t->font_light = CreateFontW(-sc(t, 17), 0, 0, 0, FW_NORMAL, 0, 0, 0, DEFAULT_CHARSET, 0, 0,
                                CLEARTYPE_QUALITY, 0, L"Segoe UI Symbol");
}

void qv_table_set_font(qv_table *t, HFONT font, UINT dpi)
{
    size_t   i;
    uint32_t k;
    t->font = font;
    t->dpi = dpi;
    make_light_font(t);
    for (i = 0; i < t->nrows; i++)
        for (k = 0; k < t->rows[i].verbs.n; k++)
            SendMessageW(t->rows[i].buttons[k], WM_SETFONT, (WPARAM)font, 1);
    arrange(t);
}

/* The width the buttons of a row take, side by side. */
static int verbs_width(const qv_table *t, const row *r, int gap)
{
    uint32_t k;
    int      total = 0;
    for (k = 0; k < r->verbs.n; k++)
        total += qv_button_width(t->hwnd, t->font, r->verbs.v[k].label, t->dpi) + (k ? gap : 0);
    return total;
}

/*
 * Works out the columns, puts every button in its place and brings the
 * scroll bar up to date.
 */
static void arrange(qv_table *t)
{
    SCROLLINFO si = {0};
    int    w, h, pad = sc(t, 8), gap = sc(t, 6), bh = sc(t, 26), rh = row_h(t);
    int    name_w = 0, verbs_w = 0, content, most, left, limit;
    size_t i;

    watch_blink(t);
    client(t, &w, &h);

    /*
     * The column of names is as wide as the longest name, within reason;
     * the column of verbs as wide as the longest row of them.
     */
    for (i = 0; i < t->nrows; i++) {
        int nw = qv_text_width(t->hwnd, t->font, t->rows[i].name);
        int vw = verbs_width(t, &t->rows[i], gap);
        if (nw > name_w)
            name_w = nw;
        if (vw > verbs_w)
            verbs_w = vw;
    }
    limit = w * 2 / 5;
    if (limit < sc(t, 60))
        limit = sc(t, 60);
    t->name_w = name_w < limit ? name_w : limit;
    t->verbs_w = verbs_w;

    content = (int)t->nrows * rh;
    most = content - h > 0 ? content - h : 0;
    if (t->scroll > most)
        t->scroll = most;
    if (t->scroll < 0)
        t->scroll = 0;
    si.cbSize = sizeof si;
    si.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
    si.nMin = 0;
    si.nMax = content > 0 ? content - 1 : 0;
    si.nPage = (UINT)(h > 0 ? h : 0);
    si.nPos = t->scroll;
    SetScrollInfo(t->hwnd, SB_VERT, &si, TRUE);
    /* The bar may have come or gone, and taken width with it. */
    client(t, &w, &h);

    left = w - pad - t->verbs_w;
    if (left < 0)
        left = 0;
    for (i = 0; i < t->nrows; i++) {
        const row *r = &t->rows[i];
        int        y = (int)i * rh - t->scroll + (rh - bh) / 2, x = left;
        uint32_t   k;
        for (k = 0; k < r->verbs.n; k++) {
            int bw = qv_button_width(t->hwnd, t->font, r->verbs.v[k].label, t->dpi);
            MoveWindow(r->buttons[k], x, y, bw, bh, TRUE);
            x += bw + gap;
        }
    }
    InvalidateRect(t->hwnd, NULL, FALSE);
}

static void scroll_to(qv_table *t, int p)
{
    int w, h, content, most;
    client(t, &w, &h);
    content = (int)t->nrows * row_h(t);
    most = content - h > 0 ? content - h : 0;
    if (p > most)
        p = most;
    if (p < 0)
        p = 0;
    if (p != t->scroll) {
        t->scroll = p;
        arrange(t);
    }
}

static void on_vscroll(qv_table *t, UINT code)
{
    int w, h, line = row_h(t);
    client(t, &w, &h);
    switch (code) {
    case SB_LINEUP:
        scroll_to(t, t->scroll - line);
        break;
    case SB_LINEDOWN:
        scroll_to(t, t->scroll + line);
        break;
    case SB_PAGEUP:
        scroll_to(t, t->scroll - h);
        break;
    case SB_PAGEDOWN:
        scroll_to(t, t->scroll + h);
        break;
    case SB_TOP:
        scroll_to(t, 0);
        break;
    case SB_BOTTOM:
        scroll_to(t, 0x3fffffff);
        break;
    case SB_THUMBTRACK:
    case SB_THUMBPOSITION: {
        SCROLLINFO si = {0};
        si.cbSize = sizeof si;
        si.fMask = SIF_TRACKPOS;
        GetScrollInfo(t->hwnd, SB_VERT, &si);
        scroll_to(t, si.nTrackPos);
        break;
    }
    default:
        break;
    }
}

static void paint(qv_table *t)
{
    PAINTSTRUCT ps;
    HDC     hdc = BeginPaint(t->hwnd, &ps), mem;
    HBITMAP bmp;
    HGDIOBJ old_bmp, old_font;
    RECT    all;
    int     w, h, pad = sc(t, 8), d = sc(t, 12), rh = row_h(t);
    int     name_x, text_x, text_end;
    UINT    flags = DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
    size_t  i;

    client(t, &w, &h);
    all.left = all.top = 0;
    all.right = w;
    all.bottom = h;

    /*
     * Drawn off screen and copied in one go: a light that blinks must
     * not make the rest flicker.
     */
    mem = CreateCompatibleDC(hdc);
    bmp = CreateCompatibleBitmap(hdc, w > 1 ? w : 1, h > 1 ? h : 1);
    old_bmp = SelectObject(mem, bmp);
    FillRect(mem, &all, GetSysColorBrush(COLOR_WINDOW));
    old_font = SelectObject(mem, t->font);
    SetBkMode(mem, TRANSPARENT);
    SetTextColor(mem, GetSysColor(COLOR_WINDOWTEXT));

    name_x = pad * 2 + d + pad;
    text_x = name_x + t->name_w + pad * 2;
    text_end = w - pad - t->verbs_w - pad;
    if (text_end < text_x)
        text_end = text_x;

    for (i = 0; i < t->nrows; i++) {
        const row *r = &t->rows[i];
        int   top = (int)i * rh - t->scroll;
        int   off;
        WCHAR disc;
        RECT  rc;

        if (top + rh < 0 || top > h)
            continue;
        /*
         * The light: a disc of the color the host gave. Hollow, or in
         * the dark half of a blink, only its rim. It is drawn as a
         * character so its edge is as smooth as that of the text.
         */
        off = (r->attrs & QUEVU_LIGHT_HOLLOW) || ((r->attrs & QUEVU_LIGHT_BLINK) && t->dark);
        disc = off ? LIGHT_OFF : LIGHT_ON;
        rc.left = pad * 2 - d / 2, rc.top = top, rc.right = pad * 2 + d + d / 2, rc.bottom = top + rh;
        SelectObject(mem, t->font_light);
        SetTextColor(mem, qv_color(r->rgb));
        DrawTextW(mem, &disc, 1, &rc, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        SelectObject(mem, t->font);
        SetTextColor(mem, GetSysColor(COLOR_WINDOWTEXT));

        rc.left = name_x, rc.top = top, rc.right = name_x + t->name_w, rc.bottom = top + rh;
        DrawTextW(mem, r->name, -1, &rc, flags);
        rc.left = text_x, rc.top = top, rc.right = text_end, rc.bottom = top + rh;
        DrawTextW(mem, r->text, -1, &rc, flags);

        /* A thin line under each row: it is a table. */
        rc.left = pad, rc.top = top + rh - 1, rc.right = w - pad, rc.bottom = top + rh;
        FillRect(mem, &rc, GetSysColorBrush(COLOR_BTNFACE));
    }

    BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(t->hwnd, &ps);
}

/*
 * A button of a row was pressed: the host hears which verb of which
 * control. Nothing changes here until the host says so.
 */
static void pressed(const qv_table *t, HWND button)
{
    size_t   i;
    uint32_t k;
    for (i = 0; i < t->nrows; i++)
        for (k = 0; k < t->rows[i].verbs.n; k++)
            if (t->rows[i].buttons[k] == button) {
                qv_emit(QUEVU_EV_CONTROL_VERB, t->rows[i].id, t->rows[i].verbs.v[k].id, 0, 0);
                return;
            }
}

static LRESULT CALLBACK table_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    qv_table *t;

    if (msg == WM_NCCREATE) {
        CREATESTRUCTW *cs = (CREATESTRUCTW *)lparam;
        seed *s = cs->lpCreateParams;
        t = qv_alloc(sizeof *t);
        t->hwnd = hwnd;
        t->hinst = cs->hInstance;
        t->font = s->font;
        t->dpi = s->dpi;
        free(s);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (LONG_PTR)t);
        make_light_font(t);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    t = qv_table_of(hwnd);
    if (msg == WM_NCDESTROY) {
        if (t) {
            size_t i;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            /* The buttons went with the window; what is left is memory. */
            for (i = 0; i < t->nrows; i++) {
                free(t->rows[i].buttons);
                qv_verbs_free(&t->rows[i].verbs);
                free(t->rows[i].name);
                free(t->rows[i].text);
            }
            free(t->rows);
            DeleteObject(t->font_light);
            free(t);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if (!t)
        return DefWindowProcW(hwnd, msg, wparam, lparam);

    switch (msg) {
    case WM_SIZE:
        arrange(t);
        return 0;
    case WM_ERASEBKGND:
        return 1;
    case WM_PAINT:
        paint(t);
        return 0;
    case WM_TIMER:
        if (wparam == TIMER_BLINK) {
            t->dark = !t->dark;
            InvalidateRect(hwnd, NULL, FALSE);
        }
        return 0;
    case WM_COMMAND:
        if (lparam != 0)
            pressed(t, (HWND)lparam);
        return 0;
    case WM_VSCROLL:
        on_vscroll(t, LOWORD(wparam));
        return 0;
    case WM_MOUSEWHEEL: {
        double notches = (double)GET_WHEEL_DELTA_WPARAM(wparam) / 120.0;
        scroll_to(t, t->scroll - (int)(notches * (double)row_h(t)));
        return 0;
    }
    default:
        break;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}
