//! Platform-neutral core of the quevu visual part.
//!
//! Nothing in this crate knows about windows, toolkits or operating
//! systems. It holds the two things every backend shares:
//!
//! - [`console`]: the model of a pseudo-console. A console is a view, not
//!   a store: it keeps what the user can see and a margin around it, and
//!   asks for everything else by position.
//! - [`shared`]: the two one-way lanes between the host application and
//!   the UI thread. Commands go down to the UI, events come back up.
//!
//! A backend (Win32, GTK, Cocoa) draws what the model says and turns what
//! the user does into calls on the model. The logic of following, pausing
//! and moving by position is written, and tested, once.

pub mod console;
pub mod shared;
pub mod text;

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

/// State of a unit, numbered as in the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    Stopped = 0,
    Starting = 1,
    Running = 2,
    Stopping = 3,
    Failed = 4,
}

impl UnitState {
    pub fn from_u32(v: u32) -> UnitState {
        match v {
            1 => UnitState::Starting,
            2 => UnitState::Running,
            3 => UnitState::Stopping,
            4 => UnitState::Failed,
            _ => UnitState::Stopped,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            UnitState::Stopped => "stopped",
            UnitState::Starting => "starting",
            UnitState::Running => "running",
            UnitState::Stopping => "stopping",
            UnitState::Failed => "failed",
        }
    }

    /// Whether the switch of the unit offers "stop" (true) or "start".
    pub fn offers_stop(self) -> bool {
        matches!(self, UnitState::Running | UnitState::Starting)
    }

    /// Whether the unit is in the middle of a change and its switch
    /// should wait.
    pub fn is_busy(self) -> bool {
        matches!(self, UnitState::Starting | UnitState::Stopping)
    }
}
