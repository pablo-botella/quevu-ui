//! The words the window shows on its own account.
//!
//! A source tab does a few things by itself: it follows the output or
//! holds the view, it searches, it offers the history. Those have to be
//! called something on the screen, and what they are called is the host's
//! decision: the window holds no word of any language. Each text has a
//! number; until the host sets it, it is empty and its place stays blank.

/// The view has nothing to show yet.
pub const WAITING: u32 = 1;
/// The view follows the live output.
pub const FOLLOWING: u32 = 2;
/// The user holds the view.
pub const PAUSED: u32 = 3;
/// The view fell behind and its source stopped sending.
pub const PAUSED_FULL: u32 = 4;
/// Recent output ends here and there is a history to ask for.
pub const HISTORY_OFFER: u32 = 5;
/// A search is running. `%s` is what it looks for.
pub const LOOKING: u32 = 6;
/// What the empty search box says.
pub const FIND_CUE: u32 = 7;
/// The label of the start of a time range.
pub const FROM: u32 = 8;
/// The label of the end of a time range.
pub const TO: u32 = 9;
/// The button that opens the history.
pub const HISTORY: u32 = 10;
/// Menu of a source tab: hold the view, or follow again.
pub const MENU_HOLD: u32 = 11;
/// Menu of a source tab: copy the selection.
pub const MENU_COPY: u32 = 12;
/// Menu of a source tab: go to the search box.
pub const MENU_FIND: u32 = 13;
/// Menu of a source tab: the next match towards older output.
pub const MENU_FIND_OLDER: u32 = 14;
/// Menu of a source tab: the next match towards newer output.
pub const MENU_FIND_NEWER: u32 = 15;
/// Menu of a source tab: show when each line entered the log.
pub const MENU_TIME: u32 = 16;

/// One more than the highest number in use.
pub const COUNT: u32 = 17;

/// The texts as the host has set them so far.
#[derive(Debug, Default)]
pub struct Texts {
    set: Vec<String>,
}

impl Texts {
    pub fn new() -> Texts {
        Texts { set: vec![String::new(); COUNT as usize] }
    }

    /// Sets a text. A number this version does not know is ignored, so a
    /// newer host can speak to an older window.
    pub fn set(&mut self, id: u32, text: String) {
        if let Some(slot) = self.set.get_mut(id as usize) {
            *slot = text;
        }
    }

    /// A text, or nothing if the host never set it.
    pub fn get(&self, id: u32) -> &str {
        self.set.get(id as usize).map(String::as_str).unwrap_or("")
    }

    /// A text with its `%s` replaced by `arg`.
    pub fn with(&self, id: u32, arg: &str) -> String {
        self.get(id).replacen("%s", arg, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_nobody_set_is_empty() {
        let t = Texts::new();
        assert_eq!(t.get(FOLLOWING), "");
        assert_eq!(t.get(9999), "");
        assert_eq!(t.with(LOOKING, "x"), "");
    }

    #[test]
    fn the_host_sets_them_and_the_gap_is_filled() {
        let mut t = Texts::new();
        t.set(LOOKING, "Buscando \"%s\"...".to_string());
        t.set(9999, "ignored".to_string());
        assert_eq!(t.with(LOOKING, "error"), "Buscando \"error\"...");
        assert_eq!(t.with(LOOKING, "50%s"), "Buscando \"50%s\"...");
        assert_eq!(t.get(9999), "");
    }
}
