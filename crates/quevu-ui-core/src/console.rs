//! The model of a pseudo-console.
//!
//! A console is a view, not a store. It holds a window of lines, bounded
//! in size, over a log that may be far longer, and it only cares about who
//! sends it the chunks. Everything outside the window is asked for by
//! position when the user moves, and dropped again when the user moves
//! away.
//!
//! It has two states:
//!
//! - **following**: the view is pinned to the end and new output is
//!   appended as it arrives.
//! - **paused**: the text stays where the user left it. Live output is
//!   not kept. This is the "do not disturb" sign, hung either by the user
//!   scrolling back or by the source when the console fell behind.
//!
//! Leaving a pause does not catch up with what was missed: the console
//! asks for the last page and follows from there.
//!
//! The model never talks to anybody. Its methods return the requests that
//! should travel up the request lane, and [`Console::deliver`] takes what
//! comes down the data lane. A backend only has to draw [`Console::lines`]
//! from [`Console::top`] and report what the user does.

use std::collections::VecDeque;

use crate::op;

/// One line of the view. A chunk is always exactly one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Position of the chunk in its log. Zero marks a line the console
    /// made up itself: a note about output that was not kept.
    pub seq: u64,
    /// When the chunk entered the log, in nanoseconds since the Unix
    /// epoch.
    pub time_ns: i64,
    /// The text, already made drawable (see [`crate::text`]).
    pub text: String,
}

impl Line {
    pub fn is_note(&self) -> bool {
        self.seq == 0
    }
}

/// A request for the source of the console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub op: u32,
    pub seq: u64,
    pub n: u32,
}

/// What the status strip of a console should say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not started yet, or waiting for the answer to a follow request.
    Waiting,
    /// Pinned to the end, showing live output.
    Following,
    /// The user holds the text still.
    Paused,
    /// The console fell behind and the source hung the sign.
    PausedFull,
}

pub struct Console {
    lines: VecDeque<Line>,
    max_lines: usize,
    page: u32,
    top: usize,
    rows: usize,
    origin: i64,
    status: Status,
    top_exhausted: bool,
    pending_before: bool,
    pending_after: bool,
}

impl Console {
    /// `max_lines` bounds the window held in memory; `page` is how many
    /// chunks are asked for at a time.
    pub fn new(max_lines: usize, page: u32) -> Console {
        let page = page.max(1);
        Console {
            lines: VecDeque::new(),
            // Room for at least a page on each side of what is visible.
            max_lines: max_lines.max(page as usize * 3),
            page,
            top: 0,
            rows: 1,
            origin: 0,
            status: Status::Waiting,
            top_exhausted: false,
            pending_before: false,
            pending_after: false,
        }
    }

    /// The window of lines currently held.
    pub fn lines(&self) -> &VecDeque<Line> {
        &self.lines
    }

    /// Index, in [`Console::lines`], of the first visible line.
    pub fn top(&self) -> usize {
        self.top
    }

    /// A stable address for the first line held: `origin() + i` names
    /// line `i` in a way that survives lines being added in front or
    /// dropped, until the next follow starts the window over. It is what
    /// a backend anchors a selection to.
    pub fn origin(&self) -> i64 {
        self.origin
    }

    /// How many lines fit in the view.
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn is_following(&self) -> bool {
        self.status == Status::Following
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.rows)
    }

    fn first_seq(&self) -> Option<u64> {
        self.lines.iter().find(|l| !l.is_note()).map(|l| l.seq)
    }

    fn last_seq(&self) -> Option<u64> {
        self.lines.iter().rev().find(|l| !l.is_note()).map(|l| l.seq)
    }

    /// The request that starts the console, and the one that resumes it
    /// after a pause: the last page, then live output.
    pub fn follow(&mut self) -> Request {
        if !self.is_following() {
            self.status = Status::Waiting;
        }
        Request { op: op::FOLLOW, seq: 0, n: self.page }
    }

    /// The view was resized to hold `rows` lines.
    pub fn set_rows(&mut self, rows: usize) {
        self.rows = rows.max(1);
        if self.is_following() {
            self.top = self.max_top();
        } else {
            self.top = self.top.min(self.max_top());
        }
    }

    /// The user wants the text to hold still (a selection is starting,
    /// for instance).
    pub fn pause(&mut self) -> Option<Request> {
        if self.is_following() {
            self.status = Status::Paused;
            Some(Request { op: op::PAUSE, seq: 0, n: 0 })
        } else {
            None
        }
    }

    /// The user scrolled by `delta` lines: negative towards older output.
    pub fn scroll_by(&mut self, delta: isize) -> Vec<Request> {
        let target = if delta < 0 {
            self.top.saturating_sub(delta.unsigned_abs())
        } else {
            self.top.saturating_add(delta as usize)
        };
        self.moved(target, delta < 0, delta > 0)
    }

    /// The user dragged the scroll bar to an absolute position.
    pub fn scroll_to(&mut self, top: usize) -> Vec<Request> {
        let older = top < self.top;
        let newer = top > self.top;
        self.moved(top, older, newer)
    }

    fn moved(&mut self, target: usize, older: bool, newer: bool) -> Vec<Request> {
        let mut out = Vec::new();
        if self.is_following() {
            if !older {
                // Already at the end: there is nowhere newer to go.
                return out;
            }
            // Scrolling back means "do not move my text".
            self.status = Status::Paused;
            out.push(Request { op: op::PAUSE, seq: 0, n: 0 });
        }
        self.top = target.min(self.max_top());
        let margin = self.rows * 2;
        if older && self.top <= margin && !self.top_exhausted && !self.pending_before {
            if let Some(seq) = self.first_seq() {
                self.pending_before = true;
                out.push(Request { op: op::BEFORE, seq, n: self.page });
            }
        }
        if newer && self.top + self.rows + margin >= self.lines.len() && !self.pending_after {
            if let Some(seq) = self.last_seq() {
                self.pending_after = true;
                out.push(Request { op: op::AFTER, seq, n: self.page });
            }
        }
        out
    }

    fn note(missing: u64) -> Line {
        let what = if missing == 1 { "chunk" } else { "chunks" };
        Line { seq: 0, time_ns: 0, text: format!("--- {missing} {what} not kept ---") }
    }

    /// Appends at the end, leaving a note where positions jump.
    fn append(&mut self, line: Line) {
        if let Some(last) = self.last_seq() {
            if line.seq <= last {
                return;
            }
            if line.seq > last + 1 {
                self.lines.push_back(Self::note(line.seq - last - 1));
            }
        }
        self.lines.push_back(line);
    }

    /// Drops the oldest lines over the limit and keeps the view on the
    /// same text.
    fn trim_front(&mut self) {
        let extra = self.lines.len().saturating_sub(self.max_lines);
        if extra > 0 {
            self.lines.drain(..extra);
            self.top = self.top.saturating_sub(extra);
            self.origin += extra as i64;
            // What was just dropped can be asked for again.
            self.top_exhausted = false;
        }
    }

    /// Something came down the data lane.
    pub fn deliver(&mut self, op: u32, full: bool, chunks: Vec<Line>) {
        match op {
            op::FOLLOW => {
                self.lines.clear();
                for c in chunks {
                    self.append(c);
                }
                self.status = Status::Following;
                self.origin = 0;
                self.top_exhausted = false;
                self.pending_before = false;
                self.pending_after = false;
                self.top = self.max_top();
            }
            op::LIVE => {
                // Output sent before the source heard about a pause.
                if !self.is_following() {
                    return;
                }
                for c in chunks {
                    self.append(c);
                }
                self.trim_front();
                self.top = self.max_top();
            }
            op::PAUSE => {
                if full {
                    self.status = Status::PausedFull;
                } else if self.status != Status::PausedFull {
                    self.status = Status::Paused;
                }
            }
            op::BEFORE => {
                self.pending_before = false;
                let first = self.first_seq();
                let mut older: Vec<Line> = match first {
                    Some(f) => chunks.into_iter().filter(|c| c.seq < f).collect(),
                    None => chunks,
                };
                if older.is_empty() {
                    // The source has nothing further back.
                    self.top_exhausted = true;
                    return;
                }
                if let (Some(f), Some(last)) = (first, older.last()) {
                    if last.seq + 1 < f {
                        older.push(Self::note(f - last.seq - 1));
                    }
                }
                let added = older.len();
                for c in older.into_iter().rev() {
                    self.lines.push_front(c);
                }
                self.top += added;
                self.origin -= added as i64;
                // Make room at the far end; it can be asked for again.
                while self.lines.len() > self.max_lines {
                    self.lines.pop_back();
                }
                self.top = self.top.min(self.max_top());
            }
            op::AFTER => {
                self.pending_after = false;
                if self.is_following() {
                    return;
                }
                for c in chunks {
                    self.append(c);
                }
                self.trim_front();
                self.top = self.top.min(self.max_top());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(from: u64, to: u64) -> Vec<Line> {
        (from..=to).map(|s| Line { seq: s, time_ns: 0, text: format!("l{s}") }).collect()
    }

    fn seqs(c: &Console) -> Vec<u64> {
        c.lines().iter().map(|l| l.seq).collect()
    }

    fn visible(c: &Console) -> Vec<u64> {
        c.lines().iter().skip(c.top()).take(c.rows()).map(|l| l.seq).collect()
    }

    /// A console of 10 rows that keeps 60 lines and asks for 20 at a time.
    fn started(upto: u64) -> Console {
        let mut c = Console::new(60, 20);
        c.set_rows(10);
        assert_eq!(c.follow(), Request { op: op::FOLLOW, seq: 0, n: 20 });
        assert_eq!(c.status(), Status::Waiting);
        c.deliver(op::FOLLOW, false, lines(upto - 19, upto));
        c
    }

    #[test]
    fn follows_pinned_to_the_end() {
        let mut c = started(100);
        assert_eq!(c.status(), Status::Following);
        assert_eq!(visible(&c), (91..=100).collect::<Vec<_>>());
        c.deliver(op::LIVE, false, lines(101, 103));
        assert_eq!(visible(&c), (94..=103).collect::<Vec<_>>());
    }

    #[test]
    fn the_window_is_bounded_while_following() {
        let mut c = started(100);
        c.deliver(op::LIVE, false, lines(101, 1000));
        assert_eq!(c.lines().len(), 60);
        assert_eq!(seqs(&c)[0], 941);
        assert_eq!(visible(&c), (991..=1000).collect::<Vec<_>>());
    }

    #[test]
    fn scrolling_back_holds_the_text_still() {
        let mut c = started(100);
        let reqs = c.scroll_by(-3);
        // The sign goes up, and since the top of the window is near, the
        // previous page is asked for at once.
        assert_eq!(
            reqs,
            vec![
                Request { op: op::PAUSE, seq: 0, n: 0 },
                Request { op: op::BEFORE, seq: 81, n: 20 },
            ]
        );
        assert_eq!(c.status(), Status::Paused);
        let before = visible(&c);
        assert_eq!(before, (88..=97).collect::<Vec<_>>());
        // Live output still in flight is not kept and moves nothing.
        c.deliver(op::LIVE, false, lines(101, 150));
        assert_eq!(visible(&c), before);
        assert_eq!(*seqs(&c).last().unwrap(), 100);
    }

    #[test]
    fn older_pages_arrive_without_moving_what_is_on_screen() {
        let mut c = started(100);
        c.scroll_by(-3);
        let before = visible(&c);
        c.deliver(op::BEFORE, false, lines(61, 80));
        assert_eq!(visible(&c), before);
        assert_eq!(seqs(&c)[0], 61);
        // Only one request for older output is in flight at a time.
        let mut c = started(100);
        let first = c.scroll_by(-1);
        assert!(first.iter().any(|r| r.op == op::BEFORE));
        assert!(c.scroll_by(-1).is_empty());
        c.deliver(op::BEFORE, false, lines(61, 80));
        c.scroll_by(-30);
        // Near the top again: the next page back is requested.
        let mut c2 = started(100);
        c2.scroll_by(-1);
        c2.deliver(op::BEFORE, false, lines(61, 80));
        let again = c2.scroll_by(-25);
        assert_eq!(again, vec![Request { op: op::BEFORE, seq: 61, n: 20 }]);
    }

    #[test]
    fn the_far_end_is_dropped_to_make_room_and_asked_for_again() {
        let mut c = started(100);
        c.scroll_by(-1);
        c.deliver(op::BEFORE, false, lines(61, 80));
        c.scroll_by(-25);
        c.deliver(op::BEFORE, false, lines(41, 60));
        assert_eq!(c.lines().len(), 60);
        assert_eq!(seqs(&c), (41..=100).collect::<Vec<_>>());
        c.scroll_by(-25);
        let on_screen = visible(&c);
        c.deliver(op::BEFORE, false, lines(21, 40));
        // Still 60 lines: the newest 20 went away.
        assert_eq!(seqs(&c), (21..=80).collect::<Vec<_>>());
        assert_eq!(visible(&c), on_screen);
        // Moving forwards near the end brings them back.
        let reqs = c.scroll_by(40);
        assert_eq!(reqs, vec![Request { op: op::AFTER, seq: 80, n: 20 }]);
        c.deliver(op::AFTER, false, lines(81, 100));
        assert_eq!(*seqs(&c).last().unwrap(), 100);
        assert_eq!(c.lines().len(), 60);
    }

    #[test]
    fn an_empty_answer_ends_the_way_back() {
        let mut c = started(20);
        c.scroll_by(-1);
        c.deliver(op::BEFORE, false, Vec::new());
        // No more requests for older output: there is none.
        assert!(c.scroll_by(-5).is_empty());
        assert!(c.scroll_by(-5).is_empty());
        // A new follow starts over.
        c.follow();
        c.deliver(op::FOLLOW, false, lines(30, 49));
        assert!(c.scroll_by(-1).iter().any(|r| r.op == op::BEFORE));
    }

    #[test]
    fn resume_jumps_to_the_end() {
        let mut c = started(100);
        c.scroll_by(-5);
        assert_eq!(c.follow(), Request { op: op::FOLLOW, seq: 0, n: 20 });
        assert_eq!(c.status(), Status::Waiting);
        // It does not catch up: the last page replaces the window.
        c.deliver(op::FOLLOW, false, lines(981, 1000));
        assert_eq!(c.status(), Status::Following);
        assert_eq!(seqs(&c), (981..=1000).collect::<Vec<_>>());
        assert_eq!(visible(&c), (991..=1000).collect::<Vec<_>>());
    }

    #[test]
    fn the_source_can_hang_the_sign() {
        let mut c = started(100);
        c.deliver(op::PAUSE, true, Vec::new());
        assert_eq!(c.status(), Status::PausedFull);
        c.deliver(op::LIVE, false, lines(101, 110));
        assert_eq!(*seqs(&c).last().unwrap(), 100);
        // The answer to a pause the console itself asked for does not
        // hide that the source had already hung the sign.
        c.deliver(op::PAUSE, false, Vec::new());
        assert_eq!(c.status(), Status::PausedFull);
    }

    #[test]
    fn explicit_pause_is_asked_once() {
        let mut c = started(100);
        assert_eq!(c.pause(), Some(Request { op: op::PAUSE, seq: 0, n: 0 }));
        assert_eq!(c.pause(), None);
        assert_eq!(c.status(), Status::Paused);
    }

    #[test]
    fn scrolling_forwards_while_paused_asks_for_newer_output() {
        let mut c = started(100);
        c.pause();
        let reqs = c.scroll_by(1);
        assert_eq!(reqs, vec![Request { op: op::AFTER, seq: 100, n: 20 }]);
        c.deliver(op::AFTER, false, lines(101, 120));
        assert_eq!(*seqs(&c).last().unwrap(), 120);
        // The view did not jump: the user moves into the new lines.
        assert_eq!(visible(&c), (91..=100).collect::<Vec<_>>());
        // Nothing newer yet: the request is free to be made again.
        c.scroll_by(30);
        c.deliver(op::AFTER, false, Vec::new());
        assert!(c.scroll_by(1).iter().any(|r| r.op == op::AFTER));
    }

    #[test]
    fn output_that_was_not_kept_leaves_a_note() {
        let mut c = started(100);
        c.pause();
        c.scroll_by(1);
        // The source no longer has 101..=149.
        c.deliver(op::AFTER, false, lines(150, 160));
        let note = c.lines().iter().find(|l| l.is_note()).expect("a note");
        assert_eq!(note.text, "--- 49 chunks not kept ---");
        // And the same looking back.
        let mut c = started(100);
        c.scroll_by(-1);
        c.deliver(op::BEFORE, false, lines(50, 79));
        assert_eq!(c.lines().iter().filter(|l| l.is_note()).count(), 1);
        assert_eq!(c.lines()[30].text, "--- 1 chunk not kept ---");
    }

    #[test]
    fn repeated_chunks_are_not_shown_twice() {
        let mut c = started(100);
        c.deliver(op::LIVE, false, lines(98, 102));
        assert_eq!(seqs(&c), (81..=102).collect::<Vec<_>>());
        c.scroll_by(-1);
        c.deliver(op::BEFORE, false, lines(70, 90));
        assert_eq!(seqs(&c), (70..=102).collect::<Vec<_>>());
    }

    #[test]
    fn the_origin_names_a_line_through_every_change() {
        let mut c = started(100);
        // Line 95 sits at index 14 of 81..=100.
        let addr = c.origin() + 14;
        let at = |c: &Console| c.lines()[(addr - c.origin()) as usize].seq;
        assert_eq!(at(&c), 95);
        c.scroll_by(-1);
        c.deliver(op::BEFORE, false, lines(61, 80));
        assert_eq!(at(&c), 95);
        // Following again and overflowing the window drops lines in front.
        let mut c = started(100);
        let addr = c.origin() + 19;
        c.deliver(op::LIVE, false, lines(101, 150));
        assert_eq!(c.lines()[(addr - c.origin()) as usize].seq, 100);
    }

    #[test]
    fn resizing_keeps_the_end_in_view_while_following() {
        let mut c = started(100);
        c.set_rows(4);
        assert_eq!(visible(&c), (97..=100).collect::<Vec<_>>());
        c.set_rows(15);
        assert_eq!(visible(&c), (86..=100).collect::<Vec<_>>());
        // A view taller than the window shows everything from the top.
        c.set_rows(500);
        assert_eq!(c.top(), 0);
    }

    #[test]
    fn scroll_bar_positions() {
        let mut c = started(100);
        let reqs = c.scroll_to(0);
        assert!(reqs.iter().any(|r| r.op == op::PAUSE));
        assert_eq!(c.top(), 0);
        // Past the end is clamped.
        c.scroll_to(10_000);
        assert_eq!(visible(&c), (91..=100).collect::<Vec<_>>());
    }

    #[test]
    fn nothing_to_scroll_in_an_empty_console() {
        let mut c = Console::new(60, 20);
        c.set_rows(10);
        c.follow();
        c.deliver(op::FOLLOW, false, Vec::new());
        // The sign goes up, but there is no position to ask from.
        assert_eq!(c.scroll_by(-3), vec![Request { op: op::PAUSE, seq: 0, n: 0 }]);
        assert!(c.scroll_by(3).is_empty());
    }
}
