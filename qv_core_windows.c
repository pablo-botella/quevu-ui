/*
 * qv_core - the parts of the visual part that need no window: strings,
 * the text of a line, the order of the tabs and the model of a console.
 *
 * Nothing here knows the system it runs on. The file carries the name of
 * one only because, for now, the window exists for one.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "qv.h"

void *qv_alloc(size_t n)
{
    void *p = calloc(1, n ? n : 1);
    if (!p)
        abort();
    return p;
}

void *qv_grow(void *p, size_t n)
{
    p = realloc(p, n ? n : 1);
    if (!p)
        abort();
    return p;
}

size_t qv_wlen(const uint16_t *s)
{
    size_t n = 0;
    if (s)
        while (s[n])
            n++;
    return n;
}

uint16_t *qv_wdup(const uint16_t *s)
{
    size_t n = qv_wlen(s);
    uint16_t *d = qv_alloc((n + 1) * sizeof *d);
    if (n)
        memcpy(d, s, n * sizeof *d);
    return d;
}

int qv_wsame(const uint16_t *a, const uint16_t *b)
{
    size_t n = qv_wlen(a);
    return n == qv_wlen(b) && (n == 0 || memcmp(a, b, n * sizeof *a) == 0);
}

/* A growing run of UTF-16 units. */
typedef struct wbuf {
    uint16_t *v;
    size_t    n, cap;
} wbuf;

static void wpush(wbuf *b, uint16_t u)
{
    if (b->n == b->cap) {
        b->cap = b->cap ? b->cap * 2 : 64;
        b->v = qv_grow(b->v, b->cap * sizeof *b->v);
    }
    b->v[b->n++] = u;
}

/* Appends a code point and says how many units it took. */
static uint32_t wpush_cp(wbuf *b, uint32_t cp)
{
    if (cp < 0x10000) {
        wpush(b, (uint16_t)cp);
        return 1;
    }
    cp -= 0x10000;
    wpush(b, (uint16_t)(0xd800 + (cp >> 10)));
    wpush(b, (uint16_t)(0xdc00 + (cp & 0x3ff)));
    return 2;
}

/*
 * Reads one code point of UTF-8 at s[*i] and moves *i past it. A byte
 * that starts nothing valid gives U+FFFD and is passed alone.
 */
static uint32_t decode(const uint8_t *s, size_t len, size_t *i)
{
    uint8_t  b = s[*i];
    uint32_t cp, min;
    size_t   need, k;

    if (b < 0x80) {
        (*i)++;
        return b;
    }
    if ((b & 0xe0) == 0xc0) {
        cp = b & 0x1f, need = 1, min = 0x80;
    } else if ((b & 0xf0) == 0xe0) {
        cp = b & 0x0f, need = 2, min = 0x800;
    } else if ((b & 0xf8) == 0xf0) {
        cp = b & 0x07, need = 3, min = 0x10000;
    } else {
        (*i)++;
        return 0xfffd;
    }
    if (len - *i <= need) {
        (*i)++;
        return 0xfffd;
    }
    for (k = 1; k <= need; k++) {
        uint8_t c = s[*i + k];
        if ((c & 0xc0) != 0x80) {
            (*i)++;
            return 0xfffd;
        }
        cp = (cp << 6) | (c & 0x3f);
    }
    if (cp < min || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) {
        (*i)++;
        return 0xfffd;
    }
    *i += need + 1;
    return cp;
}

uint16_t *qv_utf8_to_wide(const char *s)
{
    wbuf   b = {0};
    size_t len = s ? strlen(s) : 0, i = 0;

    while (i < len)
        wpush_cp(&b, decode((const uint8_t *)s, len, &i));
    wpush(&b, 0);
    return b.v;
}

char *qv_wide_to_utf8(const uint16_t *s, size_t n, size_t *len)
{
    /* Three bytes per unit is the most UTF-8 ever needs. */
    char  *out = qv_alloc(n * 3 + 1);
    size_t o = 0, i = 0;

    while (i < n) {
        uint32_t cp = s[i++];
        if (cp >= 0xd800 && cp <= 0xdbff && i < n && s[i] >= 0xdc00 && s[i] <= 0xdfff)
            cp = 0x10000 + ((cp - 0xd800) << 10) + (s[i++] - 0xdc00);
        else if (cp >= 0xd800 && cp <= 0xdfff)
            cp = 0xfffd; /* half a pair */
        if (cp < 0x80) {
            out[o++] = (char)cp;
        } else if (cp < 0x800) {
            out[o++] = (char)(0xc0 | (cp >> 6));
            out[o++] = (char)(0x80 | (cp & 0x3f));
        } else if (cp < 0x10000) {
            out[o++] = (char)(0xe0 | (cp >> 12));
            out[o++] = (char)(0x80 | ((cp >> 6) & 0x3f));
            out[o++] = (char)(0x80 | (cp & 0x3f));
        } else {
            out[o++] = (char)(0xf0 | (cp >> 18));
            out[o++] = (char)(0x80 | ((cp >> 12) & 0x3f));
            out[o++] = (char)(0x80 | ((cp >> 6) & 0x3f));
            out[o++] = (char)(0x80 | (cp & 0x3f));
        }
    }
    out[o] = 0;
    if (len)
        *len = o;
    return out;
}

void qv_line_free(qv_line *l)
{
    free(l->text);
    free(l->spans);
    l->text = NULL;
    l->spans = NULL;
    l->len = l->nspans = 0;
}

void qv_verbs_free(qv_verbs *vs)
{
    uint32_t i;
    for (i = 0; i < vs->n; i++)
        free(vs->v[i].label);
    free(vs->v);
    vs->v = NULL;
    vs->n = 0;
}

int qv_verbs_equal(const qv_verbs *a, const qv_verbs *b)
{
    uint32_t i;
    if (a->n != b->n)
        return 0;
    for (i = 0; i < a->n; i++)
        if (a->v[i].id != b->v[i].id || a->v[i].attrs != b->v[i].attrs ||
            !qv_wsame(a->v[i].label, b->v[i].label))
            return 0;
    return 1;
}

qv_cmd *qv_cmd_new(int kind)
{
    qv_cmd *c = qv_alloc(sizeof *c);
    c->kind = kind;
    return c;
}

void qv_cmd_free(qv_cmd *c)
{
    uint32_t i;
    if (!c)
        return;
    free(c->text);
    qv_verbs_free(&c->verbs);
    for (i = 0; i < c->nlines; i++)
        qv_line_free(&c->lines[i]);
    free(c->lines);
    free(c);
}

/* ---- the order of the tabs ---- */

size_t qv_tab_insert_at(const int32_t *weights, size_t n, int32_t weight)
{
    /* The heavier goes first; of two that weigh the same, the first stays. */
    size_t i = 0;
    while (i < n && weights[i] >= weight)
        i++;
    return i;
}

int qv_tab_front_after_remove(size_t removed, size_t left)
{
    if (left == 0)
        return -1;
    return (int)(removed < left - 1 ? removed : left - 1);
}

/* ---- the text of a line ---- */

/* Width of a tab stop, in cells. */
#define TAB_STOP 8

/*
 * The sixteen named colors, chosen to read well on a dark background.
 * Black is drawn as a dark gray: true black would be invisible.
 */
static const uint32_t PALETTE[16] = {
    0x666666, 0xcd3131, 0x0dbc79, 0xe5e510, 0x3b8eea, 0xbc3fbc, 0x11a8cd, 0xe5e5e5,
    0x808080, 0xf14c4c, 0x23d18b, 0xf5f543, 0x6cb6ff, 0xd670d6, 0x29b8db, 0xffffff,
};

static uint32_t cube_level(uint32_t v)
{
    return v == 0 ? 0 : 55 + v * 40;
}

/* The 256-color table: the named ones, a 6x6x6 cube and a ramp of grays. */
static uint32_t indexed(uint32_t n)
{
    if (n <= 15)
        return PALETTE[n];
    if (n <= 231) {
        n -= 16;
        return (cube_level(n / 36) << 16) | (cube_level((n / 6) % 6) << 8) | cube_level(n % 6);
    }
    if (n <= 255) {
        uint32_t g = 8 + (n - 232) * 10;
        return (g << 16) | (g << 8) | g;
    }
    return PALETTE[7];
}

enum { PEN_DEFAULT, PEN_BASIC, PEN_FIXED };

/* The color state an SGR sequence acts on. */
typedef struct pen {
    int      kind;
    uint32_t value; /* which basic color, or the fixed rgb */
    int      bold;
} pen;

static int pen_rgb(pen p, uint32_t *rgb)
{
    switch (p.kind) {
    case PEN_BASIC:
        /* Bold makes a basic color bright. */
        *rgb = PALETTE[p.value + (p.bold ? 8 : 0)];
        return 1;
    case PEN_FIXED:
        *rgb = p.value;
        return 1;
    default:
        if (!p.bold)
            return 0;
        /* Bold with no color is drawn as bright white. */
        *rgb = PALETTE[15];
        return 1;
    }
}

/* Applies the parameters of one "ESC [ ... m" sequence. */
static void pen_apply(pen *p, const char *params, size_t len)
{
    uint32_t vals[32];
    size_t   n = 0, i = 0, k;

    /* Split at each ';'. What is not a number counts as zero. */
    for (;;) {
        uint64_t v = 0;
        int      ok = 0, bad = 0;
        while (i < len && params[i] != ';') {
            if (params[i] >= '0' && params[i] <= '9' && v <= 0xffffffffu) {
                v = v * 10 + (uint64_t)(params[i] - '0');
                ok = 1;
            } else {
                bad = 1;
            }
            i++;
        }
        if (n < 32)
            vals[n++] = (ok && !bad && v <= 0xffffffffu) ? (uint32_t)v : 0;
        if (i >= len)
            break;
        i++; /* the ';' */
    }

    for (k = 0; k < n; k++) {
        uint32_t v = vals[k];
        if (v == 0) {
            p->kind = PEN_DEFAULT, p->value = 0, p->bold = 0;
        } else if (v == 1) {
            p->bold = 1;
        } else if (v == 22) {
            p->bold = 0;
        } else if (v >= 30 && v <= 37) {
            p->kind = PEN_BASIC, p->value = v - 30;
        } else if (v == 39) {
            p->kind = PEN_DEFAULT;
        } else if (v >= 90 && v <= 97) {
            p->kind = PEN_FIXED, p->value = PALETTE[v - 90 + 8];
        } else if (v == 38 || v == 48) {
            /*
             * Extended colors carry their own parameters. Background
             * ones (48) are read and ignored, so they are not mistaken
             * for something else.
             */
            int      have = 0;
            uint32_t rgb = 0;
            if (k + 1 < n) {
                uint32_t mode = vals[++k];
                if (mode == 5) {
                    if (k + 1 < n) {
                        rgb = indexed(vals[++k]);
                        have = 1;
                    }
                } else if (mode == 2) {
                    if (k + 3 < n) {
                        rgb = ((vals[k + 1] & 0xff) << 16) | ((vals[k + 2] & 0xff) << 8) | (vals[k + 3] & 0xff);
                        have = 1;
                    }
                    k += 3;
                }
            }
            if (v == 38 && have)
                p->kind = PEN_FIXED, p->value = rgb;
        }
    }
}

typedef struct spans {
    qv_span *v;
    size_t   n, cap;
} spans;

/* Appends a code point in the color of the pen. */
static void put(wbuf *out, spans *sp, pen p, uint32_t cp)
{
    uint32_t units = (uint32_t)out->n;
    uint32_t n = wpush_cp(out, cp);
    uint32_t rgb;

    if (!pen_rgb(p, &rgb))
        return;
    if (sp->n && sp->v[sp->n - 1].rgb == rgb && sp->v[sp->n - 1].start + sp->v[sp->n - 1].len == units) {
        sp->v[sp->n - 1].len += n;
        return;
    }
    if (sp->n == sp->cap) {
        sp->cap = sp->cap ? sp->cap * 2 : 8;
        sp->v = qv_grow(sp->v, sp->cap * sizeof *sp->v);
    }
    sp->v[sp->n].start = units;
    sp->v[sp->n].len = n;
    sp->v[sp->n].rgb = rgb;
    sp->n++;
}

void qv_display_line(const uint8_t *data, uint32_t len, qv_line *line)
{
    wbuf   out = {0};
    spans  sp = {0};
    pen    p = {PEN_DEFAULT, 0, 0};
    size_t col = 0; /* in cells, for tab stops */
    size_t i = 0;

    /* Color does not carry over from one chunk to the next. */
    while (i < len) {
        uint32_t c = decode(data, len, &i);
        if (c == '\t') {
            size_t n = TAB_STOP - (col % TAB_STOP), k;
            for (k = 0; k < n; k++)
                put(&out, &sp, p, ' ');
            col += n;
        } else if (c == 0x1b) {
            size_t j = i;
            if (i < len && decode(data, len, &j) == '[') {
                /* Parameters and intermediates, then one final byte. */
                char   params[128];
                size_t np = 0;
                int    last = 0;
                i = j;
                while (i < len) {
                    uint32_t f = decode(data, len, &i);
                    if (f >= 0x40 && f <= 0x7e) {
                        last = (int)f;
                        break;
                    }
                    if (np < sizeof params)
                        params[np++] = f < 0x80 ? (char)f : '?';
                }
                if (last == 'm')
                    pen_apply(&p, params, np);
            } else if (i < len) {
                /* A two-character escape. */
                i = j;
            }
        } else if (c < 0x20 || (c >= 0x7f && c <= 0x9f)) {
            /* Line ends included: a chunk is always exactly one line. */
            put(&out, &sp, p, 0xb7);
            col++;
        } else {
            put(&out, &sp, p, c);
            col++;
        }
    }
    line->text = out.v;
    line->len = (uint32_t)out.n;
    line->spans = sp.v;
    line->nspans = (uint32_t)sp.n;
}

/* ---- the model of a console ---- */

static int is_note(const qv_line *l)
{
    return l->seq == 0;
}

static void reserve(qv_console *c, size_t n)
{
    if (n > c->cap) {
        size_t cap = c->cap ? c->cap : 64;
        while (cap < n)
            cap *= 2;
        c->lines = qv_grow(c->lines, cap * sizeof *c->lines);
        c->cap = cap;
    }
}

static void push_back(qv_console *c, qv_line l)
{
    reserve(c, c->len + 1);
    c->lines[c->len++] = l;
}

static void clear(qv_console *c)
{
    size_t i;
    for (i = 0; i < c->len; i++)
        qv_line_free(&c->lines[i]);
    c->len = 0;
}

static void drop_front(qv_console *c, size_t k)
{
    size_t i;
    for (i = 0; i < k; i++)
        qv_line_free(&c->lines[i]);
    memmove(c->lines, c->lines + k, (c->len - k) * sizeof *c->lines);
    c->len -= k;
}

static size_t max_top(const qv_console *c)
{
    return c->len > c->rows ? c->len - c->rows : 0;
}

static size_t min_size(size_t a, size_t b)
{
    return a < b ? a : b;
}

static int first_seq(const qv_console *c, uint64_t *seq)
{
    size_t i;
    for (i = 0; i < c->len; i++)
        if (!is_note(&c->lines[i])) {
            *seq = c->lines[i].seq;
            return 1;
        }
    return 0;
}

static int last_seq(const qv_console *c, uint64_t *seq)
{
    size_t i;
    for (i = c->len; i > 0; i--)
        if (!is_note(&c->lines[i - 1])) {
            *seq = c->lines[i - 1].seq;
            return 1;
        }
    return 0;
}

static int position_of(const qv_console *c, uint64_t seq, size_t *at)
{
    size_t i;
    for (i = 0; i < c->len; i++)
        if (c->lines[i].seq == seq) {
            *at = i;
            return 1;
        }
    return 0;
}

void qv_console_init(qv_console *c, size_t max_lines, uint32_t page)
{
    memset(c, 0, sizeof *c);
    c->page = page ? page : 1;
    /* Room for at least a page on each side of what is visible. */
    c->max_lines = max_lines > (size_t)c->page * 3 ? max_lines : (size_t)c->page * 3;
    c->rows = 1;
    c->status = QV_WAITING;
}

void qv_console_free(qv_console *c)
{
    clear(c);
    free(c->lines);
    c->lines = NULL;
    c->cap = 0;
}

static int following(const qv_console *c)
{
    return c->status == QV_FOLLOWING;
}

qv_req qv_console_follow(qv_console *c)
{
    qv_req r = {QUEVU_OP_FOLLOW, 0, c->page};
    if (!following(c))
        c->status = QV_WAITING;
    return r;
}

void qv_console_set_rows(qv_console *c, size_t rows)
{
    c->rows = rows ? rows : 1;
    if (following(c))
        c->top = max_top(c);
    else
        c->top = min_size(c->top, max_top(c));
}

int qv_console_pause(qv_console *c, qv_req *out)
{
    if (!following(c))
        return 0;
    c->status = QV_PAUSED;
    out->op = QUEVU_OP_PAUSE, out->seq = 0, out->n = 0;
    return 1;
}

static void req(qv_reqs *out, uint32_t op, uint64_t seq, uint32_t n)
{
    if (out->n < 4) {
        out->r[out->n].op = op;
        out->r[out->n].seq = seq;
        out->r[out->n].n = n;
        out->n++;
    }
}

/* The operations to move by position with: plain, or through the history. */
static void ops(const qv_console *c, uint32_t *before, uint32_t *after)
{
    *before = c->in_history ? QUEVU_OP_HISTORY_BEFORE : QUEVU_OP_BEFORE;
    *after = c->in_history ? QUEVU_OP_HISTORY_AFTER : QUEVU_OP_AFTER;
}

static void moved(qv_console *c, size_t target, int older, int newer, qv_reqs *out)
{
    size_t   margin;
    uint32_t before, after;
    uint64_t seq;

    out->n = 0;
    if (following(c)) {
        /* Already at the end: there is nowhere newer to go. */
        if (!older)
            return;
        /* Scrolling back means "do not move my text". */
        c->status = QV_PAUSED;
        req(out, QUEVU_OP_PAUSE, 0, 0);
    }
    c->top = min_size(target, max_top(c));
    margin = c->rows * 2;
    ops(c, &before, &after);
    if (older && c->top <= margin && !c->top_exhausted && !c->pending_before && first_seq(c, &seq)) {
        c->pending_before = 1;
        req(out, before, seq, c->page);
    }
    if (newer && c->top + c->rows + margin >= c->len && !c->pending_after && last_seq(c, &seq)) {
        c->pending_after = 1;
        req(out, after, seq, c->page);
    }
}

void qv_console_scroll_by(qv_console *c, ptrdiff_t delta, qv_reqs *out)
{
    size_t target;
    if (delta < 0) {
        size_t back = (size_t)(-(delta + 1)) + 1;
        target = c->top > back ? c->top - back : 0;
    } else {
        target = c->top + (size_t)delta;
        if (target < c->top)
            target = SIZE_MAX;
    }
    moved(c, target, delta < 0, delta > 0, out);
}

void qv_console_scroll_to(qv_console *c, size_t top, qv_reqs *out)
{
    moved(c, top, top < c->top, top > c->top, out);
}

static qv_line note(uint64_t missing)
{
    char    buf[64];
    qv_line l = {0};
    size_t  n, i;

    snprintf(buf, sizeof buf, "--- %llu %s not kept ---", (unsigned long long)missing, missing == 1 ? "chunk" : "chunks");
    n = strlen(buf);
    l.text = qv_alloc((n + 1) * sizeof *l.text);
    for (i = 0; i < n; i++)
        l.text[i] = (uint16_t)buf[i];
    l.len = (uint32_t)n;
    return l;
}

/* Appends at the end, leaving a note where positions jump. */
static void append(qv_console *c, qv_line l)
{
    uint64_t last;
    if (last_seq(c, &last)) {
        if (l.seq <= last) {
            qv_line_free(&l);
            return;
        }
        if (l.seq > last + 1)
            push_back(c, note(l.seq - last - 1));
    }
    push_back(c, l);
}

/* Drops the oldest lines over the limit and keeps the view on its text. */
static void trim_front(qv_console *c)
{
    size_t extra = c->len > c->max_lines ? c->len - c->max_lines : 0;
    if (extra) {
        drop_front(c, extra);
        c->top = c->top > extra ? c->top - extra : 0;
        c->origin += (int64_t)extra;
        /* What was just dropped can be asked for again. */
        c->top_exhausted = 0;
    }
}

uint64_t qv_console_find_from(const qv_console *c, int forward)
{
    size_t i, end;
    if (c->has_mark)
        return c->mark;
    end = min_size(c->len, c->top + c->rows);
    if (forward) {
        for (i = c->top; i < end; i++)
            if (!is_note(&c->lines[i]))
                return c->lines[i].seq - 1;
        return 0;
    }
    /* Going backwards, zero means the end of the log. */
    if (following(c))
        return 0;
    for (i = end; i > c->top; i--)
        if (!is_note(&c->lines[i - 1]))
            return c->lines[i - 1].seq + 1;
    return 0;
}

void qv_console_goto(qv_console *c, qv_line *lines, uint32_t n, uint64_t mark, int history, qv_reqs *out)
{
    size_t   at, third;
    uint32_t before, after, i;
    uint64_t first, last;

    out->n = 0;
    if (following(c))
        req(out, QUEVU_OP_PAUSE, 0, 0);
    c->status = QV_PAUSED;
    c->has_mark = 1;
    c->mark = mark;
    if (history) {
        c->in_history = 1;
        c->history_offered = 0;
    }
    third = c->rows / 3;
    if (position_of(c, mark, &at)) {
        /*
         * Already held: nothing is reloaded. In sight means comfortably
         * in sight: a line on the very edge of the view is brought in.
         */
        size_t margin = min_size(c->rows / 4, 2);
        if (at < c->top + margin || at + margin >= c->top + c->rows)
            c->top = min_size(at > third ? at - third : 0, max_top(c));
        for (i = 0; i < n; i++)
            qv_line_free(&lines[i]);
        return;
    }
    clear(c);
    for (i = 0; i < n; i++) {
        qv_line l = lines[i];
        memset(&lines[i], 0, sizeof lines[i]);
        append(c, l);
    }
    c->origin = 0;
    c->top_exhausted = 0;
    if (!position_of(c, mark, &at))
        at = 0;
    c->top = min_size(at > third ? at - third : 0, max_top(c));
    /* What is around the line found is fetched from whoever has it. */
    ops(c, &before, &after);
    c->pending_before = first_seq(c, &first);
    c->pending_after = c->pending_before;
    if (first_seq(c, &first) && last_seq(c, &last)) {
        req(out, before, first, c->page);
        req(out, after, last, c->page);
    }
}

void qv_console_offer_history(qv_console *c, int available)
{
    c->history_offered = available && !c->in_history && c->short_back;
}

int qv_console_request_history(qv_console *c, qv_req *out)
{
    uint64_t seq;
    if (!c->history_offered)
        return 0;
    c->history_offered = 0;
    c->in_history = 1;
    c->top_exhausted = 0;
    c->pending_before = 1;
    /* With nothing on screen, the history is entered from its end. */
    if (!first_seq(c, &seq))
        seq = (uint64_t)INT64_MAX;
    out->op = QUEVU_OP_HISTORY_BEFORE, out->seq = seq, out->n = c->page;
    return 1;
}

static void take_all(qv_console *c, qv_line *lines, uint32_t n)
{
    uint32_t i;
    for (i = 0; i < n; i++) {
        qv_line l = lines[i];
        memset(&lines[i], 0, sizeof lines[i]);
        append(c, l);
    }
}

static void free_all(qv_line *lines, uint32_t n)
{
    uint32_t i;
    for (i = 0; i < n; i++)
        qv_line_free(&lines[i]);
}

void qv_console_deliver(qv_console *c, uint32_t op, int full, qv_line *lines, uint32_t n)
{
    switch (op) {
    case QUEVU_OP_FOLLOW:
        clear(c);
        take_all(c, lines, n);
        c->status = QV_FOLLOWING;
        c->origin = 0;
        c->has_mark = 0;
        c->history_offered = 0;
        c->in_history = 0;
        c->short_back = 0;
        c->top_exhausted = 0;
        c->pending_before = 0;
        c->pending_after = 0;
        c->top = max_top(c);
        return;
    case QUEVU_OP_LIVE:
        /* Output sent before the source heard about a pause. */
        if (!following(c))
            break;
        take_all(c, lines, n);
        trim_front(c);
        c->top = max_top(c);
        return;
    case QUEVU_OP_PAUSE:
        if (full)
            c->status = QV_PAUSED_FULL;
        else if (c->status != QV_PAUSED_FULL)
            c->status = QV_PAUSED;
        break;
    case QUEVU_OP_BEFORE:
    case QUEVU_OP_HISTORY_BEFORE: {
        uint64_t first = 0;
        int      has_first = first_seq(c, &first);
        uint32_t i, kept = 0;
        size_t   added;
        qv_line  gap;
        int      has_gap = 0;

        c->pending_before = 0;
        c->short_back = n < c->page;
        /* Only what is older than what is held; the rest is not news. */
        for (i = 0; i < n; i++) {
            if (!has_first || lines[i].seq < first) {
                if (kept != i) {
                    lines[kept] = lines[i];
                    memset(&lines[i], 0, sizeof lines[i]);
                }
                kept++;
            } else {
                qv_line_free(&lines[i]);
            }
        }
        if (kept == 0) {
            /* The source has nothing further back. */
            c->top_exhausted = 1;
            return;
        }
        if (has_first && lines[kept - 1].seq + 1 < first) {
            gap = note(first - lines[kept - 1].seq - 1);
            has_gap = 1;
        }
        added = kept + (has_gap ? 1 : 0);
        reserve(c, c->len + added);
        memmove(c->lines + added, c->lines, c->len * sizeof *c->lines);
        memcpy(c->lines, lines, kept * sizeof *c->lines);
        memset(lines, 0, kept * sizeof *lines);
        if (has_gap)
            c->lines[kept] = gap;
        c->len += added;
        c->top += added;
        c->origin -= (int64_t)added;
        /* Make room at the far end; it can be asked for again. */
        while (c->len > c->max_lines)
            qv_line_free(&c->lines[--c->len]);
        c->top = min_size(c->top, max_top(c));
        return;
    }
    case QUEVU_OP_AFTER:
    case QUEVU_OP_HISTORY_AFTER:
        c->pending_after = 0;
        if (following(c))
            break;
        take_all(c, lines, n);
        trim_front(c);
        c->top = min_size(c->top, max_top(c));
        return;
    default:
        break;
    }
    free_all(lines, n);
}
