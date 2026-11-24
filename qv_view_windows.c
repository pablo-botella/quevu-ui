/*
 * qv_view - the pseudo-console as a Win32 control of its own.
 *
 * It is deliberately small: a grid of fixed-width cells drawn straight
 * from the model. Everything about following, pausing and moving by
 * position lives in the model (qv_console); this file only draws lines
 * and turns mouse, wheel, keys and scroll bars into calls on the model.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "qv_win.h"

#define VIEW_CLASS L"QuevuConsole"

/* How many lines the view keeps, and how many it asks for at a time. */
#define MAX_LINES 3000
#define PAGE 300

#define PAD 4
/* Width of the time margin, in cells: "HH:MM:SS.mmm" and a space. */
#define TIME_COLS 13

#define BG RGB(0x1e, 0x1e, 0x1e)
#define FG RGB(0xd4, 0xd4, 0xd4)
#define NOTE_FG RGB(0x8a, 0x8a, 0x8a)
#define SEL_BG RGB(0x26, 0x4f, 0x78)
#define SEL_FG RGB(0xff, 0xff, 0xff)
/* The line a search landed on. */
#define MARK_BG RGB(0x51, 0x48, 0x10)

/* A place in the text: a line, by its stable address, and a column. */
typedef struct pos {
    int64_t line;
    int     col;
} pos;

struct qv_view {
    HWND       hwnd;
    uint32_t   tab;
    qv_console model;
    HFONT      font;
    int        cw, ch;   /* width and height of a cell, in pixels       */
    int        hpos;     /* first visible column                        */
    int        max_cols; /* the longest line in sight                   */
    int        has_anchor;
    pos        anchor;   /* where the mouse went down                   */
    pos        caret;    /* where it is                                 */
    int        dragging;
    int        show_time;
};

/* What the creator of a view hands over through CreateWindowEx. */
typedef struct seed {
    uint32_t tab;
    HFONT    font;
    int      cw, ch;
} seed;

static LRESULT CALLBACK view_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

void qv_view_register(HINSTANCE hinst)
{
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    wc.style = CS_DBLCLKS;
    wc.lpfnWndProc = view_proc;
    wc.hInstance = hinst;
    wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_IBEAM);
    wc.lpszClassName = VIEW_CLASS;
    RegisterClassExW(&wc);
}

HWND qv_view_create(HWND parent, HINSTANCE hinst, uint32_t tab, HFONT font, int cw, int ch)
{
    seed *s = qv_alloc(sizeof *s);
    s->tab = tab;
    s->font = font;
    s->cw = cw;
    s->ch = ch;
    return CreateWindowExW(0, VIEW_CLASS, NULL,
                           WS_CHILD | WS_VSCROLL | WS_HSCROLL | WS_CLIPSIBLINGS | WS_TABSTOP,
                           0, 0, 0, 0, parent, NULL, hinst, s);
}

qv_view *qv_view_of(HWND hwnd)
{
    return (qv_view *)GetWindowLongPtrW(hwnd, GWLP_USERDATA);
}

static void emit(const qv_view *v, const qv_req *r)
{
    qv_emit(QUEVU_EV_CONSOLE_REQUEST, v->tab, r->op, r->n, r->seq);
}

static void emit_all(const qv_view *v, const qv_reqs *rs)
{
    int i;
    for (i = 0; i < rs->n; i++)
        emit(v, &rs->r[i]);
}

static void client(const qv_view *v, int *w, int *h)
{
    RECT rc;
    GetClientRect(v->hwnd, &rc);
    *w = rc.right;
    *h = rc.bottom;
}

/* Width of the time margin, in pixels. */
static int gutter(const qv_view *v)
{
    return v->show_time ? TIME_COLS * v->cw : 0;
}

static int cols(const qv_view *v)
{
    int w, h, n;
    client(v, &w, &h);
    n = (w - PAD - gutter(v)) / v->cw;
    return n > 1 ? n : 1;
}

static void sync_scroll(qv_view *v)
{
    const qv_console *m = &v->model;
    SCROLLINFO si = {0};
    size_t     i, end = m->top + m->rows + 1;
    int        n = cols(v), most;

    v->max_cols = 0;
    for (i = m->top; i < end && i < m->len; i++)
        if ((int)m->lines[i].len > v->max_cols)
            v->max_cols = (int)m->lines[i].len;
    most = v->max_cols - n + 1;
    if (most < 0)
        most = 0;
    if (v->hpos > most)
        v->hpos = most;
    if (v->hpos < 0)
        v->hpos = 0;

    si.cbSize = sizeof si;
    si.fMask = SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL;
    si.nMin = 0;
    si.nMax = m->len > 0 ? (int)m->len - 1 : 0;
    si.nPage = (UINT)m->rows;
    si.nPos = (int)m->top;
    SetScrollInfo(v->hwnd, SB_VERT, &si, TRUE);
    si.nMax = v->max_cols > 0 ? v->max_cols : 0;
    si.nPage = (UINT)n;
    si.nPos = v->hpos;
    SetScrollInfo(v->hwnd, SB_HORZ, &si, TRUE);
}

/*
 * The model changed: scroll bars, pixels and the status strip of the
 * window follow.
 */
static void changed(qv_view *v)
{
    sync_scroll(v);
    InvalidateRect(v->hwnd, NULL, FALSE);
    /*
     * Posted, not sent: the window reads the state when it gets to it,
     * and nothing here runs inside another window procedure.
     */
    PostMessageW(GetParent(v->hwnd), WM_QV_STATE, v->tab, 0);
}

static void resized(qv_view *v)
{
    int w, h, rows;
    client(v, &w, &h);
    rows = h / v->ch;
    qv_console_set_rows(&v->model, rows > 1 ? (size_t)rows : 1);
    changed(v);
}

int qv_view_status(const qv_view *v)
{
    return v->model.status;
}

void qv_view_follow(qv_view *v)
{
    qv_req r = qv_console_follow(&v->model);
    emit(v, &r);
    changed(v);
}

void qv_view_pause(qv_view *v)
{
    qv_req r;
    if (qv_console_pause(&v->model, &r))
        emit(v, &r);
    changed(v);
}

void qv_view_goto(qv_view *v, qv_line *lines, uint32_t n, uint64_t mark, int history)
{
    /*
     * A selection is anchored to this window of lines: it only survives
     * if the window does.
     */
    int64_t origin = v->model.origin;
    size_t  held = v->model.len;
    qv_reqs rs;

    qv_console_goto(&v->model, lines, n, mark, history, &rs);
    if (v->model.origin != origin || v->model.len != held) {
        v->has_anchor = 0;
        v->hpos = 0;
    }
    emit_all(v, &rs);
    changed(v);
}

int qv_view_history_offered(const qv_view *v)
{
    return v->model.history_offered;
}

int qv_view_in_history(const qv_view *v)
{
    return v->model.in_history;
}

void qv_view_request_history(qv_view *v)
{
    qv_req r;
    if (qv_console_request_history(&v->model, &r))
        emit(v, &r);
    changed(v);
}

uint64_t qv_view_find_from(const qv_view *v, int forward)
{
    return qv_console_find_from(&v->model, forward);
}

void qv_view_deliver(qv_view *v, uint32_t op, int full, int history, qv_line *lines, uint32_t n)
{
    if (op == QUEVU_OP_FOLLOW) {
        /* The window starts over: addresses mean something else now. */
        v->has_anchor = 0;
        v->hpos = 0;
    }
    qv_console_deliver(&v->model, op, full, lines, n);
    if (op == QUEVU_OP_BEFORE)
        qv_console_offer_history(&v->model, history);
    changed(v);
}

void qv_view_set_font(qv_view *v, HFONT font, int cw, int ch)
{
    v->font = font;
    v->cw = cw > 1 ? cw : 1;
    v->ch = ch > 1 ? ch : 1;
    resized(v);
}

void qv_view_set_show_time(qv_view *v, int on)
{
    v->show_time = on;
    changed(v);
}

static void scroll_by(qv_view *v, ptrdiff_t delta)
{
    qv_reqs rs;
    qv_console_scroll_by(&v->model, delta, &rs);
    emit_all(v, &rs);
    changed(v);
}

static void scroll_to(qv_view *v, size_t top)
{
    qv_reqs rs;
    qv_console_scroll_to(&v->model, top, &rs);
    emit_all(v, &rs);
    changed(v);
}

static void hscroll_to(qv_view *v, int p)
{
    v->hpos = p > 0 ? p : 0;
    changed(v);
}

/* Division that rounds towards minus infinity, for points off the edge. */
static int floor_div(int a, int b)
{
    int q = a / b;
    if ((a % b != 0) && ((a < 0) != (b < 0)))
        q--;
    return q;
}

/* The place in the text under a point of the client area. */
static pos hit(const qv_view *v, int x, int y)
{
    pos p;
    p.line = v->model.origin + (int64_t)v->model.top + floor_div(y, v->ch);
    p.col = v->hpos + floor_div(x - PAD - gutter(v) + v->cw / 2, v->cw);
    if (p.col < 0)
        p.col = 0;
    return p;
}

static int pos_cmp(pos a, pos b)
{
    if (a.line != b.line)
        return a.line < b.line ? -1 : 1;
    return a.col < b.col ? -1 : a.col > b.col;
}

/* The selection with its ends in reading order. */
static int selection(const qv_view *v, pos *from, pos *to)
{
    int c;
    if (!v->has_anchor)
        return 0;
    c = pos_cmp(v->anchor, v->caret);
    if (c == 0)
        return 0;
    *from = c < 0 ? v->anchor : v->caret;
    *to = c < 0 ? v->caret : v->anchor;
    return 1;
}

/* The columns of a line that fall inside the selection. */
static void selected_cols(int has, pos a, pos b, int64_t addr, int len, int *from, int *to)
{
    *from = *to = 0;
    if (!has || addr < a.line || addr > b.line)
        return;
    *from = addr == a.line ? (a.col < len ? a.col : len) : 0;
    *to = addr == b.line ? (b.col < len ? b.col : len) : len;
    if (*to < *from)
        *to = *from;
}

static void select_all(qv_view *v)
{
    if (v->model.len == 0)
        return;
    qv_view_pause(v);
    v->has_anchor = 1;
    v->anchor.line = v->model.origin;
    v->anchor.col = 0;
    v->caret.line = v->model.origin + (int64_t)v->model.len - 1;
    v->caret.col = (int)v->model.lines[v->model.len - 1].len;
    changed(v);
}

/* Puts the selected text on the clipboard, one line per chunk. */
void qv_view_copy(const qv_view *v)
{
    const qv_console *m = &v->model;
    pos     a, b;
    size_t  i, total = 1, o = 0;
    WCHAR  *out;
    HGLOBAL h;

    if (!selection(v, &a, &b))
        return;
    for (i = 0; i < m->len; i++)
        total += m->lines[i].len + 2;
    out = qv_alloc(total * sizeof *out);
    for (i = 0; i < m->len; i++) {
        int64_t addr = m->origin + (int64_t)i;
        int     from, to;
        if (addr < a.line || addr > b.line)
            continue;
        selected_cols(1, a, b, addr, (int)m->lines[i].len, &from, &to);
        memcpy(out + o, m->lines[i].text + from, (size_t)(to - from) * sizeof *out);
        o += (size_t)(to - from);
        if (addr < b.line) {
            out[o++] = 13;
            out[o++] = 10;
        }
    }
    out[o++] = 0;
    if (OpenClipboard(v->hwnd)) {
        EmptyClipboard();
        h = GlobalAlloc(GMEM_MOVEABLE, o * sizeof *out);
        if (h) {
            WCHAR *p = GlobalLock(h);
            if (p) {
                memcpy(p, out, o * sizeof *out);
                GlobalUnlock(h);
                /* The clipboard owns the block from here on. */
                SetClipboardData(CF_UNICODETEXT, h);
            } else {
                GlobalFree(h);
            }
        }
        CloseClipboard();
    }
    free(out);
}

/* The local time of day of a chunk, as "HH:MM:SS.mmm". */
static int clock_text(int64_t time_ns, WCHAR *buf, size_t cap)
{
    /*
     * Windows counts in units of 100 ns from 1601; the chunk carries
     * nanoseconds from 1970.
     */
    ULONGLONG  ticks = (ULONGLONG)(time_ns / 100 + 116444736000000000LL);
    FILETIME   ft;
    SYSTEMTIME utc, local;

    ft.dwLowDateTime = (DWORD)ticks;
    ft.dwHighDateTime = (DWORD)(ticks >> 32);
    if (!FileTimeToSystemTime(&ft, &utc) || !SystemTimeToTzSpecificLocalTime(NULL, &utc, &local))
        return 0;
    return _snwprintf(buf, cap, L"%02u:%02u:%02u.%03u", local.wHour, local.wMinute, local.wSecond, local.wMilliseconds);
}

static void draw_run(const qv_view *v, HDC dc, int y, const qv_line *line, int from, int to, COLORREF fg, COLORREF bg)
{
    /* Only what is in sight is handed to GDI. */
    int end = v->hpos + cols(v) + 1;
    if (from < v->hpos)
        from = v->hpos;
    if (to > end)
        to = end;
    if (to > (int)line->len)
        to = (int)line->len;
    if (to <= from)
        return;
    SetTextColor(dc, fg);
    SetBkColor(dc, bg);
    ExtTextOutW(dc, PAD + gutter(v) + (from - v->hpos) * v->cw, y, 0, NULL, line->text + from, (UINT)(to - from), NULL);
}

static int int_cmp(const void *a, const void *b)
{
    int x = *(const int *)a, y = *(const int *)b;
    return x < y ? -1 : x > y;
}

static void paint(qv_view *v)
{
    const qv_console *m = &v->model;
    PAINTSTRUCT ps;
    HDC     hdc = BeginPaint(v->hwnd, &ps), mem;
    HBITMAP bmp;
    HGDIOBJ old_bmp, old_font;
    HBRUSH  brush;
    RECT    rc;
    int     w, h, has_sel, row;
    pos     a = {0, 0}, b = {0, 0};
    int    *cuts = NULL;
    size_t  cuts_cap = 0;

    client(v, &w, &h);
    rc.left = rc.top = 0;
    rc.right = w;
    rc.bottom = h;

    /*
     * Drawn off screen and copied in one go, so nothing flickers while
     * output streams in.
     */
    mem = CreateCompatibleDC(hdc);
    bmp = CreateCompatibleBitmap(hdc, w > 1 ? w : 1, h > 1 ? h : 1);
    old_bmp = SelectObject(mem, bmp);
    brush = CreateSolidBrush(BG);
    FillRect(mem, &rc, brush);
    DeleteObject(brush);
    old_font = SelectObject(mem, v->font);
    SetBkMode(mem, OPAQUE);

    has_sel = selection(v, &a, &b);
    for (row = 0; (size_t)row <= m->rows && m->top + (size_t)row < m->len; row++) {
        const qv_line *line = &m->lines[m->top + (size_t)row];
        int      y = row * v->ch;
        int      len = (int)line->len;
        int      noted = line->seq == 0;
        COLORREF row_bg = (!noted && m->has_mark && m->mark == line->seq) ? MARK_BG : BG;
        COLORREF base = noted ? NOTE_FG : FG;
        int      from, to;
        size_t   n = 0, k, need = 4 + 2 * (size_t)line->nspans;
        uint32_t s;

        if (row_bg != BG) {
            RECT band;
            band.left = 0, band.top = y, band.right = w, band.bottom = y + v->ch;
            brush = CreateSolidBrush(row_bg);
            FillRect(mem, &band, brush);
            DeleteObject(brush);
        }
        if (v->show_time && !noted) {
            /*
             * The time sits in a margin of its own: it is not part of
             * the text, so it is never selected or copied.
             */
            WCHAR stamp[32];
            int   sn = clock_text(line->time_ns, stamp, 32);
            if (sn > 0) {
                SetTextColor(mem, NOTE_FG);
                SetBkColor(mem, row_bg);
                ExtTextOutW(mem, PAD, y, 0, NULL, stamp, (UINT)sn, NULL);
            }
        }
        selected_cols(has_sel, a, b, m->origin + (int64_t)m->top + row, len, &from, &to);
        /*
         * The line is cut wherever the color or the selection changes
         * and each piece is drawn in one go. The selection wins over the
         * color of the text.
         */
        if (need > cuts_cap) {
            cuts_cap = need * 2;
            cuts = qv_grow(cuts, cuts_cap * sizeof *cuts);
        }
        cuts[n++] = 0;
        cuts[n++] = len;
        cuts[n++] = from;
        cuts[n++] = to;
        for (s = 0; s < line->nspans; s++) {
            cuts[n++] = (int)line->spans[s].start;
            cuts[n++] = (int)(line->spans[s].start + line->spans[s].len);
        }
        qsort(cuts, n, sizeof *cuts, int_cmp);
        for (k = 0; k + 1 < n; k++) {
            int      p = cuts[k], q = cuts[k + 1];
            COLORREF fg = base;
            if (p == q || p < 0 || q > len)
                continue;
            if (from < to && p >= from && q <= to) {
                draw_run(v, mem, y, line, p, q, SEL_FG, SEL_BG);
                continue;
            }
            for (s = 0; s < line->nspans; s++)
                if ((int)line->spans[s].start <= p && p < (int)(line->spans[s].start + line->spans[s].len)) {
                    fg = qv_color(line->spans[s].rgb);
                    break;
                }
            draw_run(v, mem, y, line, p, q, fg, row_bg);
        }
    }
    free(cuts);

    BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp);
    DeleteDC(mem);
    EndPaint(v->hwnd, &ps);
}

static ptrdiff_t page_rows(const qv_view *v)
{
    return (ptrdiff_t)(v->model.rows > 2 ? v->model.rows : 2) - 1;
}

static int track_pos(HWND hwnd, int bar)
{
    SCROLLINFO si = {0};
    si.cbSize = sizeof si;
    si.fMask = SIF_TRACKPOS;
    GetScrollInfo(hwnd, bar, &si);
    return si.nTrackPos;
}

static void on_vscroll(qv_view *v, UINT code)
{
    switch (code) {
    case SB_LINEUP:
        scroll_by(v, -1);
        break;
    case SB_LINEDOWN:
        scroll_by(v, 1);
        break;
    case SB_PAGEUP:
        scroll_by(v, -page_rows(v));
        break;
    case SB_PAGEDOWN:
        scroll_by(v, page_rows(v));
        break;
    case SB_TOP:
        scroll_to(v, 0);
        break;
    case SB_BOTTOM:
        scroll_to(v, SIZE_MAX);
        break;
    case SB_THUMBTRACK:
    case SB_THUMBPOSITION: {
        int p = track_pos(v->hwnd, SB_VERT);
        scroll_to(v, p > 0 ? (size_t)p : 0);
        break;
    }
    default:
        break;
    }
}

static void on_hscroll(qv_view *v, UINT code)
{
    int page = cols(v) - 1;
    if (page < 1)
        page = 1;
    switch (code) {
    case SB_LINEUP:
        hscroll_to(v, v->hpos - 1);
        break;
    case SB_LINEDOWN:
        hscroll_to(v, v->hpos + 1);
        break;
    case SB_PAGEUP:
        hscroll_to(v, v->hpos - page);
        break;
    case SB_PAGEDOWN:
        hscroll_to(v, v->hpos + page);
        break;
    case SB_THUMBTRACK:
    case SB_THUMBPOSITION:
        hscroll_to(v, track_pos(v->hwnd, SB_HORZ));
        break;
    default:
        break;
    }
}

static int on_key(qv_view *v, WPARAM vk)
{
    int ctrl = GetKeyState(VK_CONTROL) < 0;
    switch (vk) {
    case VK_UP:
        scroll_by(v, -1);
        return 1;
    case VK_DOWN:
        scroll_by(v, 1);
        return 1;
    case VK_PRIOR:
        scroll_by(v, -page_rows(v));
        return 1;
    case VK_NEXT:
        scroll_by(v, page_rows(v));
        return 1;
    case VK_HOME:
        scroll_to(v, 0);
        return 1;
    case VK_END:
        /* End is "take me back to the live output". */
        qv_view_follow(v);
        return 1;
    default:
        break;
    }
    if (ctrl && vk == 'C') {
        qv_view_copy(v);
        return 1;
    }
    if (ctrl && vk == 'A') {
        select_all(v);
        return 1;
    }
    return 0;
}

static LRESULT CALLBACK view_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    qv_view *v;

    if (msg == WM_NCCREATE) {
        CREATESTRUCTW *cs = (CREATESTRUCTW *)lparam;
        seed *s = cs->lpCreateParams;
        v = qv_alloc(sizeof *v);
        v->hwnd = hwnd;
        v->tab = s->tab;
        v->font = s->font;
        v->cw = s->cw > 1 ? s->cw : 1;
        v->ch = s->ch > 1 ? s->ch : 1;
        qv_console_init(&v->model, MAX_LINES, PAGE);
        free(s);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (LONG_PTR)v);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    v = qv_view_of(hwnd);
    if (msg == WM_NCDESTROY) {
        if (v) {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            qv_console_free(&v->model);
            free(v);
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    if (!v)
        return DefWindowProcW(hwnd, msg, wparam, lparam);

    switch (msg) {
    case WM_SIZE:
        resized(v);
        return 0;
    case WM_ERASEBKGND:
        return 1;
    case WM_PAINT:
        paint(v);
        return 0;
    case WM_GETDLGCODE:
        return DLGC_WANTARROWS | DLGC_WANTCHARS;
    case WM_MOUSEWHEEL: {
        double notches = (double)GET_WHEEL_DELTA_WPARAM(wparam) / 120.0;
        if (GetKeyState(VK_SHIFT) < 0) {
            hscroll_to(v, v->hpos - (int)(notches * 8.0));
        } else {
            /* Wheel up is positive, and means older output. */
            double lines = notches * 3.0;
            scroll_by(v, -(ptrdiff_t)(lines < 0 ? lines - 0.5 : lines + 0.5));
        }
        return 0;
    }
    case WM_VSCROLL:
        on_vscroll(v, LOWORD(wparam));
        return 0;
    case WM_HSCROLL:
        on_hscroll(v, LOWORD(wparam));
        return 0;
    case WM_KEYDOWN:
        if (on_key(v, wparam))
            return 0;
        break;
    case WM_LBUTTONDOWN:
        SetFocus(hwnd);
        SetCapture(hwnd);
        /* A selection needs the text to hold still. */
        qv_view_pause(v);
        v->anchor = hit(v, (short)LOWORD(lparam), (short)HIWORD(lparam));
        v->caret = v->anchor;
        v->has_anchor = 1;
        v->dragging = 1;
        changed(v);
        return 0;
    case WM_MOUSEMOVE:
        if (v->dragging) {
            int x = (short)LOWORD(lparam), y = (short)HIWORD(lparam), w, h;
            client(v, &w, &h);
            /* Dragging past an edge walks the text along. */
            if (y < 0)
                scroll_by(v, -1);
            else if (y >= h)
                scroll_by(v, 1);
            if (y > h - 1)
                y = h - 1;
            if (y < 0)
                y = 0;
            v->caret = hit(v, x, y);
            changed(v);
        }
        return 0;
    case WM_LBUTTONUP:
        if (v->dragging) {
            v->dragging = 0;
            ReleaseCapture();
        }
        return 0;
    default:
        break;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}
