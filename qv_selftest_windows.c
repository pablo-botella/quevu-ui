/*
 * qv_selftest - checks of the parts that need no window: the text of a
 * line, the order of the tabs and the model of a console.
 *
 * They are compiled into the package and run from its Go tests, which is
 * the only way a Go test reaches C. Each check that fails is counted and
 * the first one is reported.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "qv.h"

static char failure[256];
static int  failures;

static void fail(const char *test, int line, const char *what)
{
    if (failures++ == 0)
        snprintf(failure, sizeof failure, "%s (line %d): %s", test, line, what);
}

#define CHECK(cond) \
    do { \
        if (!(cond)) \
            fail(__func__, __LINE__, #cond); \
    } while (0)

/* ---- text ---- */

/* The drawable text of some bytes, as UTF-8, to compare with a literal. */
static char *shown(const char *data, size_t len, qv_line *line)
{
    char *utf8;
    memset(line, 0, sizeof *line);
    qv_display_line((const uint8_t *)data, (uint32_t)len, line);
    utf8 = qv_wide_to_utf8(line->text, line->len, NULL);
    return utf8;
}

static int shows(const char *data, size_t len, const char *want)
{
    qv_line line;
    char   *got = shown(data, len, &line);
    int     ok = strcmp(got, want) == 0;
    free(got);
    qv_line_free(&line);
    return ok;
}

#define SHOWS(lit, want) shows(lit, sizeof(lit) - 1, want)
#define SHOWN(lit, line) shown(lit, sizeof(lit) - 1, line)

static int span_is(const qv_line *l, uint32_t i, uint32_t start, uint32_t len, uint32_t rgb)
{
    return i < l->nspans && l->spans[i].start == start && l->spans[i].len == len && l->spans[i].rgb == rgb;
}

static void test_text(void)
{
    qv_line l;
    char   *t;

    /* Plain text is untouched. */
    t = SHOWN("GET /index 200 \xc3\xa1\xc3\xb1", &l);
    CHECK(strcmp(t, "GET /index 200 \xc3\xa1\xc3\xb1") == 0);
    CHECK(l.nspans == 0 && l.len == 17);
    free(t);
    qv_line_free(&l);

    /* Tabs go to the next stop. */
    CHECK(SHOWS("a\tb", "a       b"));
    CHECK(SHOWS("12345678\tb", "12345678        b"));
    CHECK(SHOWS("\t", "        "));

    /* ANSI sequences leave the text clean. */
    CHECK(SHOWS("\x1b[31mred\x1b[0m plain", "red plain"));
    CHECK(SHOWS("\x1b[1;32;40mok", "ok"));
    CHECK(SHOWS("a\x1bMb", "ab"));
    /* Cursor movement is dropped, not drawn. */
    CHECK(SHOWS("a\x1b[2Kb\x1b[10;5Hc", "abc"));
    /* A sequence cut short swallows the rest and nothing breaks. */
    CHECK(SHOWS("a\x1b[12", "a"));

    /* Colors become spans. */
    t = SHOWN("12:00 \x1b[31mERROR\x1b[0m disk \x1b[33mfull\x1b[39m!", &l);
    CHECK(strcmp(t, "12:00 ERROR disk full!") == 0);
    CHECK(l.nspans == 2 && span_is(&l, 0, 6, 5, 0xcd3131) && span_is(&l, 1, 17, 4, 0xe5e510));
    free(t);
    qv_line_free(&l);

    /* Bold brightens, and resets. */
    t = SHOWN("\x1b[1;31mA\x1b[22mB\x1b[0mC\x1b[1mD", &l);
    CHECK(l.nspans == 3 && span_is(&l, 0, 0, 1, 0xf14c4c) && span_is(&l, 1, 1, 1, 0xcd3131) &&
          span_is(&l, 2, 3, 1, 0xffffff));
    free(t);
    qv_line_free(&l);

    /* Bright, indexed and true colors. */
    t = SHOWN("\x1b[92mA\x1b[38;5;196mB\x1b[38;2;1;2;3mC\x1b[38;5;244mD", &l);
    CHECK(l.nspans == 4 && span_is(&l, 0, 0, 1, 0x23d18b) && span_is(&l, 1, 1, 1, 0xff0000) &&
          span_is(&l, 2, 2, 1, 0x010203) && span_is(&l, 3, 3, 1, 0x808080));
    free(t);
    qv_line_free(&l);

    /*
     * Background colors are read and ignored: 48;5;31 is not "31, red".
     */
    t = SHOWN("\x1b[48;5;31mA\x1b[48;2;31;32;33mB\x1b[41mC", &l);
    CHECK(strcmp(t, "ABC") == 0 && l.nspans == 0);
    free(t);
    qv_line_free(&l);

    /* Spans count in UTF-16 units and merge: the emoji takes two. */
    t = SHOWN("\x1b[32m\xf0\x9f\x98\x80ok\x1b[32m!\x1b[0m", &l);
    CHECK(strcmp(t, "\xf0\x9f\x98\x80ok!") == 0);
    CHECK(l.nspans == 1 && span_is(&l, 0, 0, 5, 0x0dbc79));
    free(t);
    qv_line_free(&l);

    /* Color spans cover expanded tabs. */
    t = SHOWN("\x1b[34ma\tb", &l);
    CHECK(strcmp(t, "a       b") == 0 && l.nspans == 1 && span_is(&l, 0, 0, 9, 0x3b8eea));
    free(t);
    qv_line_free(&l);

    /* A chunk is always one line. */
    CHECK(SHOWS("two\nlines\r\n", "two\xc2\xb7lines\xc2\xb7\xc2\xb7"));
    CHECK(shows("nul\0", 4, "nul\xc2\xb7"));
    /* Invalid UTF-8 does not break. */
    CHECK(SHOWS("a\xff" "b", "a\xef\xbf\xbd" "b"));
}

/* ---- the order of the tabs ---- */

static void test_tabs(void)
{
    static const int32_t three[] = {30, 20, 10}, zeros[] = {0, 0}, four[] = {30, 20, 20, 10};
    static const int32_t a[] = {5, 0, 0}, b[] = {5, 0, -1}, c[] = {0, -1, -5};

    /* The heavier goes first. */
    CHECK(qv_tab_insert_at(NULL, 0, 0) == 0);
    CHECK(qv_tab_insert_at(three, 3, 100) == 0);
    CHECK(qv_tab_insert_at(three, 3, 25) == 1);
    CHECK(qv_tab_insert_at(three, 3, 5) == 3);
    /* Of two that weigh the same, the first stays first. */
    CHECK(qv_tab_insert_at(zeros, 2, 0) == 2);
    CHECK(qv_tab_insert_at(four, 4, 20) == 3);
    /* A negative weight goes after those that say nothing. */
    CHECK(qv_tab_insert_at(a, 3, -1) == 3);
    CHECK(qv_tab_insert_at(b, 3, 0) == 2);
    CHECK(qv_tab_insert_at(c, 3, -3) == 2);
    /* The neighbour takes the front. */
    CHECK(qv_tab_front_after_remove(0, 0) == -1);
    CHECK(qv_tab_front_after_remove(0, 2) == 0);
    CHECK(qv_tab_front_after_remove(1, 2) == 1);
    CHECK(qv_tab_front_after_remove(2, 2) == 1);
}

/* ---- the model of a console ---- */

/* Lines from..to, each saying "l<seq>". The caller frees the array. */
static qv_line *lines(uint64_t from, uint64_t to, uint32_t *n)
{
    qv_line *v;
    uint64_t s;
    char     buf[32];

    *n = to >= from ? (uint32_t)(to - from + 1) : 0;
    v = qv_alloc(*n * sizeof *v);
    for (s = from; s <= to && *n; s++) {
        qv_line *l = &v[s - from];
        size_t   k, len = (size_t)snprintf(buf, sizeof buf, "l%llu", (unsigned long long)s);
        l->seq = s;
        l->text = qv_alloc((len + 1) * sizeof *l->text);
        for (k = 0; k < len; k++)
            l->text[k] = (uint16_t)buf[k];
        l->len = (uint32_t)len;
    }
    return v;
}

static void deliver(qv_console *c, uint32_t op, int full, uint64_t from, uint64_t to)
{
    uint32_t n;
    qv_line *v = lines(from, to, &n);
    qv_console_deliver(c, op, full, v, n);
    free(v);
}

static void deliver_none(qv_console *c, uint32_t op, int full)
{
    qv_console_deliver(c, op, full, NULL, 0);
}

static void go(qv_console *c, uint64_t from, uint64_t to, uint64_t mark, int history, qv_reqs *out)
{
    uint32_t n;
    qv_line *v = lines(from, to, &n);
    qv_console_goto(c, v, n, mark, history, out);
    free(v);
}

/* A console of 10 rows that keeps 60 lines and asks for 20 at a time. */
static void started(qv_console *c, uint64_t upto)
{
    qv_req r;
    qv_console_init(c, 60, 20);
    qv_console_set_rows(c, 10);
    r = qv_console_follow(c);
    CHECK(r.op == QUEVU_OP_FOLLOW && r.seq == 0 && r.n == 20);
    CHECK(c->status == QV_WAITING);
    deliver(c, QUEVU_OP_FOLLOW, 0, upto - 19, upto);
}

/* Whether what is on screen is exactly from..to. */
static int visible(const qv_console *c, uint64_t from, uint64_t to)
{
    size_t i, n = c->len - c->top < c->rows ? c->len - c->top : c->rows;
    if (n != (size_t)(to - from + 1))
        return 0;
    for (i = 0; i < n; i++)
        if (c->lines[c->top + i].seq != from + i)
            return 0;
    return 1;
}

/* Whether the window holds exactly from..to, with no notes. */
static int holds(const qv_console *c, uint64_t from, uint64_t to)
{
    size_t i;
    if (c->len != (size_t)(to - from + 1))
        return 0;
    for (i = 0; i < c->len; i++)
        if (c->lines[i].seq != from + i)
            return 0;
    return 1;
}

static int sees(const qv_console *c, uint64_t seq)
{
    size_t i;
    for (i = c->top; i < c->len && i < c->top + c->rows; i++)
        if (c->lines[i].seq == seq)
            return 1;
    return 0;
}

static int is_req(const qv_reqs *rs, int i, uint32_t op, uint64_t seq, uint32_t n)
{
    return i < rs->n && rs->r[i].op == op && rs->r[i].seq == seq && rs->r[i].n == n;
}

static int asks(const qv_reqs *rs, uint32_t op)
{
    int i;
    for (i = 0; i < rs->n; i++)
        if (rs->r[i].op == op)
            return 1;
    return 0;
}

static int note_says(const qv_line *l, const char *want)
{
    char *got = qv_wide_to_utf8(l->text, l->len, NULL);
    int   ok = l->seq == 0 && strcmp(got, want) == 0;
    free(got);
    return ok;
}

static void test_following(void)
{
    qv_console c;
    qv_reqs    rs;

    /* Pinned to the end. */
    started(&c, 100);
    CHECK(c.status == QV_FOLLOWING && visible(&c, 91, 100));
    deliver(&c, QUEVU_OP_LIVE, 0, 101, 103);
    CHECK(visible(&c, 94, 103));
    /* The window is bounded while following. */
    deliver(&c, QUEVU_OP_LIVE, 0, 104, 1000);
    CHECK(c.len == 60 && c.lines[0].seq == 941 && visible(&c, 991, 1000));
    qv_console_free(&c);

    /* Scrolling back holds the text still. */
    started(&c, 100);
    qv_console_scroll_by(&c, -3, &rs);
    CHECK(rs.n == 2 && is_req(&rs, 0, QUEVU_OP_PAUSE, 0, 0) && is_req(&rs, 1, QUEVU_OP_BEFORE, 81, 20));
    CHECK(c.status == QV_PAUSED && visible(&c, 88, 97));
    /* Live output still in flight is not kept and moves nothing. */
    deliver(&c, QUEVU_OP_LIVE, 0, 101, 150);
    CHECK(visible(&c, 88, 97) && c.lines[c.len - 1].seq == 100);
    qv_console_free(&c);

    /* Resizing keeps the end in view while following. */
    started(&c, 100);
    qv_console_set_rows(&c, 4);
    CHECK(visible(&c, 97, 100));
    qv_console_set_rows(&c, 15);
    CHECK(visible(&c, 86, 100));
    qv_console_set_rows(&c, 500);
    CHECK(c.top == 0);
    qv_console_free(&c);

    /* Repeated chunks are not shown twice. */
    started(&c, 100);
    deliver(&c, QUEVU_OP_LIVE, 0, 98, 102);
    CHECK(holds(&c, 81, 102));
    qv_console_scroll_by(&c, -1, &rs);
    deliver(&c, QUEVU_OP_BEFORE, 0, 70, 90);
    CHECK(holds(&c, 70, 102));
    qv_console_free(&c);
}

static void test_moving(void)
{
    qv_console c;
    qv_reqs    rs;
    qv_req     r;

    /* Older pages arrive without moving what is on screen. */
    started(&c, 100);
    qv_console_scroll_by(&c, -3, &rs);
    deliver(&c, QUEVU_OP_BEFORE, 0, 61, 80);
    CHECK(visible(&c, 88, 97) && c.lines[0].seq == 61);
    qv_console_free(&c);

    /* Only one request for older output is in flight at a time. */
    started(&c, 100);
    qv_console_scroll_by(&c, -1, &rs);
    CHECK(asks(&rs, QUEVU_OP_BEFORE));
    qv_console_scroll_by(&c, -1, &rs);
    CHECK(rs.n == 0);
    deliver(&c, QUEVU_OP_BEFORE, 0, 61, 80);
    /* Near the top again: the next page back is requested. */
    qv_console_scroll_by(&c, -25, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_BEFORE, 61, 20));
    /* The far end is dropped to make room, and asked for again. */
    deliver(&c, QUEVU_OP_BEFORE, 0, 41, 60);
    CHECK(holds(&c, 41, 100));
    qv_console_scroll_by(&c, -25, &rs);
    {
        uint64_t first = c.lines[c.top].seq;
        deliver(&c, QUEVU_OP_BEFORE, 0, 21, 40);
        /* Still 60 lines: the newest 20 went away. */
        CHECK(holds(&c, 21, 80) && visible(&c, first, first + 9));
    }
    qv_console_scroll_by(&c, 40, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_AFTER, 80, 20));
    deliver(&c, QUEVU_OP_AFTER, 0, 81, 100);
    CHECK(c.lines[c.len - 1].seq == 100 && c.len == 60);
    qv_console_free(&c);

    /* An empty answer ends the way back. */
    started(&c, 20);
    qv_console_scroll_by(&c, -1, &rs);
    deliver_none(&c, QUEVU_OP_BEFORE, 0);
    qv_console_scroll_by(&c, -5, &rs);
    CHECK(rs.n == 0);
    qv_console_scroll_by(&c, -5, &rs);
    CHECK(rs.n == 0);
    /* A new follow starts over. */
    qv_console_follow(&c);
    deliver(&c, QUEVU_OP_FOLLOW, 0, 30, 49);
    qv_console_scroll_by(&c, -1, &rs);
    CHECK(asks(&rs, QUEVU_OP_BEFORE));
    qv_console_free(&c);

    /* Resume jumps to the end: it does not catch up. */
    started(&c, 100);
    qv_console_scroll_by(&c, -5, &rs);
    r = qv_console_follow(&c);
    CHECK(r.op == QUEVU_OP_FOLLOW && r.n == 20 && c.status == QV_WAITING);
    deliver(&c, QUEVU_OP_FOLLOW, 0, 981, 1000);
    CHECK(c.status == QV_FOLLOWING && holds(&c, 981, 1000) && visible(&c, 991, 1000));
    qv_console_free(&c);

    /* The source can hang the sign. */
    started(&c, 100);
    deliver_none(&c, QUEVU_OP_PAUSE, 1);
    CHECK(c.status == QV_PAUSED_FULL);
    deliver(&c, QUEVU_OP_LIVE, 0, 101, 110);
    CHECK(c.lines[c.len - 1].seq == 100);
    /* The answer to a pause the console asked for does not hide it. */
    deliver_none(&c, QUEVU_OP_PAUSE, 0);
    CHECK(c.status == QV_PAUSED_FULL);
    qv_console_free(&c);

    /* An explicit pause is asked once. */
    started(&c, 100);
    CHECK(qv_console_pause(&c, &r) && r.op == QUEVU_OP_PAUSE);
    CHECK(!qv_console_pause(&c, &r) && c.status == QV_PAUSED);
    /* Scrolling forwards while paused asks for newer output. */
    qv_console_scroll_by(&c, 1, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_AFTER, 100, 20));
    deliver(&c, QUEVU_OP_AFTER, 0, 101, 120);
    /* The view did not jump: the user moves into the new lines. */
    CHECK(c.lines[c.len - 1].seq == 120 && visible(&c, 91, 100));
    /* Nothing newer yet: the request is free to be made again. */
    qv_console_scroll_by(&c, 30, &rs);
    deliver_none(&c, QUEVU_OP_AFTER, 0);
    qv_console_scroll_by(&c, 1, &rs);
    CHECK(asks(&rs, QUEVU_OP_AFTER));
    qv_console_free(&c);

    /* Output that was not kept leaves a note. */
    started(&c, 100);
    qv_console_pause(&c, &r);
    qv_console_scroll_by(&c, 1, &rs);
    deliver(&c, QUEVU_OP_AFTER, 0, 150, 160);
    CHECK(c.len == 32 && note_says(&c.lines[20], "--- 49 chunks not kept ---"));
    qv_console_free(&c);
    /* And the same looking back. */
    started(&c, 100);
    qv_console_scroll_by(&c, -1, &rs);
    deliver(&c, QUEVU_OP_BEFORE, 0, 50, 79);
    CHECK(c.len == 51 && note_says(&c.lines[30], "--- 1 chunk not kept ---"));
    qv_console_free(&c);

    /* The origin names a line through every change. */
    started(&c, 100);
    {
        int64_t addr = c.origin + 14; /* line 95 of 81..100 */
        CHECK(c.lines[addr - c.origin].seq == 95);
        qv_console_scroll_by(&c, -1, &rs);
        deliver(&c, QUEVU_OP_BEFORE, 0, 61, 80);
        CHECK(c.lines[addr - c.origin].seq == 95);
    }
    qv_console_free(&c);
    started(&c, 100);
    {
        int64_t addr = c.origin + 19;
        deliver(&c, QUEVU_OP_LIVE, 0, 101, 150);
        CHECK(c.lines[addr - c.origin].seq == 100);
    }
    qv_console_free(&c);

    /* Scroll bar positions; past the end is clamped. */
    started(&c, 100);
    qv_console_scroll_to(&c, 0, &rs);
    CHECK(asks(&rs, QUEVU_OP_PAUSE) && c.top == 0);
    qv_console_scroll_to(&c, 10000, &rs);
    CHECK(visible(&c, 91, 100));
    qv_console_free(&c);

    /* Nothing to scroll in an empty console. */
    qv_console_init(&c, 60, 20);
    qv_console_set_rows(&c, 10);
    qv_console_follow(&c);
    deliver_none(&c, QUEVU_OP_FOLLOW, 0);
    qv_console_scroll_by(&c, -3, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_PAUSE, 0, 0));
    qv_console_scroll_by(&c, 3, &rs);
    CHECK(rs.n == 0);
    qv_console_free(&c);
}

static void test_searching(void)
{
    qv_console c;
    qv_reqs    rs;

    /* A search starts from what is on screen. */
    started(&c, 100);
    CHECK(qv_console_find_from(&c, 0) == 0);
    CHECK(qv_console_find_from(&c, 1) == 90);
    qv_console_scroll_by(&c, -5, &rs);
    CHECK(visible(&c, 86, 95));
    CHECK(qv_console_find_from(&c, 0) == 96 && qv_console_find_from(&c, 1) == 85);
    qv_console_free(&c);

    /* A result far away replaces the window and holds it. */
    started(&c, 1000);
    go(&c, 380, 420, 400, 0, &rs);
    CHECK(rs.n == 3 && is_req(&rs, 0, QUEVU_OP_PAUSE, 0, 0) && is_req(&rs, 1, QUEVU_OP_BEFORE, 380, 20) &&
          is_req(&rs, 2, QUEVU_OP_AFTER, 420, 20));
    deliver_none(&c, QUEVU_OP_BEFORE, 0);
    deliver_none(&c, QUEVU_OP_AFTER, 0);
    CHECK(c.status == QV_PAUSED && c.has_mark && c.mark == 400 && holds(&c, 380, 420));
    /* The line found sits a third of the way down a view of 10 rows. */
    CHECK(visible(&c, 397, 406));
    CHECK(qv_console_find_from(&c, 0) == 400 && qv_console_find_from(&c, 1) == 400);
    /* Already held: no second pause request, only the surroundings. */
    go(&c, 300, 300, 300, 0, &rs);
    CHECK(rs.n == 2 && is_req(&rs, 0, QUEVU_OP_BEFORE, 300, 20) && is_req(&rs, 1, QUEVU_OP_AFTER, 300, 20));
    /* The line found is enough: the rest arrives like any page. */
    deliver(&c, QUEVU_OP_BEFORE, 0, 280, 299);
    deliver(&c, QUEVU_OP_AFTER, 0, 301, 320);
    CHECK(holds(&c, 280, 320) && sees(&c, 300));
    /* Going back to the live output forgets the mark. */
    qv_console_follow(&c);
    deliver(&c, QUEVU_OP_FOLLOW, 0, 981, 1000);
    CHECK(!c.has_mark);
    qv_console_free(&c);

    /* A match already held is marked without reloading. */
    started(&c, 100);
    qv_console_scroll_by(&c, -1, &rs);
    deliver(&c, QUEVU_OP_BEFORE, 0, 61, 80);
    {
        uint64_t first = c.lines[c.top].seq, target = first + 4;
        /* In sight: nothing moves at all. */
        go(&c, target - 2, target + 2, target, 0, &rs);
        CHECK(rs.n == 0 && c.mark == target && holds(&c, 61, 100) && visible(&c, first, first + 9));
    }
    /* Held but out of sight: the view moves, the window stays. */
    go(&c, 60, 64, 62, 0, &rs);
    CHECK(rs.n == 0 && holds(&c, 61, 100) && sees(&c, 62));
    qv_console_free(&c);
    /* From the live output it is held first. */
    started(&c, 100);
    qv_console_goto(&c, NULL, 0, 95, 0, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_PAUSE, 0, 0) && c.status == QV_PAUSED && visible(&c, 91, 100));
    qv_console_free(&c);

    /* A match found in the history puts the console in it. */
    started(&c, 1000);
    go(&c, 300, 300, 300, 1, &rs);
    CHECK(c.in_history && rs.n == 3 && is_req(&rs, 1, QUEVU_OP_HISTORY_BEFORE, 300, 20) &&
          is_req(&rs, 2, QUEVU_OP_HISTORY_AFTER, 300, 20));
    qv_console_free(&c);
}

static void test_history(void)
{
    qv_console c;
    qv_reqs    rs;
    qv_req     r;

    /* The history is offered, and entered only by the user. */
    started(&c, 100);
    qv_console_scroll_by(&c, -1, &rs);
    /* Recent output runs short, and the source says there is more. */
    deliver(&c, QUEVU_OP_BEFORE, 0, 75, 80);
    qv_console_offer_history(&c, 1);
    CHECK(c.history_offered && !c.in_history);
    /* Scrolling further asks for recent output only, and then stops. */
    qv_console_scroll_by(&c, -30, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_BEFORE, 75, 20));
    deliver_none(&c, QUEVU_OP_BEFORE, 0);
    qv_console_offer_history(&c, 1);
    qv_console_scroll_by(&c, -5, &rs);
    CHECK(rs.n == 0 && c.history_offered);
    /* The user asks. */
    CHECK(qv_console_request_history(&c, &r) && r.op == QUEVU_OP_HISTORY_BEFORE && r.seq == 75 && r.n == 20);
    CHECK(c.in_history && !c.history_offered);
    CHECK(!qv_console_request_history(&c, &r));
    deliver(&c, QUEVU_OP_HISTORY_BEFORE, 0, 55, 74);
    CHECK(c.lines[0].seq == 55);
    /* From here on the console moves through the history by itself. */
    qv_console_scroll_by(&c, -40, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_HISTORY_BEFORE, 55, 20));
    deliver(&c, QUEVU_OP_HISTORY_BEFORE, 0, 35, 54);
    qv_console_scroll_by(&c, 60, &rs);
    CHECK(rs.n == 1 && is_req(&rs, 0, QUEVU_OP_HISTORY_AFTER, 94, 20));
    /* A full page looking back is not the end, so no offer is kept. */
    qv_console_offer_history(&c, 1);
    CHECK(!c.history_offered);
    /* Following again leaves the history. */
    qv_console_follow(&c);
    deliver(&c, QUEVU_OP_FOLLOW, 0, 981, 1000);
    CHECK(!c.in_history && !c.history_offered);
    qv_console_scroll_by(&c, -1, &rs);
    CHECK(is_req(&rs, 1, QUEVU_OP_BEFORE, 981, 20));
    qv_console_free(&c);

    /* No offer without a history. */
    started(&c, 100);
    qv_console_scroll_by(&c, -1, &rs);
    deliver_none(&c, QUEVU_OP_BEFORE, 0);
    qv_console_offer_history(&c, 0);
    CHECK(!c.history_offered && !qv_console_request_history(&c, &r));
    qv_console_free(&c);
}

/* ---- the lanes ---- */

static void test_lanes(void)
{
    quevu_ui_event ev;
    qv_cmd *c;
    uint32_t a = qv_new_id(), b = qv_new_id();
    char     buf[8];
    int64_t  since = -1, until = -1;

    /* Identifiers are never zero and never repeat. */
    CHECK(a != 0 && b != 0 && a != b);

    /* Commands wait for the loop and keep their order. */
    while ((c = qv_drain()) != NULL)
        while (c) {
            qv_cmd *next = c->next;
            qv_cmd_free(c);
            c = next;
        }
    qv_post(qv_cmd_new(QV_TRAY));
    qv_post(qv_cmd_new(QV_QUIT));
    c = qv_drain();
    CHECK(c && c->kind == QV_TRAY && c->next && c->next->kind == QV_QUIT && !c->next->next);
    CHECK(qv_drain() == NULL);
    while (c) {
        qv_cmd *next = c->next;
        qv_cmd_free(c);
        c = next;
    }

    /* Events time out, arrive, and end after what is still queued. */
    qv_begin();
    while (qv_next_event(&ev, 0) == 1)
        ;
    CHECK(qv_next_event(&ev, 10) == 0);
    qv_emit(QUEVU_EV_CONTROL_VERB, 7, 3, 0, 0);
    CHECK(qv_next_event(&ev, 0) == 1 && ev.kind == QUEVU_EV_CONTROL_VERB && ev.id == 7 && ev.op == 3);
    qv_emit(QUEVU_EV_MENU, 3, 0, 0, 0);
    qv_detach();
    CHECK(qv_next_event(&ev, -1) == 1 && ev.kind == QUEVU_EV_MENU && ev.id == 3);
    CHECK(qv_next_event(&ev, -1) == -1);
    CHECK(qv_next_event(&ev, 5) == -1);
    qv_begin();

    /* What a console is looking for waits for the host to read it. */
    CHECK(qv_find_text(a, buf, sizeof buf) == 0 && buf[0] == 0);
    qv_set_find(a, qv_wide_to_utf8((const uint16_t *)L"needle in hay", 13, NULL), 5, 9);
    CHECK(qv_find_text(a, buf, sizeof buf) == 13 && strcmp(buf, "needle ") == 0);
    qv_find_range(a, &since, &until);
    CHECK(since == 5 && until == 9);
    qv_find_range(b, &since, &until);
    CHECK(since == 0 && until == 0);
}

const char *qv_selftest(void)
{
    failures = 0;
    failure[0] = 0;
    test_text();
    test_tabs();
    test_following();
    test_moving();
    test_searching();
    test_history();
    test_lanes();
    if (failures > 1) {
        size_t n = strlen(failure);
        snprintf(failure + n, sizeof failure - n, " (and %d more)", failures - 1);
    }
    return failures ? failure : NULL;
}
