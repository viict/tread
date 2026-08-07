//! What a member says about itself: its bytes, its key, and where it sits.
//!
//! Split from [`super`], which is about *finding* members — the byte walk, the
//! budgets, the node table. This half is about naming one that has already been
//! found, and the two grew past one file's worth together.
//!
//! # Identity is derived, never stored
//!
//! A node holds a link to its parent and its own short path step, and nothing
//! else about where it is. The fold id (`/0/3/7`) and the readable path
//! (`.users[3].name`) are both spelled by walking that chain when they are
//! asked for.
//!
//! Storing either one whole is the obvious design, and it is quadratic: a
//! document nested N levels opens N nodes, and the Nth node's path is N steps
//! long, so the paths alone cost N^2. A 200KB file of nothing but `[`
//! allocated 8.5GB that way and was killed by the OOM reaper. Derived, every
//! node is the same size and the same file costs 20MB.
//!
//! [`Doc::fold_id`] is on the painting path — once per row — so it also avoids
//! allocating per level, which is the difference between a deep document being
//! slow and being unusable.
#![deny(unsafe_code)]

use crate::json::index::{Member, Shape};

use super::{Doc, NodeId};

impl Doc {
    // -- where a node sits ---------------------------------------------------

    /// The fold id of a node: the index path `/0/3/7`, empty for the root.
    ///
    /// Derived from the parent chain on demand rather than stored (see
    /// the node's parent link). Deliberately positional rather than `.users[3]`:
    /// duplicate object keys are kept (SPEC.md §JSON, "Values"), so a key path
    /// is not unique and a fold id has to be.
    ///
    /// The chain runs leaf-to-root but the id reads root-to-leaf, and this is
    /// called once per painted row on documents nested thousands deep — so it
    /// builds the answer backwards in one pass over one buffer and reverses it
    /// at the end, rather than collecting the indices first. Writing each level's
    /// digits reversed too means the single final reverse puts *both* orders
    /// right: `/1/23` is built as `32/1/`.
    pub fn fold_id(&self, id: NodeId) -> String {
        let depth = self.nodes[id as usize].depth as usize;
        let mut buf: Vec<u8> = Vec::with_capacity(depth * 3);
        let mut at = Some(id);
        while let Some(n) = at {
            let node = &self.nodes[n as usize];
            let Some((p, mut i)) = node.parent else { break };
            loop {
                buf.push(b'0' + (i % 10) as u8);
                i /= 10;
                if i == 0 {
                    break;
                }
            }
            buf.push(b'/');
            at = Some(p);
        }
        buf.reverse();
        // Every byte pushed is ASCII, so this cannot fail.
        String::from_utf8(buf).unwrap_or_default()
    }

    /// The readable path of a node: `.users[3]`. Empty for the root.
    pub fn dpath(&self, id: NodeId) -> String {
        let mut out = String::new();
        for seg in self.chain(id).iter().rev() {
            out.push_str(seg);
        }
        out
    }

    /// The path segments from the root down to `id`, innermost first.
    fn chain(&self, id: NodeId) -> Vec<&str> {
        let mut out = Vec::new();
        let mut at = Some(id);
        while let Some(n) = at {
            let node = &self.nodes[n as usize];
            out.push(node.seg.as_str());
            at = node.parent.map(|(p, _)| p);
        }
        out
    }

    // -- what a member is ----------------------------------------------------

    /// What a member is, from its first byte.
    ///
    /// A member with an empty range is a key whose value never arrived (see
    /// [`crate::json::index::Scan`]); it has no first byte and no shape, and
    /// answering from whatever punctuation follows the key would let it be
    /// mistaken for a container.
    pub fn shape_of(&mut self, m: Member) -> Shape {
        if m.start >= m.end {
            return Shape::Bad;
        }
        match self.reader.chunk(m.start, 1).first() {
            Some(&b) => Shape::of(b),
            None => Shape::Bad,
        }
    }

    /// The bytes of `start..end`, and whether they were clipped by
    /// [`super::PARSE_CAP`] or by a file that shrank.
    pub fn bytes(&mut self, start: u64, end: u64) -> (Vec<u8>, bool) {
        let span = self.reader.bytes(start, end);
        (span.data, span.truncated)
    }

    /// The text of `start..end`, lossily decoded. For keys and short values.
    pub fn text(&mut self, start: u64, end: u64) -> String {
        let (data, _) = self.bytes(start, end);
        String::from_utf8_lossy(&data).into_owned()
    }

    /// The key of a member, unquoted and unescaped, or `None` for an array
    /// element. Escapes are resolved through the parser, so a key written
    /// `"aA"` reads as `aA` — the same string the document means.
    pub fn key_text(&mut self, m: Member) -> Option<String> {
        let (s, e) = m.key?;
        let raw = self.text(s, e);
        Some(match crate::json::parse(raw.as_bytes()) {
            Ok(v) => v.as_str().unwrap_or(&raw).to_string(),
            Err(_) => raw,
        })
    }

    /// The path segment a member adds: `.name`, `["odd key"]` or `[3]`. The
    /// shared spelling — the record source names a path the same way
    /// ([`crate::source::jsonrow::path_step`]).
    pub fn segment(&mut self, m: Member, i: usize) -> String {
        let key = self.key_text(m);
        crate::source::jsonrow::path_step(key.as_deref(), i)
    }

    /// The readable path of member `i` of `parent`: `.users[3].name`.
    pub fn path_of(&mut self, parent: NodeId, i: usize) -> String {
        let Some(m) = self.nodes[parent as usize].member(i) else {
            return self.dpath(parent);
        };
        let seg = self.segment(m, i);
        format!("{}{seg}", self.dpath(parent))
    }
}
