//! Fold state, and the tree flattened into rows.
//!
//! # The walk is an explicit stack
//!
//! A document nested ten thousand deep must flatten without touching the call
//! stack, so [`Flat::extend`] is a loop over a `Vec` of `(node, next member)`
//! frames — the same shape the parser and the scanner use. It is also
//! *resumable*: it stops when it has the rows it was asked for or has spent its
//! byte budget, and carries on from the same frame next time. That is what
//! makes `len()` grow the way a CSV's row count grows, instead of the first
//! screen waiting for the last.
//!
//! # Fold state is a default plus exceptions
//!
//! Not a list of open nodes: `zR` on a 900MB document would then have to
//! enumerate every container in it, which is the one thing this reader must
//! never do. Instead [`Folds`] holds *whether nodes are open by default* and the
//! set of ids that disagree. Expanding everything is one boolean, and staying
//! lazy costs nothing.
//!
//! Ids are index paths (`/0/3/7`) rather than key paths, because duplicate keys
//! are kept and a fold id must be unique. The spelling is the shared one
//! ([`crate::source::jsonrow::ALL_OPEN`] and friends), so a fold id means the
//! same thing in the record reader.
#![deny(unsafe_code)]

use std::collections::HashSet;

use super::tree::{Doc, NodeId};

use crate::source::jsonrow::ALL_OPEN;

/// Which nodes are open.
#[derive(Debug, Default)]
pub struct Folds {
    default_open: bool,
    /// Ids whose state is the opposite of the default.
    flip: HashSet<String>,
}

impl Folds {
    /// Root open, everything under it folded (SPEC.md §JSON, "The tree").
    pub fn new() -> Folds {
        let mut flip = HashSet::new();
        flip.insert(String::new());
        Folds { default_open: false, flip }
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.default_open != self.flip.contains(id)
    }

    /// Returns true when something changed.
    pub fn set(&mut self, id: &str, open: bool) -> bool {
        if self.is_open(id) == open {
            return false;
        }
        match self.default_open == open {
            true => self.flip.remove(id),
            false => self.flip.insert(id.to_string()),
        };
        true
    }

    /// `zM` / `zR`. One boolean, so expanding everything stays lazy.
    pub fn all(&mut self, open: bool) {
        self.default_open = open;
        self.flip.clear();
    }

    /// Could anything below the root be open? When not — the state a document
    /// opens in — the walk skips even looking at a member's first byte.
    pub fn any_below(&self) -> bool {
        self.default_open || self.flip.iter().any(|id| !id.is_empty())
    }

    /// The answer [`Folds::is_open`] gives *every* node, when it gives them all
    /// the same one — which is the state `zR` and `zM` leave behind.
    ///
    /// Spelling a fold id costs a string as long as the node is deep, so on a
    /// document nested ten thousand levels asking the question the ordinary way
    /// once per row is quadratic. When there are no exceptions to the default
    /// there is nothing for the id to change, and the caller can skip building
    /// it at all.
    pub fn uniform(&self) -> Option<bool> {
        self.flip.is_empty().then_some(self.default_open)
    }

    pub fn state(&self) -> Vec<String> {
        let mut out: Vec<String> = self.flip.iter().cloned().collect();
        out.sort();
        if self.default_open {
            out.insert(0, ALL_OPEN.to_string());
        }
        out
    }

    pub fn restore(&mut self, state: Vec<String>) {
        self.default_open = state.iter().any(|s| s == ALL_OPEN);
        self.flip = state.into_iter().filter(|s| s != ALL_OPEN).collect();
    }
}

/// Slot sentinels: a row is either a container's opening line, its closing
/// line, or one of its members rendered whole.
const OPEN: u32 = u32::MAX;
const CLOSE: u32 = u32::MAX - 1;

/// One visible row: a node, and which part of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub node: NodeId,
    slot: u32,
}

/// What a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The container's own line: `▾ "users": [`, or the collapsed summary.
    Head,
    /// Member `i` of the node, shown whole — a scalar, or a folded container.
    Member(usize),
    /// The closing bracket of an open container.
    Tail,
}

impl Row {
    pub fn part(self) -> Part {
        match self.slot {
            OPEN => Part::Head,
            CLOSE => Part::Tail,
            i => Part::Member(i as usize),
        }
    }
}

/// The visible rows, grown on demand.
#[derive(Default)]
pub struct Flat {
    rows: Vec<Row>,
    /// The walk's own stack: `(node, next member index)`.
    stack: Vec<(NodeId, usize)>,
    started: bool,
    done: bool,
}

impl Flat {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn get(&self, row: usize) -> Option<Row> {
        self.rows.get(row).copied()
    }

    /// True when every row of the document has been found.
    pub fn done(&self) -> bool {
        self.done
    }

    /// Throw the row list away, keeping every index the tree has built. Called
    /// when the fold state changes: the rows are a function of the folds, the
    /// indexes are a function of the file.
    pub fn reset(&mut self) {
        self.rows.clear();
        self.stack.clear();
        self.started = false;
        self.done = false;
    }

    /// Find `want` rows, spending at most `budget` bytes of scanning.
    pub fn extend(&mut self, doc: &mut Doc, folds: &Folds, want: usize, budget: u64) {
        if !self.started {
            self.start(doc, folds);
        }
        let probe = folds.any_below();
        let from = doc.walked();
        while self.rows.len() < want && !self.done {
            if doc.walked().saturating_sub(from) >= budget {
                return;
            }
            if !self.step(doc, folds, probe) {
                return;
            }
        }
    }

    /// The root row, and the frame under it when the root is an open container.
    fn start(&mut self, doc: &mut Doc, folds: &Folds) {
        self.started = true;
        let Some(root) = doc.root() else {
            self.done = true;
            return;
        };
        self.rows.push(Row { node: root, slot: OPEN });
        let open = folds.is_open(&doc.fold_id(root));
        match doc.node(root).shape.is_container() && open {
            true => self.stack.push((root, 0)),
            false => self.done = true,
        }
    }

    /// One row, or one slice of scanning. False when there is nothing more to
    /// do right now — either the document is finished or the scan needs another
    /// budget.
    fn step(&mut self, doc: &mut Doc, folds: &Folds, probe: bool) -> bool {
        let Some(&(node, i)) = self.stack.last() else {
            self.done = true;
            return false;
        };
        let known = doc.index(node, i + 1, STEP_BYTES);
        if i >= known {
            if !doc.node(node).complete() {
                return false;
            }
            self.rows.push(Row { node, slot: CLOSE });
            self.stack.pop();
            self.done = self.stack.is_empty();
            return true;
        }
        if let Some(top) = self.stack.last_mut() {
            top.1 = i + 1;
        }
        match self.child(doc, folds, probe, node, i) {
            Some(child) => {
                self.rows.push(Row { node: child, slot: OPEN });
                self.stack.push((child, 0));
            }
            None => self.rows.push(Row { node, slot: i as u32 }),
        }
        true
    }

    /// The node for member `i`, when that member is a container the fold state
    /// says is open. In the default state `probe` is false and this does not
    /// even read the member's first byte.
    fn child(
        &self,
        doc: &mut Doc,
        folds: &Folds,
        probe: bool,
        node: NodeId,
        i: usize,
    ) -> Option<NodeId> {
        if !probe {
            return None;
        }
        let open = match folds.uniform() {
            Some(o) => o,
            None => folds.is_open(&crate::source::jsonrow::child_id(&doc.fold_id(node), i)),
        };
        if !open {
            return None;
        }
        doc.open_child(node, i)
    }
}

/// Bytes one member-indexing step may walk. Small enough that a frame with a
/// pathological member in it still returns, large enough that ordinary
/// documents index in one go.
const STEP_BYTES: u64 = 1 << 20;

#[cfg(test)]
#[path = "flat_tests.rs"]
mod tests;
