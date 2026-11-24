/*
 * qv.h - what the files of the visual part share among themselves.
 *
 * The interface a host sees is quevu_ui.h, and nothing else. This header
 * is the inside: the lines of a console, the model that holds them, and
 * the two one-way lanes between the threads of the host and the thread
 * of the window.
 *
 * Text is kept as UTF-16, the unit the native toolkit counts and draws
 * in. Strings are NUL-terminated and owned by whoever holds the pointer.
 */
#ifndef QV_H
#define QV_H

#include <stddef.h>
#include <stdint.h>

#include "quevu_ui.h"

/* Memory. Both end the program if there is none left. */
void *qv_alloc(size_t n);          /* zeroed */
void *qv_grow(void *p, size_t n);  /* realloc */

/* Strings. */
size_t    qv_wlen(const uint16_t *s);
uint16_t *qv_wdup(const uint16_t *s);
int       qv_wsame(const uint16_t *a, const uint16_t *b);
/* UTF-8 to UTF-16; what is not valid becomes U+FFFD. Null gives "". */
uint16_t *qv_utf8_to_wide(const char *s);
/* UTF-16 to UTF-8, NUL-terminated; *len is the length in bytes. */
char     *qv_wide_to_utf8(const uint16_t *s, size_t n, size_t *len);

/* A run of colored text inside a line, in UTF-16 units. */
typedef struct qv_span {
    uint32_t start;
    uint32_t len;
    uint32_t rgb; /* 0xRRGGBB */
} qv_span;

/* One line of a console. A chunk is always exactly one line. */
typedef struct qv_line {
    uint64_t  seq;     /* position in its log; 0 is a note of the console */
    int64_t   time_ns; /* when it entered the log                         */
    uint16_t *text;    /* made drawable; not NUL-terminated               */
    uint32_t  len;
    qv_span  *spans;   /* empty for plain text                            */
    uint32_t  nspans;
} qv_line;

void qv_line_free(qv_line *l);

/*
 * Makes the bytes of a chunk drawable on one line of a monospace grid:
 * tabs expanded, ANSI colors turned into spans, every other escape
 * sequence dropped, control characters shown as a middle dot. Fills the
 * text and the spans of *out and leaves the rest alone.
 */
void qv_display_line(const uint8_t *data, uint32_t len, qv_line *out);

/* Where a new tab goes among those already there, given their weights. */
size_t qv_tab_insert_at(const int32_t *weights, size_t n, int32_t weight);
/* The tab that comes to the front when the one in front goes; -1: none. */
int qv_tab_front_after_remove(size_t removed, size_t left);

/* A request of a console for its source. */
typedef struct qv_req {
    uint32_t op;
    uint64_t seq;
    uint32_t n;
} qv_req;

typedef struct qv_reqs {
    qv_req r[4];
    int    n;
} qv_reqs;

enum { QV_WAITING, QV_FOLLOWING, QV_PAUSED, QV_PAUSED_FULL };

/*
 * The model of a pseudo-console: a window of lines, bounded in size,
 * over a log that may be far longer. It talks to nobody: its functions
 * give back the requests that should travel to the source, and
 * qv_console_deliver takes what comes back.
 */
typedef struct qv_console {
    qv_line *lines;
    size_t   len, cap;
    size_t   max_lines;
    uint32_t page;
    size_t   top;    /* index of the first visible line                */
    size_t   rows;   /* how many lines fit in the view                 */
    int64_t  origin; /* origin + i names line i through every change   */
    int      has_mark;
    uint64_t mark;   /* the line a search landed on                    */
    int      history_offered;
    int      in_history;
    int      short_back; /* the last answer looking back came short    */
    int      status;
    int      top_exhausted;
    int      pending_before;
    int      pending_after;
} qv_console;

void     qv_console_init(qv_console *c, size_t max_lines, uint32_t page);
void     qv_console_free(qv_console *c);
qv_req   qv_console_follow(qv_console *c);
void     qv_console_set_rows(qv_console *c, size_t rows);
int      qv_console_pause(qv_console *c, qv_req *out);
void     qv_console_scroll_by(qv_console *c, ptrdiff_t delta, qv_reqs *out);
void     qv_console_scroll_to(qv_console *c, size_t top, qv_reqs *out);
uint64_t qv_console_find_from(const qv_console *c, int forward);
void     qv_console_offer_history(qv_console *c, int available);
int      qv_console_request_history(qv_console *c, qv_req *out);
/*
 * These two take what the lines hold (text and spans): each line is kept
 * by the console or freed. The array itself stays with the caller.
 */
void qv_console_goto(qv_console *c, qv_line *lines, uint32_t n, uint64_t mark,
                     int history, qv_reqs *out);
void qv_console_deliver(qv_console *c, uint32_t op, int full, qv_line *lines,
                        uint32_t n);

/* A verb as the window keeps it. */
typedef struct qv_verb {
    uint32_t  id;
    uint32_t  attrs;
    uint16_t *label;
} qv_verb;

typedef struct qv_verbs {
    qv_verb *v;
    uint32_t n;
} qv_verbs;

void qv_verbs_free(qv_verbs *vs);
int  qv_verbs_equal(const qv_verbs *a, const qv_verbs *b);

/* The texts the window shows on its own account: QUEVU_TEXT_* below this. */
#define QV_TEXT_COUNT 17

/* Something the host asks the window to do. */
enum {
    QV_CONSOLE_ADD = 1, /* id, text: title, weight                        */
    QV_CONTROLS_ADD,    /* id, text: title, weight                        */
    QV_TAB_TITLE,       /* id, text                                       */
    QV_TAB_SELECT,      /* id                                             */
    QV_TAB_REMOVE,      /* id                                             */
    QV_TAB_NOTE,        /* id: tab, text                                  */
    QV_TAB_VERBS,       /* id: tab, verbs                                 */
    QV_CONTROL_ADD,     /* id, tab, text: name                            */
    QV_CONTROL_STATE,   /* id, a: rgb, b: attrs, text, verbs              */
    QV_CONTROL_REMOVE,  /* id                                             */
    QV_DELIVER,         /* id: tab, a: op, full, history, lines           */
    QV_MENU_ADD,        /* id, tab: parent, text: label, b: attrs         */
    QV_MENU_SET,        /* id, text: label, b: attrs                      */
    QV_MENU_REMOVE,     /* id                                             */
    QV_TEXT,            /* id, text                                       */
    QV_TRAY,            /* full: enabled                                  */
    QV_CONSOLE_TIME,    /* id: tab, full: on                              */
    QV_GOTO,            /* id: tab, lines, seq: mark, history             */
    QV_FIND_STATUS,     /* id: tab, text, seq: resume, history            */
    QV_SHOW,            /* full: visible                                  */
    QV_QUIT
};

typedef struct qv_cmd {
    struct qv_cmd *next;
    int       kind;
    uint32_t  id;
    uint32_t  tab;
    uint32_t  a, b;
    int32_t   weight;
    uint64_t  seq;
    int       full;
    int       history;
    uint16_t *text;
    qv_verbs  verbs;
    qv_line  *lines;
    uint32_t  nlines;
} qv_cmd;

qv_cmd *qv_cmd_new(int kind);
/* Frees a command with whatever it still owns. */
void qv_cmd_free(qv_cmd *c);

/*
 * The two lanes. Commands go down: the host queues one and wakes the
 * thread of the window, and never waits. Events come up: the window
 * queues one and a thread of the host picks it up, and the window never
 * waits either.
 */
uint32_t qv_new_id(void);
void     qv_post(qv_cmd *c);  /* host side; the queue owns c from here   */
qv_cmd  *qv_drain(void);      /* window side: all queued so far, in order */
void     qv_begin(void);      /* window side: a window is about to exist  */
void     qv_attach(void *window); /* window side: the loop is up          */
void     qv_detach(void);     /* window side: the loop is over            */
void     qv_emit(uint32_t kind, uint32_t id, uint32_t op, uint32_t n,
                 uint64_t seq);
int      qv_next_event(quevu_ui_event *ev, int32_t timeout_ms);
/* What the console of a tab is looking for; text is UTF-8 and is taken. */
void     qv_set_find(uint32_t tab, char *text, int64_t since_ns,
                     int64_t until_ns);
uint32_t qv_find_text(uint32_t tab, char *buf, uint32_t cap);
void     qv_find_range(uint32_t tab, int64_t *since_ns, int64_t *until_ns);

/* Runs the window on the calling thread until the host says quit. */
int qv_win_run(const uint16_t *title);

/* Checks the parts that need no window. Null if all is well. */
const char *qv_selftest(void);

#endif /* QV_H */
