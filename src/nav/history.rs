//! The document history stack.
//!
//! A [`Snapshot`] is everything needed to put a document back exactly as it was
//! left: which file, where the viewport was, where the cursor was, which
//! sections were folded, and which link was focused. Following a link pushes a
//! snapshot; `Backspace`/`-` pops one and restores it verbatim, moving the
//! document being left onto the forward stack so `+` can redo the move.
#![deny(unsafe_code)]

use std::path::PathBuf;

/// How many documents back the stack remembers. Snapshots are small (a path
/// plus a handful of heading ids), so this is generous on purpose; the cap only
/// exists so a long random walk cannot grow without bound.
pub const MAX_DEPTH: usize = 128;

/// A restorable document position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    /// First visible row (index into the visible-line list).
    pub top: usize,
    /// Cursor row (index into the visible-line list).
    pub cursor: usize,
    /// Folded heading ids, in fold order.
    pub collapsed: Vec<String>,
    /// Focused link, as an index into the document's link list.
    pub link: Option<usize>,
}

impl Snapshot {
    /// A snapshot of a document nobody has scrolled yet. Test-only: the pager
    /// always builds a full snapshot from live state.
    #[cfg(test)]
    pub fn of(path: PathBuf) -> Snapshot {
        Snapshot {
            path,
            ..Snapshot::default()
        }
    }
}

/// Back/forward stacks. The *current* document is never stored here; the caller
/// hands it over at the moment it stops being current.
#[derive(Debug, Default)]
pub struct History {
    back: Vec<Snapshot>,
    fwd: Vec<Snapshot>,
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    /// Record `current` as the document being left by a *new* navigation. Any
    /// forward history is discarded, exactly like a browser.
    pub fn push(&mut self, current: Snapshot) {
        self.fwd.clear();
        self.back.push(current);
        if self.back.len() > MAX_DEPTH {
            let excess = self.back.len() - MAX_DEPTH;
            self.back.drain(0..excess);
        }
    }

    /// Go back one document, handing over the snapshot of the one being left.
    pub fn back(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.back.pop()?;
        self.fwd.push(current);
        if self.fwd.len() > MAX_DEPTH {
            let excess = self.fwd.len() - MAX_DEPTH;
            self.fwd.drain(0..excess);
        }
        Some(prev)
    }

    /// Redo a `back`, if one is pending.
    pub fn forward(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.fwd.pop()?;
        self.back.push(current);
        Some(next)
    }

    /// How many documents deep we are; shown as `[3 back]` in the status bar.
    pub fn depth(&self) -> usize {
        self.back.len()
    }

    /// How much forward history a `back` left behind. Test-only.
    #[cfg(test)]
    pub fn forward_depth(&self) -> usize {
        self.fwd.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, top: usize, cursor: usize, folds: &[&str]) -> Snapshot {
        Snapshot {
            path: PathBuf::from(name),
            top,
            cursor,
            collapsed: folds.iter().map(|s| s.to_string()).collect(),
            link: Some(cursor),
        }
    }

    #[test]
    fn pop_restores_the_exact_state_that_was_pushed() {
        let mut h = History::new();
        let a = snap("a.md", 12, 40, &["alpha", "beta"]);
        h.push(a.clone());
        let b = snap("b.md", 0, 3, &[]);
        let restored = h.back(b).expect("one entry");
        assert_eq!(restored, a);
        assert_eq!(restored.top, 12);
        assert_eq!(restored.cursor, 40);
        assert_eq!(restored.collapsed, vec!["alpha", "beta"]);
        assert_eq!(restored.link, Some(40));
        assert_eq!(h.depth(), 0);
    }

    #[test]
    fn back_then_forward_round_trips() {
        let mut h = History::new();
        let a = snap("a.md", 5, 5, &["x"]);
        h.push(a.clone());
        let b = snap("b.md", 9, 9, &["y"]);
        let got_a = h.back(b.clone()).unwrap();
        assert_eq!(got_a, a);
        assert_eq!(h.forward_depth(), 1);
        let got_b = h.forward(got_a.clone()).unwrap();
        assert_eq!(got_b, b);
        assert_eq!(h.depth(), 1);
        assert_eq!(h.forward_depth(), 0);
    }

    #[test]
    fn a_new_push_discards_forward_history() {
        let mut h = History::new();
        h.push(snap("a.md", 0, 0, &[]));
        let _ = h.back(snap("b.md", 0, 0, &[]));
        assert_eq!(h.forward_depth(), 1);
        h.push(snap("a.md", 0, 0, &[]));
        assert_eq!(h.forward_depth(), 0);
        assert_eq!(h.depth(), 1);
    }

    #[test]
    fn empty_stacks_pop_to_nothing() {
        let mut h = History::new();
        assert!(h.back(snap("a.md", 0, 0, &[])).is_none());
        assert!(h.forward(snap("a.md", 0, 0, &[])).is_none());
        assert_eq!(h.depth(), 0);
    }

    #[test]
    fn depth_is_capped_and_drops_the_oldest() {
        let mut h = History::new();
        for i in 0..MAX_DEPTH + 10 {
            h.push(snap(&format!("{i}.md"), i, i, &[]));
        }
        assert_eq!(h.depth(), MAX_DEPTH);
        let oldest_kept = h.back(snap("cur.md", 0, 0, &[])).unwrap();
        assert_eq!(oldest_kept.cursor, MAX_DEPTH + 9);
    }

}
