//! Platform-neutral core of the quevu visual part.
//!
//! Nothing in this crate knows about windows, toolkits or operating
//! systems. It holds what every backend shares:
//!
//! - [`console`]: the model of a pseudo-console. A console is a view, not
//!   a store: it keeps what the user can see and a margin around it, and
//!   asks for everything else by position.
//! - [`shared`]: the two one-way lanes between the host application and
//!   the UI thread. Commands go down to the UI, events come back up.
//! - [`tabs`]: where a tab goes among the others, by its weight.
//! - [`texts`]: the words the window shows on its own account, which are
//!   the host's to set.
//!
//! A backend (Win32, GTK, Cocoa) draws what the model says and turns what
//! the user does into calls on the model. The logic of following, pausing
//! and moving by position is written, and tested, once.
//!
//! The window has no language and decides nothing. Every word on it comes
//! from the host, and so does every tab, control, verb and menu entry; what
//! the user does with them goes back to the host as an event.

pub mod console;
pub mod shared;
pub mod tabs;
pub mod text;
pub mod texts;

/// Operations of a feed, as numbered on the wire and in the C interface.
pub mod op {
    /// New chunks, sent while the console follows the log.
    pub const LIVE: u32 = 1;
    /// The last N chunks and, from there on, the live output.
    pub const FOLLOW: u32 = 2;
    /// The "do not disturb" sign.
    pub const PAUSE: u32 = 3;
    /// The last N chunks, without changing the state of the feed.
    pub const TAIL: u32 = 4;
    /// The N chunks that precede a position.
    pub const BEFORE: u32 = 5;
    /// The N chunks that follow a position.
    pub const AFTER: u32 = 6;
    /// BEFORE, answered from the history. Sent only because the user
    /// asked for the history.
    pub const HISTORY_BEFORE: u32 = 7;
    /// AFTER, for a console that is in the history: forwards through it
    /// and out of it, back into recent output.
    pub const HISTORY_AFTER: u32 = 8;
}

/// Attributes, as numbered in the C interface. Each kind of thing has its
/// own word of them; the bits not listed are reserved and must be zero.
pub mod attr {
    /// The light of a control goes on and off.
    pub const LIGHT_BLINK: u32 = 1;
    /// The light of a control is drawn as a ring: it is off.
    pub const LIGHT_HOLLOW: u32 = 2;

    /// A verb is shown but cannot be pressed.
    pub const VERB_DISABLED: u32 = 1;

    /// A menu entry is a line between others. It has no label.
    pub const MENU_SEPARATOR: u32 = 1;
    /// A menu entry carries the mark of something that is on.
    pub const MENU_CHECKED: u32 = 2;
    /// A menu entry is shown but cannot be chosen.
    pub const MENU_DISABLED: u32 = 4;
    /// A menu entry opens a menu of its own instead of being chosen.
    pub const MENU_SUBMENU: u32 = 8;
}

/// The two places a menu entry can hang from that are not another entry.
pub mod menu {
    /// The bar across the top of the window.
    pub const BAR: u32 = 0;
    /// The menu of the icon in the notification area.
    pub const TRAY: u32 = 0xffff_ffff;
}
