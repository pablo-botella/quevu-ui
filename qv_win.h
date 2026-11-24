/*
 * qv_win.h - what the Win32 files of the visual part share: the helpers,
 * the pseudo-console as a control and the table of controls as another.
 *
 * Everything declared here runs on the thread of the window.
 */
#ifndef QV_WIN_H
#define QV_WIN_H

#include <windows.h>
#include <commctrl.h>
#include <shellapi.h>

#include "qv.h"

/* The host queued commands: drain them. */
#define WM_QV_WAKE (WM_APP + 1)
/* The console of tab wparam changed state: refresh its status strip. */
#define WM_QV_STATE (WM_APP + 2)
/* Something happened to the icon in the notification area. */
#define WM_QV_TRAY (WM_APP + 3)

/* Scales a length given for 96 dpi to the dpi of a window. */
static inline int qv_scale(int v, UINT dpi)
{
    return (int)((long long)v * (long long)dpi / 96);
}

/* A color of the interface, 0x00RRGGBB, as GDI wants it. */
static inline COLORREF qv_color(uint32_t rgb)
{
    return RGB((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
}

/* The width a text takes in a font, in pixels. */
int qv_text_width(HWND hwnd, HFONT font, const WCHAR *text);
/* The width of a push button that says label: the words and some air. */
int qv_button_width(HWND hwnd, HFONT font, const WCHAR *label, UINT dpi);
/* A push button as a child of parent, with the host's words on it. */
HWND qv_button(HWND parent, HINSTANCE hinst, HFONT font, const WCHAR *label, int disabled, UINT id);

/* ---- the pseudo-console ---- */

typedef struct qv_view qv_view;

void     qv_view_register(HINSTANCE hinst);
HWND     qv_view_create(HWND parent, HINSTANCE hinst, uint32_t tab, HFONT font, int cw, int ch);
qv_view *qv_view_of(HWND hwnd);
int      qv_view_status(const qv_view *v);
void     qv_view_follow(qv_view *v);
void     qv_view_pause(qv_view *v);
/* Take what the lines hold, like the model does. */
void     qv_view_deliver(qv_view *v, uint32_t op, int full, int history, qv_line *lines, uint32_t n);
void     qv_view_goto(qv_view *v, qv_line *lines, uint32_t n, uint64_t mark, int history);
int      qv_view_history_offered(const qv_view *v);
int      qv_view_in_history(const qv_view *v);
void     qv_view_request_history(qv_view *v);
uint64_t qv_view_find_from(const qv_view *v, int forward);
void     qv_view_set_font(qv_view *v, HFONT font, int cw, int ch);
void     qv_view_set_show_time(qv_view *v, int on);
void     qv_view_copy(const qv_view *v);

/* ---- the table of controls ---- */

typedef struct qv_table qv_table;

void      qv_table_register(HINSTANCE hinst);
HWND      qv_table_create(HWND parent, HINSTANCE hinst, HFONT font, UINT dpi);
qv_table *qv_table_of(HWND hwnd);
int       qv_table_has(const qv_table *t, uint32_t id);
/* The name, the text and the verbs given are taken. */
void      qv_table_add(qv_table *t, uint32_t id, WCHAR *name);
void      qv_table_set(qv_table *t, uint32_t id, uint32_t rgb, uint32_t attrs, WCHAR *text, qv_verbs *verbs);
void      qv_table_remove(qv_table *t, uint32_t id);
void      qv_table_set_font(qv_table *t, HFONT font, UINT dpi);

#endif /* QV_WIN_H */
