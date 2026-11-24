package qui

// Text names one of the texts the window shows that no tab, control or
// menu entry of the application carries: what a source tab says of its
// view and of its search. The window holds no word of any language; this
// package hands it the English below, and an application changes any of
// them with Window.SetText.
type Text int

// What a source tab says of its view, and the words on its bar. The
// window draws these; their numbers are the ones of quevu_ui.h.
const (
	// TextWaiting: the view has nothing to show yet.
	TextWaiting Text = 1
	// TextFollowing: the view follows the live output.
	TextFollowing Text = 2
	// TextPaused: the user holds the view.
	TextPaused Text = 3
	// TextPausedFull: the view fell behind and its source stopped sending.
	TextPausedFull Text = 4
	// TextHistoryOffer: recent output ends here and there is a history.
	TextHistoryOffer Text = 5
	// TextLooking: a search is running. %s is what it looks for.
	TextLooking Text = 6
	// TextFindCue: what the empty search box says.
	TextFindCue Text = 7
	// TextFrom and TextTo label the two ends of a time range.
	TextFrom Text = 8
	TextTo   Text = 9
	// TextHistory: the button that opens the history.
	TextHistory Text = 10
	// The entries of the menu of a source tab. An entry whose text is
	// empty is left out of the menu.
	TextMenuHold      Text = 11
	TextMenuCopy      Text = 12
	TextMenuFind      Text = 13
	TextMenuFindOlder Text = 14
	TextMenuFindNewer Text = 15
	TextMenuTime      Text = 16
)

// What this package says of a search. They are formats of fmt, with the
// values numbered so a translation can put them in another order.
const (
	// TextFindFailed: the search could not run. %[1]v is the error.
	TextFindFailed Text = 100 + iota
	// TextFound: a match. %[1]q is the text, %[2]s when its line entered
	// the log.
	TextFound
	// TextFindHistory: nothing in recent output and a history not looked
	// into. %[1]q is the text, %[2]d the lines examined.
	TextFindHistory
	// TextFindOutOfRange: nothing within the time range. As above.
	TextFindOutOfRange
	// TextFindEndOlder and TextFindEndNewer: nothing, and the log ran
	// out in that direction. As above.
	TextFindEndOlder
	TextFindEndNewer
	// TextFindMoreOlder and TextFindMoreNewer: nothing so far, and more
	// to look into. %[1]q is the text, %[2]d the lines examined.
	TextFindMoreOlder
	TextFindMoreNewer
	// TextFindMoreOlderAt and TextFindMoreNewerAt: the same, when it is
	// known how far the step got: %[3]s is that moment.
	TextFindMoreOlderAt
	TextFindMoreNewerAt
	// TextTimeLayout: how a moment is written in the texts above, as a
	// layout of package time.
	TextTimeLayout
)

// defaultTexts is what the window says until the application says
// otherwise.
var defaultTexts = map[Text]string{
	TextWaiting:       "Waiting for output...",
	TextFollowing:     "Following the live output",
	TextPaused:        "View held (the application keeps running). Press play or End to follow the live output",
	TextPausedFull:    "View held: it fell behind and stopped receiving (the application keeps running). Play jumps to the end",
	TextHistoryOffer:  "This is the oldest recent output. There is a history: press History to load it",
	TextLooking:       `Looking for "%s"...`,
	TextFindCue:       "Find in this log",
	TextFrom:          "From",
	TextTo:            "To",
	TextHistory:       "History",
	TextMenuHold:      "&Hold / follow the output\tEnd",
	TextMenuCopy:      "&Copy\tCtrl+C",
	TextMenuFind:      "&Find...\tCtrl+F",
	TextMenuFindOlder: "Find &older\tF3",
	TextMenuFindNewer: "Find &newer\tShift+F3",
	TextMenuTime:      "Show the &time of each line",

	TextFindFailed:      "The search failed: %[1]v",
	TextFound:           "Found %[1]q at %[2]s. F3 looks for the next older one, Shift+F3 newer; End goes back to the live output",
	TextFindHistory:     "No %[1]q in recent output (%[2]d lines examined). The history has not been searched: press History to look there",
	TextFindOutOfRange:  "No %[1]q within the time range (%[2]d lines examined)",
	TextFindEndOlder:    "No %[1]q: reached the beginning of what is kept (%[2]d lines examined)",
	TextFindEndNewer:    "No %[1]q: reached the end of the log (%[2]d lines examined)",
	TextFindMoreOlder:   "No %[1]q in %[2]d lines. F3 keeps looking from there",
	TextFindMoreNewer:   "No %[1]q in %[2]d lines. Shift+F3 keeps looking from there",
	TextFindMoreOlderAt: "No %[1]q in %[2]d lines back to %[3]s. F3 keeps looking from there",
	TextFindMoreNewerAt: "No %[1]q in %[2]d lines up to %[3]s. Shift+F3 keeps looking from there",
	TextTimeLayout:      "2006-01-02 15:04:05",
}

// firstOwnText is where the texts of this package begin: the ones below
// it are drawn by the window and have to be handed to it.
const firstOwnText = 100

// SetText changes one of the texts of the window. It may be called
// before Run or while the window is open.
func (w *Window) SetText(id Text, text string) {
	w.mu.Lock()
	w.texts[id] = text
	w.mu.Unlock()
	if id < firstOwnText {
		uiText(uint32(id), text)
	}
}

// text returns a text as it stands now.
func (w *Window) text(id Text) string {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.texts[id]
}
