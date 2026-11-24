//go:build !windows || !cgo

package qui

// The door of a program with no window: built for a system the window
// does not exist for, or without cgo. Nothing is shown and nothing
// happens; Run says so at once.

const uiAvailable = false

// uiRun returns the code of a system with no visual part.
func uiRun(string) int32 { return -1 }

func uiQuit()                                             {}
func uiText(uint32, string)                               {}
func uiConsoleAdd(string, int32) uint32                   { return 0 }
func uiControlsAdd(string, int32) uint32                  { return 0 }
func uiTabTitle(uint32, string)                           {}
func uiTabSelect(uint32)                                  {}
func uiTabRemove(uint32)                                  {}
func uiTabNote(uint32, string)                            {}
func uiTabVerbs(uint32, []uiVerb)                         {}
func uiControlAdd(uint32, string) uint32                  { return 0 }
func uiControlState(_, _, _ uint32, _ string, _ []uiVerb) {}
func uiControlRemove(uint32)                              {}
func uiConsoleDeliver(_, _, _ uint32, _ []uiChunk)        {}
func uiConsoleTime(_, _ uint32)                           {}
func uiMenuAdd(uint32, string, uint32) uint32             { return 0 }
func uiMenuSet(uint32, string, uint32)                    {}
func uiMenuRemove(uint32)                                 {}
func uiTray(uint32)                                       {}
func uiShow(uint32)                                       {}
func uiFindText(uint32) string                            { return "" }
func uiFindRange(uint32) (int64, int64)                   { return 0, 0 }
func uiConsoleGoto(uint32, []uiChunk, uint64, uint32)     {}
func uiFindStatus(uint32, string, uint64, uint32)         {}

// uiNextEvent says the window is gone: there never was one.
func uiNextEvent(int32) (uiEvent, int32) { return uiEvent{}, -1 }
