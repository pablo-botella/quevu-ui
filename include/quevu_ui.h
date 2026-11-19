/*
 * quevu_ui.h - C interface of the quevu visual part.
 *
 * The visual part is a window with the switches of an application (a
 * global one and one per unit) and its screens (a tab per log, each a
 * pseudo-console). It ships as a dynamic library: quevu_ui.dll on
 * Windows, libquevu_ui.so on Linux, libquevu_ui.dylib on macOS.
 *
 * Rules of the interface:
 *
 * - quevu_ui_run blocks the calling thread for the life of the window.
 *   On macOS that must be the main thread of the process, so a portable
 *   host calls it from the main thread everywhere.
 * - Every other function may be called from any thread, before or while
 *   quevu_ui_run is running, and never waits for the window.
 * - Nothing calls back into the host. What the user does is read with
 *   quevu_ui_next_event, from a thread of the host's choosing.
 * - Strings are NUL-terminated UTF-8 and are copied before the call
 *   returns. So is chunk data.
 */
#ifndef QUEVU_UI_H
#define QUEVU_UI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Version of this interface, as returned by quevu_ui_abi. */
#define QUEVU_UI_ABI 2u

/* Operations of a feed: what a console asks for and what it is given. */
#define QUEVU_OP_LIVE   1u /* new chunks, while the console follows      */
#define QUEVU_OP_FOLLOW 2u /* the last n chunks, then the live output    */
#define QUEVU_OP_PAUSE  3u /* the "do not disturb" sign                  */
#define QUEVU_OP_TAIL   4u /* the last n chunks                          */
#define QUEVU_OP_BEFORE 5u /* the n chunks that precede seq              */
#define QUEVU_OP_AFTER  6u /* the n chunks that follow seq               */

/* States of a unit. */
#define QUEVU_UNIT_STOPPED  0u
#define QUEVU_UNIT_STARTING 1u
#define QUEVU_UNIT_RUNNING  2u
#define QUEVU_UNIT_STOPPING 3u
#define QUEVU_UNIT_FAILED   4u

/* Kinds of event. */
#define QUEVU_EV_READY           1u /* the window exists                           */
#define QUEVU_EV_UNIT_START      2u /* id: unit whose switch went to "start"       */
#define QUEVU_EV_UNIT_STOP       3u /* id: unit whose switch went to "stop"        */
#define QUEVU_EV_START_ALL       4u /* the global switch, on                       */
#define QUEVU_EV_STOP_ALL        5u /* the global switch, off                      */
#define QUEVU_EV_CONSOLE_REQUEST 6u /* id: tab; op, seq, n: what the console asks  */
#define QUEVU_EV_MENU            7u /* id: application menu entry chosen           */
#define QUEVU_EV_CLOSE           8u /* the user asked to close the application     */
#define QUEVU_EV_CONSOLE_FIND    9u /* id: tab; op: 1 newer, 0 older; seq: from    */

/* A chunk of output: one line of a console. */
typedef struct quevu_ui_chunk {
    uint64_t       seq;     /* position in its log, from 1          */
    int64_t        time_ns; /* nanoseconds since the Unix epoch     */
    const uint8_t *data;    /* the bytes; need not be text          */
    uint32_t       len;     /* how many                             */
} quevu_ui_chunk;

/* Something the user did. */
typedef struct quevu_ui_event {
    uint32_t kind; /* QUEVU_EV_*                                    */
    uint32_t id;   /* unit, tab or menu entry, depending on kind    */
    uint32_t op;   /* QUEVU_OP_*, for a console request             */
    uint32_t n;    /* how many chunks, for a console request        */
    uint64_t seq;  /* from which position, for a console request    */
} quevu_ui_event;

/* The version of the interface the library implements. */
uint32_t quevu_ui_abi(void);

/*
 * Opens the window and runs its event loop on the calling thread until
 * quevu_ui_quit. Returns 0 on a clean end, -1 if this system has no
 * visual part, another value if the window could not be created.
 */
int32_t quevu_ui_run(const char *title);

/* Ends the event loop: quevu_ui_run returns. */
void quevu_ui_quit(void);

/* Adds a switch for a unit and returns its identifier. */
uint32_t quevu_ui_unit_add(const char *name);

/* Shows the state of a unit; err is shown next to a failed one. */
void quevu_ui_unit_state(uint32_t unit, uint32_t state, const char *err);

/*
 * Adds a tab with a pseudo-console and returns its identifier. The
 * console asks for its first page at once (QUEVU_EV_CONSOLE_REQUEST with
 * QUEVU_OP_FOLLOW).
 */
uint32_t quevu_ui_console_add(const char *title);

/*
 * Hands a console what came down its data lane. op is the operation
 * being answered, or QUEVU_OP_LIVE for live output; full marks a pause
 * the source decided by itself because the console fell behind.
 */
void quevu_ui_console_deliver(uint32_t tab, uint32_t op, uint32_t full,
                              const quevu_ui_chunk *chunks, uint32_t n);

/* Sets the note shown next to the state of a console. */
void quevu_ui_console_note(uint32_t tab, const char *note);

/*
 * Searching. A log may hold millions of chunks, so a search is a step:
 * it starts at a position, walks one way, examines a bounded number of
 * chunks and stops. The host runs the step; the window only asks for it
 * and shows how it went.
 *
 * On QUEVU_EV_CONSOLE_FIND the host reads the text with
 * quevu_ui_console_find_text, searches from ev.seq in the direction of
 * ev.op, and answers with quevu_ui_console_goto if it found a line, and
 * with quevu_ui_console_find_status either way.
 */

/*
 * Reads the text the console of a tab is looking for. Copies at most
 * cap - 1 bytes of UTF-8 plus a NUL into buf and returns the length of
 * the whole text, which may be larger than what fitted.
 */
uint32_t quevu_ui_console_find_text(uint32_t tab, char *buf, uint32_t cap);

/*
 * Moves a console to the lines around something a search found: the
 * window is replaced by chunks, the view is held, and the line at
 * position mark is told apart from the rest.
 */
void quevu_ui_console_goto(uint32_t tab, const quevu_ui_chunk *chunks,
                           uint32_t n, uint64_t mark);

/*
 * Tells the user how a step of a search went. A non-zero resume is the
 * position the next step should start from if the user asks for the same
 * text again: the step ran out of budget, not out of log.
 */
void quevu_ui_console_find_status(uint32_t tab, const char *status,
                                  uint64_t resume);

/* Removes a tab. */
void quevu_ui_tab_remove(uint32_t tab);

/*
 * Adds an entry to the application menu and returns its identifier,
 * which comes back in a QUEVU_EV_MENU event when the user chooses it.
 */
uint32_t quevu_ui_menu_add(const char *label);

/*
 * Turns the icon in the notification area on or off. While it is on,
 * closing or minimizing the window hides it there instead.
 */
void quevu_ui_tray(uint32_t enabled);

/* Shows or hides the window. */
void quevu_ui_show(uint32_t visible);

/*
 * Waits for the next thing the user did. Returns 1 with *ev filled, 0 if
 * nothing happened within timeout_ms (negative waits without limit), or
 * -1 once the event loop has ended and nothing is left to read.
 */
int32_t quevu_ui_next_event(quevu_ui_event *ev, int32_t timeout_ms);

#ifdef __cplusplus
}
#endif

#endif /* QUEVU_UI_H */
