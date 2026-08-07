//! The lazily indexed document tree (SPEC.md §JSON).
//!
//! # What is in memory
//!
//! Never the document. A [`Doc`] owns an open file, one sliding read window
//! (shared with the CSV side — [`Reader`] is a byte window over a file and
//! knows nothing about either format), and a [`Node`] for each container that
//! has actually been *opened*. A node holds its members as byte ranges
//! ([`Members`], 16 bytes each), never as values.
//!
//! # What costs what
//!
//! * Opening a document: one `stat`, one 64-byte read to find where the root
//!   value starts. Nothing else, at any file size.
//! * Painting a row: one windowed read of that member's bytes, and a parse of
//!   *that member* — capped by [`PARSE_CAP`], so a 40MB string says how big it
//!   is instead of being loaded.
//! * Summarising a collapsed container (`{…5 keys}`): a structural walk of that
//!   container only, resumable and budgeted, cached in [`Doc::counts`] while the
//!   row is on screen. No parse, and no walk of anything it contains.
//! * Expanding a node: its own [`Scan`], run the same way. Laziness is at every
//!   level, so one object holding one enormous array opens instantly.
//!
//! Nothing here recurses on nesting: the scanner is a byte loop, and the walk
//! over the tree lives in [`super::flat`], where it is an explicit stack.
#![deny(unsafe_code)]

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::csv::read::{Reader, MAX_ROW_BYTES, WINDOW};
use crate::json::index::{root, Member, Members, Scan, Shape};

/// The most one member may be read and parsed for display.
///
/// The cap is per *member*, never per document (SPEC.md §JSON): a member past
/// it reports its size and this limit rather than being loaded. It is the same
/// number the CSV side uses for one row, for the same reason — no terminal can
/// show a megabyte, and a keystroke must not turn into a 40MB allocation.
pub const PARSE_CAP: u64 = MAX_ROW_BYTES as u64;

/// The smallest slice of a container the scanner reads at once, and the unit
/// the slices grow from. One page: enough for the members of an ordinary
/// object, small enough that a first screen of a huge array is not a window.
const CHUNK: usize = 4096;

/// Index of a node in [`Doc::nodes`]. Only *opened* containers get one.
pub type NodeId = u32;

/// The deepest container the tree will open.
///
/// The shared presentation limit ([`crate::source::jsonrow::MAX_DEPTH`]), so a
/// `.json` document and a `.jsonl` record draw the line in the same place: a
/// reader should not find that a shape is readable as a line of a log and not
/// as a file. Past it the container renders as the note
/// [`crate::source::jsonrow::too_deep`] rather than being opened — the "flat
/// render" SPEC.md §JSON allows for hostile nesting. It is also what keeps the
/// re-walk this module pays per level bounded; see the constant's own note.
pub const MAX_DEPTH: u16 = {
    let d = crate::source::jsonrow::MAX_DEPTH;
    assert!(d <= u16::MAX as usize);
    d as u16
};

/// An opened container.
pub struct Node {
    /// The container this hangs off, and which of its members it is. `None` for
    /// the root.
    ///
    /// The fold id (`/0/3/7`) and the readable path (`.users[3].name`) are both
    /// derived from this chain rather than stored, because storing either one
    /// whole costs a string as long as the node is deep — and a document nested
    /// N levels opens N nodes, so a stored path makes memory quadratic in the
    /// nesting. A 200KB file of nothing but `[` used 8.5GB that way.
    parent: Option<(NodeId, u32)>,
    /// This node's own step in the readable path: `.name`, `[3]`. Short by
    /// construction, and the only path text a node holds.
    seg: String,
    /// The key this node hangs off, decoded. `None` for the root and for an
    /// array element, neither of which has one.
    pub key: Option<String>,
    pub start: u64,
    /// One past the closing bracket. Known from the parent's index; for the
    /// root it is the end of the file until the scan proves otherwise.
    pub end: u64,
    pub shape: Shape,
    pub depth: u16,
    members: Members,
    /// `None` once every member is indexed.
    scan: Option<Scan>,
    /// Children that are themselves opened, by member index.
    kids: HashMap<u32, NodeId>,
}

impl Node {
    pub fn count(&self) -> usize {
        self.members.len()
    }

    /// Every member is indexed, so [`Node::count`] is the real count.
    pub fn complete(&self) -> bool {
        self.scan.is_none()
    }

    pub fn member(&self, i: usize) -> Option<Member> {
        self.members.get(i)
    }

    /// How far into this container the structural scan has got.
    pub fn scanned(&self) -> u64 {
        self.scan.as_ref().map(Scan::pos).unwrap_or(self.end)
    }
}

/// Where a node sits: everything about it that comes from its parent rather
/// than from its own bytes.
struct Place {
    parent: Option<(NodeId, u32)>,
    seg: String,
    key: Option<String>,
    depth: u16,
}

/// A structural count in progress, for a container that is collapsed.
struct Count {
    scan: Scan,
    n: usize,
    end: u64,
}

impl Count {
    fn done(&self) -> bool {
        self.scan.done() || self.scan.pos() >= self.end
    }
}

/// Collapsed-container counts held at once. The map is the current window's
/// worth of rows; past that it is dropped whole rather than grown, because a
/// count is cheap to start again and a cache that outlives the viewport is a
/// leak with a nicer name.
const COUNT_CACHE: usize = 2048;

/// The document: bytes, and the nodes opened so far.
pub struct Doc {
    reader: Reader,
    nodes: Vec<Node>,
    counts: HashMap<u64, Count>,
    /// `None` for a document that holds no value at all.
    root: Option<NodeId>,
    size: u64,
    /// Bytes of structural scanning done so far, over the whole document. The
    /// walk in [`super::flat`] budgets itself against this rather than each
    /// call guessing what it cost.
    walked: u64,
}

impl Doc {
    /// Open `path`. Stats it and reads at most [`HEAD`] bytes to find the root.
    pub fn open(path: &Path) -> io::Result<Doc> {
        let reader = Reader::open(path)?;
        Ok(Doc::new(reader))
    }

    /// A document over bytes that arrived on a pipe.
    pub fn memory(data: Vec<u8>) -> Doc {
        Doc::new(Reader::memory(data))
    }

    /// Bytes read to find where the root value starts: a BOM plus whatever
    /// whitespace a pretty-printer left in front of it.
    const HEAD: usize = 64;

    fn new(mut reader: Reader) -> Doc {
        let size = reader.size();
        let head = reader.chunk(0, Doc::HEAD).to_vec();
        let mut doc = Doc {
            reader,
            nodes: Vec::new(),
            counts: HashMap::new(),
            root: None,
            size,
            walked: 0,
        };
        if let Some((start, shape)) = root(&head, 0) {
            let at = Place { parent: None, seg: String::new(), key: None, depth: 0 };
            let id = doc.push_node(at, start, size, shape);
            doc.root = Some(id);
        }
        doc
    }

    fn push_node(&mut self, at: Place, start: u64, end: u64, shape: Shape) -> NodeId {
        let Place { parent, seg, key, depth } = at;
        let scan = shape
            .is_container()
            .then(|| Scan::new(start, shape == Shape::Object));
        self.nodes.push(Node {
            parent,
            seg,
            key,
            start,
            end,
            shape,
            depth,
            members: Members::new(start),
            scan,
            kids: HashMap::new(),
        });
        (self.nodes.len() - 1) as NodeId
    }

    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// The document's size in bytes. Only the tests ask: everything else works
    /// in byte *ranges* the index handed it.
    #[cfg(test)]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Bytes structurally scanned so far. The unit callers budget in.
    pub fn walked(&self) -> u64 {
        self.walked
    }

    /// How far the root's own scan has got, 0..=100. What the status bar shows
    /// while the document is still being discovered — the same honest "not yet"
    /// a CSV gives (SPEC.md §CSV, `End::Scanning`).
    pub fn progress(&self) -> u8 {
        let Some(root) = self.root.map(|r| self.node(r)) else {
            return 100;
        };
        let total = self.size.saturating_sub(root.start);
        if root.complete() || total == 0 {
            return 100;
        }
        ((root.scanned().saturating_sub(root.start).min(total) * 100) / total) as u8
    }

    // -- indexing --------------------------------------------------------------

    /// Index members of `id` until there are `want` of them, the container
    /// ends, or `budget` bytes have been walked. Returns the members now known.
    ///
    /// This is the whole driving surface, exactly as [`crate::csv::index`] has
    /// one: painting a screen asks for that screen, an idle tick asks for more,
    /// and `G` is nothing but a caller spending slice after slice.
    pub fn index(&mut self, id: NodeId, want: usize, budget: u64) -> usize {
        let size = self.size;
        let mut spent = 0u64;
        let mut step = 0u32;
        loop {
            let Doc { reader, nodes, walked, .. } = self;
            let node = &mut nodes[id as usize];
            if node.members.len() >= want || node.scan.is_none() || spent >= budget {
                return node.members.len();
            }
            let Some(mut scan) = node.scan.take() else {
                return node.members.len();
            };
            let from = scan.pos();
            let limit = node.end.min(size);
            let members = &mut node.members;
            if from >= limit {
                scan.finish(limit, &mut |m| members.push(m));
            } else {
                // The first slice is small and the next ones grow: finding the
                // four members a first screen needs must not cost a whole
                // window, while a full walk must not cost a syscall per row.
                let grow = (CHUNK << step.min(6)) as u64;
                let want_bytes = (limit - from).min(grow).min(WINDOW as u64) as usize;
                let chunk = reader.chunk(from, want_bytes);
                // A short read means the file ended under us: settle the
                // container here rather than spinning on an offset that will
                // never yield another byte.
                match chunk.is_empty() {
                    true => scan.finish(from, &mut |m| members.push(m)),
                    false => scan.feed(chunk, &mut |m| members.push(m)),
                }
            }
            let took = scan.pos().saturating_sub(from);
            spent += took;
            *walked += took;
            step += 1;
            match scan.done() {
                true => {
                    node.end = scan.end().unwrap_or(node.end);
                    node.scan = None;
                }
                false => node.scan = Some(scan),
            }
        }
    }

    /// How many members a collapsed container has, and whether that is final.
    ///
    /// The count comes from the structural index and never from parsing
    /// (SPEC.md §JSON: "summarising a node does not require parsing it"). A
    /// container too big to walk inside one budget reports what it has so far,
    /// which the row shows as `≥N`, and converges on the idle tick.
    pub fn count(&mut self, m: Member, budget: u64) -> (usize, bool) {
        let shape = self.shape_of(m);
        if !shape.is_container() {
            return (0, true);
        }
        if self.counts.len() > COUNT_CACHE {
            self.counts.clear();
        }
        let Doc { reader, counts, walked, .. } = self;
        let c = counts.entry(m.start).or_insert_with(|| Count {
            scan: Scan::new(m.start, shape == Shape::Object),
            n: 0,
            end: m.end,
        });
        advance(c, reader, budget, walked);
        (c.n, c.done())
    }

    /// Push every unfinished count along by `budget` bytes each. Called from
    /// the idle tick, so a `≥120 items` on screen settles into `120 items`.
    /// Returns true while any of them still has work left.
    pub fn extend_counts(&mut self, budget: u64) -> bool {
        let Doc { reader, counts, walked, .. } = self;
        let mut more = false;
        for c in counts.values_mut() {
            if c.done() {
                continue;
            }
            advance(c, reader, budget, walked);
            more |= !c.done();
        }
        more
    }

    /// The node for member `i` of `parent`, creating it the first time the
    /// member is opened. `None` when the member is not a container, or when it
    /// sits deeper than [`MAX_DEPTH`] — see [`Doc::too_deep`].
    pub fn open_child(&mut self, parent: NodeId, i: usize) -> Option<NodeId> {
        if let Some(&id) = self.nodes[parent as usize].kids.get(&(i as u32)) {
            return Some(id);
        }
        if self.nodes[parent as usize].depth >= MAX_DEPTH {
            return None;
        }
        let m = self.nodes[parent as usize].member(i)?;
        let shape = self.shape_of(m);
        if !shape.is_container() {
            return None;
        }
        let seg = self.segment(m, i);
        let p = &self.nodes[parent as usize];
        let at = Place {
            parent: Some((parent, i as u32)),
            seg,
            key: None,
            depth: p.depth + 1,
        };
        let child = self.push_node(at, m.start, m.end, shape);
        self.nodes[child as usize].key = self.key_text(m);
        self.nodes[parent as usize].kids.insert(i as u32, child);
        Some(child)
    }

    /// Whether member `i` of `parent` is a container the tree refuses to open
    /// because it sits past [`MAX_DEPTH`]. The renderer asks so the row can say
    /// so instead of showing a fold marker that does nothing.
    pub fn too_deep(&self, parent: NodeId) -> bool {
        self.nodes[parent as usize].depth >= MAX_DEPTH
    }

}

/// Naming a member — its bytes, its key, its path — is the other half of this
/// module; it lives next door so both stay under the file size limit.
#[path = "ident.rs"]
mod ident;

/// Walk one collapsed container a slice at a time.
fn advance(c: &mut Count, reader: &mut Reader, budget: u64, walked: &mut u64) {
    let mut spent = 0u64;
    while !c.done() && spent < budget {
        let from = c.scan.pos();
        // The caller's budget sets the granularity: a small budget must not be
        // overspent by a whole window on the first read.
        let grow = budget.saturating_sub(spent).max(CHUNK as u64);
        let want = (c.end.saturating_sub(from)).min(grow).min(WINDOW as u64) as usize;
        let chunk = reader.chunk(from, want);
        if chunk.is_empty() {
            c.scan.finish(from, &mut |_| {});
            return;
        }
        let n = &mut c.n;
        c.scan.feed(chunk, &mut |_| *n += 1);
        let step = c.scan.pos().saturating_sub(from);
        spent += step;
        *walked += step;
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
