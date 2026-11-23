/*
 * quevu_ui.h - C interface of the quevu visual part.
 *
 * The visual part is a window with a row of tabs and nothing of its own.
 * The host opens every tab, names it and says where it goes by its
 * weight. A tab is one of two kinds: a source tab, with a pseudo-console
 * over a log, and a control tab, with a table of controls. A control is
 * a row with a light, a name, the text of its state and the buttons of
 * its verbs. The menu bar is the host's too.
 *
 * The window has no word of any language and decides nothing. States,
 * verbs, colors, menu entries and every text come from the host; what
 * the user does goes back as an event; and the window goes away only
 * when the host says so.
 *
 * It ships as a dynamic library: quevu_ui.dll on Windows,
 * libquevu_ui.so on Linux, libquevu_ui.dylib on macOS.
 *
 * Rules of the interface:
 *
 * - quevu_ui_run blocks the calling thread for the life of the window.
 *   On macOS that must be the main thread of the process, so a portable
 *   host calls it from the main thread everywhere.
 * - Every other function may be called from any thread, before or while
 *   quevu_ui_run is running, and never waits for the window. What is
 *   asked for before quevu_ui_run is on the window when it first shows.
 * - Nothing calls back into the host. What the user does is read with
 *   quevu_ui_next_event, from a thread of the host's choosing.
 * - Strings are NUL-terminated UTF-8 and are copied before the call
 *   returns. So is chunk data, and so are the labels of verbs.
 * - Identifiers are never zero and never repeat: a tab, a control and a
 *   menu entry cannot be taken for one another.
 */
#ifndef QUEVU_UI_H
#define QUEVU_UI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Version of this interface, as returned by quevu_ui_abi. */
#define QUEVU_UI_ABI 5u

/* Operations of a feed: what a console asks for and what it is given. */
#define QUEVU_OP_LIVE   1u /* new chunks, while the console follows      */
#define QUEVU_OP_FOLLOW 2u /* the last n chunks, then the live output    */
#define QUEVU_OP_PAUSE  3u /* the "do not disturb" sign                  */
#define QUEVU_OP_TAIL   4u /* the last n chunks                          */
#define QUEVU_OP_BEFORE 5u /* the n chunks that precede seq              */
#define QUEVU_OP_AFTER  6u /* the n chunks that follow seq               */
#define QUEVU_OP_HISTORY_BEFORE 7u /* before, from the history: user asked */
#define QUEVU_OP_HISTORY_AFTER  8u /* after, through the history and out   */

/*
 * A console escalates in three steps: its own memory; recent output,
 * which its source gives without being asked twice; and the history,
 * which is only ever asked for because the user said so. The window
 * offers the history when the source says there is one, and sends the
 * two HISTORY operations only after the user accepted.
 */

/* Flags. */
#define QUEVU_FLAG_FULL    1u /* the source paused the console by itself  */
#define QUEVU_FLAG_HISTORY 2u /* a history exists / is involved (see uses) */

/*
 * Attributes. Each kind of thing has its own word of them; the bits not
 * listed are reserved and must be zero.
 */
#define QUEVU_LIGHT_BLINK  1u /* the light goes on and off               */
#define QUEVU_LIGHT_HOLLOW 2u /* the light is drawn as a ring: it is off */

#define QUEVU_VERB_DISABLED 1u /* shown, but cannot be pressed           */

#define QUEVU_MENU_SEPARATOR 1u /* a line between entries; no label      */
#define QUEVU_MENU_CHECKED   2u /* carries the mark of something on      */
#define QUEVU_MENU_DISABLED  4u /* shown, but cannot be chosen           */
#define QUEVU_MENU_SUBMENU   8u /* opens a menu of its own               */

/* Where a menu entry hangs from, when it is not from another entry. */
#define QUEVU_MENU_BAR  0u          /* the bar across the top of the window */
#define QUEVU_MENU_TRAY 0xffffffffu /* the menu of the notification icon    */

/*
 * The texts the window shows on its own account: what a source tab says
 * of its view and of its search. Each is empty, and its place blank,
 * until the host sets it with quevu_ui_text. A number this version does
 * not know is ignored.
 */
#define QUEVU_TEXT_WAITING          1u /* the view has nothing to show yet   */
#define QUEVU_TEXT_FOLLOWING        2u /* the view follows the live output   */
#define QUEVU_TEXT_PAUSED           3u /* the user holds the view            */
#define QUEVU_TEXT_PAUSED_FULL      4u /* it fell behind; its source stopped */
#define QUEVU_TEXT_HISTORY_OFFER    5u /* recent output ends; history exists */
#define QUEVU_TEXT_LOOKING          6u /* a search runs; %s is its text      */
#define QUEVU_TEXT_FIND_CUE         7u /* what the empty search box says     */
#define QUEVU_TEXT_FROM             8u /* label: start of a time range       */
#define QUEVU_TEXT_TO               9u /* label: end of a time range         */
#define QUEVU_TEXT_HISTORY         10u /* the button that opens the history  */
#define QUEVU_TEXT_MENU_HOLD       11u /* tab menu: hold / follow the view   */
#define QUEVU_TEXT_MENU_COPY       12u /* tab menu: copy the selection       */
#define QUEVU_TEXT_MENU_FIND       13u /* tab menu: go to the search box     */
#define QUEVU_TEXT_MENU_FIND_OLDER 14u /* tab menu: next match, older        */
#define QUEVU_TEXT_MENU_FIND_NEWER 15u /* tab menu: next match, newer        */
#define QUEVU_TEXT_MENU_TIME       16u /* tab menu: show the time of a line  */

/*
 * Kinds of event. Numbers 2 to 5 belonged to the fixed switches of
 * earlier versions and are not used.
 */
#define QUEVU_EV_READY           1u /* the window exists                           */
#define QUEVU_EV_CONSOLE_REQUEST 6u /* id: tab; op, seq, n: what the console asks  */
#define QUEVU_EV_MENU            7u /* id: menu entry chosen                       */
#define QUEVU_EV_CLOSE           8u /* the user asked to close the window          */
#define QUEVU_EV_CONSOLE_FIND    9u /* id: tab; op bit 0: newer; bit 1: history    */
                                    /* allowed by the user; seq: from              */
#define QUEVU_EV_CONTROL_VERB   10u /* id: control; op: verb pressed               */
#define QUEVU_EV_TAB_VERB       11u /* id: control tab; op: verb pressed           */

/* A chunk of output: one line of a console. */
typedef struct quevu_ui_chunk {
    uint64_t       seq;     /* position in its log, from 1          */
    int64_t        time_ns; /* nanoseconds since the Unix epoch     */
    const uint8_t *data;    /* the bytes; need not be text          */
    uint32_t       len;     /* how many                             */
} quevu_ui_chunk;

/* A verb: a button with the host's words on it. */
typedef struct quevu_ui_verb {
    uint32_t    id;    /* what comes back when it is pressed        */
    uint32_t    attrs; /* QUEVU_VERB_*                              */
    const char *label; /* the words on the button                   */
} quevu_ui_verb;

/* Something the user did. */
typedef struct quevu_ui_event {
    uint32_t kind; /* QUEVU_EV_*                                    */
    uint32_t id;   /* tab, control or menu entry, depending on kind */
    uint32_t op;   /* QUEVU_OP_* of a console request; verb pressed */
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

/*
 * Ends the event loop: quevu_ui_run returns. It is the only way the
 * window goes away; a user who asks to close it only sends
 * QUEVU_EV_CLOSE. Called while no window is open, it does nothing.
 */
void quevu_ui_quit(void);

/* Sets one of the texts the window shows on its own account. */
void quevu_ui_text(uint32_t id, const char *text);

/*
 * Tabs. No tab is fixed: the host opens and closes them when it wants.
 * A tab takes its place in the row by its weight: the heavier, the
 * nearer the start; of two that weigh the same, the one opened first
 * goes first. A negative weight sends a tab after those that weigh zero.
 * The first tab opened is in front until another is selected.
 */

/*
 * Opens a tab with a pseudo-console and returns its identifier. The
 * console asks for its first page at once (QUEVU_EV_CONSOLE_REQUEST with
 * QUEVU_OP_FOLLOW).
 */
uint32_t quevu_ui_console_add(const char *title, int32_t weight);

/* Opens a tab with a table of controls and returns its identifier. */
uint32_t quevu_ui_controls_add(const char *title, int32_t weight);

/* Changes the name of a tab. */
void quevu_ui_tab_title(uint32_t tab, const char *title);

/* Brings a tab to the front. */
void quevu_ui_tab_select(uint32_t tab);

/* Closes a tab, with everything on it. */
void quevu_ui_tab_remove(uint32_t tab);

/*
 * Sets what the status strip of a tab says on the host's account. On a
 * source tab it goes next to the state of the view; on a control tab it
 * is all the strip says.
 */
void quevu_ui_tab_note(uint32_t tab, const char *note);

/*
 * Sets the verbs of a control tab as a whole: the buttons of its bar, in
 * the order given. Pressing one sends QUEVU_EV_TAB_VERB. The list
 * replaces the one before it; verbs may be null when n is zero.
 */
void quevu_ui_tab_verbs(uint32_t tab, const quevu_ui_verb *verbs, uint32_t n);

/*
 * Controls. The window draws what it is told and understands none of
 * it: a control has the states and the verbs its host gives it.
 */

/*
 * Adds a row to a control tab and returns the identifier of the control.
 * It shows its name and an unlit light until its state is set.
 */
uint32_t quevu_ui_control_add(uint32_t tab, const char *name);

/*
 * Sets everything a control shows: the color of its light as
 * 0x00RRGGBB, the attributes of the light (QUEVU_LIGHT_*), the text of
 * its state and the verbs it offers from now on, in the order given.
 * Pressing one sends QUEVU_EV_CONTROL_VERB. A list of verbs equal to the
 * one the control already has leaves its buttons untouched.
 */
void quevu_ui_control_state(uint32_t control, uint32_t rgb, uint32_t attrs,
                            const char *state, const quevu_ui_verb *verbs,
                            uint32_t n);

/* Removes a control from its tab. */
void quevu_ui_control_remove(uint32_t control);

/*
 * Hands a console what came down its data lane. op is the operation
 * being answered, or QUEVU_OP_LIVE for live output. In flags,
 * QUEVU_FLAG_FULL marks a pause the source decided by itself because the
 * console fell behind, and QUEVU_FLAG_HISTORY an answer to
 * QUEVU_OP_BEFORE that ran short of recent output while a history
 * exists: the window then offers it to the user.
 */
void quevu_ui_console_deliver(uint32_t tab, uint32_t op, uint32_t flags,
                              const quevu_ui_chunk *chunks, uint32_t n);

/*
 * Shows, or stops showing, next to each line of a console when it entered
 * the log. The user can change it too, from the menu of the tab.
 */
void quevu_ui_console_time(uint32_t tab, uint32_t on);

/*
 * Searching. A log may hold millions of chunks, so a search is a step:
 * it starts at a position, walks one way, examines a bounded number of
 * chunks and stops. The host runs the step; the window only asks for it
 * and shows how it went.
 *
 * On QUEVU_EV_CONSOLE_FIND the host reads the text with
 * quevu_ui_console_find_text and the time range, if the user set one,
 * with quevu_ui_console_find_range, searches from ev.seq in the direction
 * of ev.op, and answers with quevu_ui_console_goto if it found a line, and
 * with quevu_ui_console_find_status either way.
 */

/*
 * Reads the text the console of a tab is looking for. Copies at most
 * cap - 1 bytes of UTF-8 plus a NUL into buf and returns the length of
 * the whole text, which may be larger than what fitted.
 */
uint32_t quevu_ui_console_find_text(uint32_t tab, char *buf, uint32_t cap);

/*
 * Reads the time range the console of a tab is searching within:
 * nanoseconds since the Unix epoch, with zero for no bound on that side.
 * Either pointer may be null.
 */
void quevu_ui_console_find_range(uint32_t tab, int64_t *since_ns,
                                 int64_t *until_ns);

/*
 * Moves a console to a line a search found: the line at position mark is
 * told apart and the view is held. chunks need only hold that line; the
 * console asks for what surrounds it. QUEVU_FLAG_HISTORY in flags says it
 * was found in the history.
 */
void quevu_ui_console_goto(uint32_t tab, const quevu_ui_chunk *chunks,
                           uint32_t n, uint64_t mark, uint32_t flags);

/*
 * Tells the user how a step of a search went, in the host's words. A
 * non-zero resume is the position the next step should start from if the
 * user asks for the same text again: the step ran out of budget, not out
 * of log. QUEVU_FLAG_HISTORY in flags offers the history: the step ran
 * out of recent output and did not look further.
 */
void quevu_ui_console_find_status(uint32_t tab, const char *status,
                                  uint64_t resume, uint32_t flags);

/*
 * Menus. The window has none of its own: a bar with no entries is not
 * shown.
 */

/*
 * Adds a menu entry and returns its identifier, which comes back in a
 * QUEVU_EV_MENU event when the user chooses it. parent is where it hangs
 * from: QUEVU_MENU_BAR, QUEVU_MENU_TRAY, or the identifier of an entry
 * added with QUEVU_MENU_SUBMENU.
 */
uint32_t quevu_ui_menu_add(uint32_t parent, const char *label, uint32_t attrs);

/*
 * Changes the label and the attributes of a menu entry. Whether it opens
 * a menu of its own does not change. Nothing changes by itself when an
 * entry is chosen: a mark is the host's to move.
 */
void quevu_ui_menu_set(uint32_t item, const char *label, uint32_t attrs);

/* Removes a menu entry, with whatever hangs from it. */
void quevu_ui_menu_remove(uint32_t item);

/*
 * Turns the icon in the notification area on or off. While it is on,
 * minimizing the window hides it there. Its menu is the one the host
 * hangs from QUEVU_MENU_TRAY.
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
