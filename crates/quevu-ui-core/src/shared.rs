//! The two one-way lanes between the host application and the UI thread.
//!
//! The host talks to the visual part from any thread; the visual part
//! lives on one thread, the one that runs the event loop of the system.
//! They never call each other directly:
//!
//! - **Commands** go down. The host pushes one and wakes the UI thread,
//!   which drains the queue when it gets to it. The host never waits.
//! - **Events** come up. The UI thread pushes one, and a thread of the
//!   host that sits in [`Shared::next_event`] picks it up. The UI thread
//!   never waits either.
//!
//! No callbacks cross the boundary, which keeps the C interface trivial
//! to bind from any language.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::console::{Line, Request};
use crate::UnitState;

/// Something the host asks the visual part to do.
#[derive(Debug)]
pub enum Command {
    /// A new switch.
    UnitAdd { id: u32, name: String },
    /// A switch changed state.
    UnitState { id: u32, state: UnitState, err: String },
    /// A new tab holding a pseudo-console.
    ConsoleAdd { id: u32, title: String },
    /// Something for a console came down its data lane.
    /// `history` is the source's offer: recent output ran short and there
    /// is a history the user could ask for.
    Deliver { tab: u32, op: u32, full: bool, history: bool, lines: Vec<Line> },
    /// A note for the status strip of a console (the state of its queues,
    /// for instance).
    ConsoleNote { tab: u32, text: String },
    /// A tab goes away.
    TabRemove { id: u32 },
    /// A new entry in the application menu.
    MenuAdd { id: u32, label: String },
    /// Whether closing the window hides it in the system tray.
    Tray { enabled: bool },
    /// A search found a line: the console moves to the lines around it.
    /// `history` says it was found with the history open.
    Goto { tab: u32, lines: Vec<Line>, mark: u64, history: bool },
    /// What a step of a search has to say. A non-zero `resume` is where
    /// the next step should start if the user asks for the same thing
    /// again: the search ran out of budget, not out of log.
    /// `history` is an offer: the search ran out of recent output and
    /// there is a history it did not look into.
    FindStatus { tab: u32, text: String, resume: u64, history: bool },
    /// Shows or hides the window.
    Show { visible: bool },
    /// Ends the event loop.
    Quit,
}

/// Kinds of [`Event`], as numbered in the C interface.
pub mod event {
    /// The window exists and the event loop is running.
    pub const READY: u32 = 1;
    /// The user flipped the switch of unit `id` to "start".
    pub const UNIT_START: u32 = 2;
    /// The user flipped the switch of unit `id` to "stop".
    pub const UNIT_STOP: u32 = 3;
    /// The global switch, on.
    pub const START_ALL: u32 = 4;
    /// The global switch, off.
    pub const STOP_ALL: u32 = 5;
    /// The console of tab `id` asks its source for `op`, `seq`, `n`.
    pub const CONSOLE_REQUEST: u32 = 6;
    /// The user chose the application menu entry `id`.
    pub const MENU: u32 = 7;
    /// The user asked to close the application. The host decides what
    /// that means and when: usually an orderly stop, then quit.
    pub const CLOSE: u32 = 8;
    /// The user asked the console of tab `id` to look for a text: `op` is
    /// 1 to look towards newer output and 0 towards older, `seq` is where
    /// to start. Bit 1 of `op` set means the user allowed the history.
    /// The text itself is read with [`super::Shared::find_text`].
    pub const CONSOLE_FIND: u32 = 9;
}

/// What a console is looking for. Events are fixed in size, so this waits
/// in [`Shared`] for the host to read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindParams {
    pub text: String,
    /// The time range, in nanoseconds since the Unix epoch. Zero is no
    /// bound on that side.
    pub since_ns: i64,
    pub until_ns: i64,
}

/// Something the user did, on its way to the host.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: u32,
    pub id: u32,
    pub op: u32,
    pub n: u32,
    pub seq: u64,
}

impl Event {
    pub fn simple(kind: u32, id: u32) -> Event {
        Event { kind, id, ..Event::default() }
    }

    pub fn request(tab: u32, r: Request) -> Event {
        Event { kind: event::CONSOLE_REQUEST, id: tab, op: r.op, n: r.n, seq: r.seq }
    }
}

/// What [`Shared::next_event`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Event(Event),
    /// Nothing happened within the time given.
    Timeout,
    /// The event loop has ended and every event has been read.
    Ended,
}

type Waker = Box<dyn Fn() + Send + Sync>;

struct Events {
    queue: VecDeque<Event>,
    ended: bool,
}

/// The state both sides share. One per process.
pub struct Shared {
    commands: Mutex<VecDeque<Command>>,
    waker: Mutex<Option<Waker>>,
    events: Mutex<Events>,
    arrived: Condvar,
    next_id: AtomicU32,
    finds: Mutex<HashMap<u32, FindParams>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

impl Shared {
    pub fn new() -> Shared {
        Shared {
            commands: Mutex::new(VecDeque::new()),
            waker: Mutex::new(None),
            events: Mutex::new(Events { queue: VecDeque::new(), ended: false }),
            arrived: Condvar::new(),
            next_id: AtomicU32::new(1),
            finds: Mutex::new(HashMap::new()),
        }
    }

    /// A fresh identifier for a unit, a tab or a menu entry. Never zero.
    pub fn new_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Host side: queues a command and wakes the UI thread. Commands
    /// posted before the event loop starts wait for it.
    pub fn post(&self, cmd: Command) {
        self.commands.lock().unwrap().push_back(cmd);
        if let Some(wake) = self.waker.lock().unwrap().as_ref() {
            wake();
        }
    }

    /// UI side: takes every command queued so far, in order.
    pub fn drain(&self) -> Vec<Command> {
        self.commands.lock().unwrap().drain(..).collect()
    }

    /// UI side: the event loop is up. `wake` must make the UI thread call
    /// [`Shared::drain`] soon, and be callable from any thread.
    pub fn attach(&self, wake: Waker) {
        self.events.lock().unwrap().ended = false;
        *self.waker.lock().unwrap() = Some(wake);
    }

    /// UI side: the event loop is over.
    pub fn detach(&self) {
        *self.waker.lock().unwrap() = None;
        self.events.lock().unwrap().ended = true;
        self.arrived.notify_all();
    }

    /// UI side: the text the console of a tab is looking for. Events are
    /// fixed in size, so the text waits here for the host to read it.
    pub fn set_find(&self, tab: u32, params: FindParams) {
        self.finds.lock().unwrap().insert(tab, params);
    }

    /// Host side: the text the console of a tab is looking for.
    pub fn find_text(&self, tab: u32) -> String {
        self.find(tab).text
    }

    /// Host side: everything the console of a tab is looking for.
    pub fn find(&self, tab: u32) -> FindParams {
        self.finds.lock().unwrap().get(&tab).cloned().unwrap_or_default()
    }

    /// UI side: reports something the user did.
    pub fn emit(&self, ev: Event) {
        self.events.lock().unwrap().queue.push_back(ev);
        self.arrived.notify_one();
    }

    /// Host side: waits for the next event. A negative timeout waits
    /// without limit.
    pub fn next_event(&self, timeout_ms: i32) -> Next {
        let deadline = (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
        let mut ev = self.events.lock().unwrap();
        loop {
            if let Some(e) = ev.queue.pop_front() {
                return Next::Event(e);
            }
            if ev.ended {
                return Next::Ended;
            }
            match deadline {
                None => ev = self.arrived.wait(ev).unwrap(),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return Next::Timeout;
                    }
                    ev = self.arrived.wait_timeout(ev, d - now).unwrap().0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn commands_wait_for_the_loop_and_keep_their_order() {
        let s = Shared::new();
        s.post(Command::Tray { enabled: true });
        s.post(Command::Quit);
        let got = s.drain();
        assert!(matches!(got[0], Command::Tray { enabled: true }));
        assert!(matches!(got[1], Command::Quit));
        assert!(s.drain().is_empty());
    }

    #[test]
    fn posting_wakes_the_ui_thread_once_attached() {
        let s = Shared::new();
        let woken = Arc::new(AtomicUsize::new(0));
        s.post(Command::Quit);
        assert_eq!(woken.load(Ordering::SeqCst), 0);
        let w = woken.clone();
        s.attach(Box::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));
        s.post(Command::Quit);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        s.detach();
        s.post(Command::Quit);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn events_time_out_arrive_and_end() {
        let s = Arc::new(Shared::new());
        assert_eq!(s.next_event(10), Next::Timeout);
        s.emit(Event::simple(event::UNIT_STOP, 7));
        assert_eq!(s.next_event(0), Next::Event(Event::simple(event::UNIT_STOP, 7)));

        // A waiting host thread is woken by an event from the UI thread.
        let s2 = s.clone();
        let host = thread::spawn(move || s2.next_event(-1));
        thread::sleep(Duration::from_millis(20));
        s.emit(Event::simple(event::CLOSE, 0));
        assert_eq!(host.join().unwrap(), Next::Event(Event::simple(event::CLOSE, 0)));

        // The end of the loop is seen after the events still queued.
        s.emit(Event::simple(event::MENU, 3));
        s.detach();
        assert_eq!(s.next_event(-1), Next::Event(Event::simple(event::MENU, 3)));
        assert_eq!(s.next_event(-1), Next::Ended);
        assert_eq!(s.next_event(5), Next::Ended);
    }

    #[test]
    fn ids_are_never_zero_and_never_repeat() {
        let s = Shared::new();
        let a = s.new_id();
        let b = s.new_id();
        assert!(a != 0 && b != 0 && a != b);
    }
}
