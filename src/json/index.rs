//! The lazy structural index (SPEC.md §JSON, "Structural indexing").
//!
//! A container is indexed by *byte range*, never parsed. Finding the boundaries
//! of a container's immediate members is a linear byte walk with a depth
//! counter, an in-string flag and an escape flag: it builds no values, and it
//! allocates nothing per member beyond three offsets. That is what lets a 900MB
//! JSON open as fast as a 900MB CSV — the same discipline as
//! [`crate::csv::index`], one level down.
//!
//! # Resumable
//!
//! [`Scan`] is fed one chunk at a time and remembers where it stopped, exactly
//! as [`crate::csv::parse::Scanner`] does, including mid-string and mid-escape.
//! A caller therefore spends a bounded number of bytes per frame and comes back
//! for the rest, which is what keeps `q` from ever waiting on a scan. Nothing
//! here reads a file: chunks come from above, so this module is pure and
//! host-tested against byte slices.
//!
//! # Every level, not just the root
//!
//! One [`Scan`] indexes *one* container. Expanding a node runs another over
//! that node's bytes, so laziness is not limited to the top level: a document
//! that is one object holding one enormous array stays instant, because the
//! array is only walked when it is opened.
//!
//! # Nothing recurses
//!
//! The walk is a `for` loop over bytes with an integer depth. Ten thousand
//! levels of `[[[[` cost ten thousand increments of a `u32` and no stack at
//! all.
#![deny(unsafe_code)]

/// One immediate member of a container, as byte ranges into the document.
///
/// `key` is the *quoted* key of an object member, including both quotes;
/// `None` for an array element. `start..end` is the member's value, with
/// surrounding whitespace already trimmed off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub key: Option<(u64, u64)>,
    pub start: u64,
    pub end: u64,
}

impl Member {
    /// Bytes the value occupies. The cap on parsing a member is applied to
    /// this, never to the document.
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

/// What a value is, from its first byte alone — no parsing, no allocation.
///
/// This is how a row knows whether it is a container (and so has a count and a
/// fold) before anything reads it. A byte that starts nothing valid is
/// [`Shape::Bad`], which renders as an error row rather than stopping the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Array,
    Object,
    Str,
    Number,
    Bool,
    Null,
    Bad,
}

impl Shape {
    pub fn of(first: u8) -> Shape {
        match first {
            b'[' => Shape::Array,
            b'{' => Shape::Object,
            b'"' => Shape::Str,
            b'-' | b'0'..=b'9' => Shape::Number,
            b't' | b'f' => Shape::Bool,
            b'n' => Shape::Null,
            _ => Shape::Bad,
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, Shape::Array | Shape::Object)
    }

    /// The pair of brackets a container is written with.
    pub fn brackets(self) -> (&'static str, &'static str) {
        match self {
            Shape::Object => ("{", "}"),
            _ => ("[", "]"),
        }
    }

    /// What a collapsed container counts, singular and plural.
    pub fn unit(self, n: usize) -> &'static str {
        match (self, n) {
            (Shape::Object, 1) => "key",
            (Shape::Object, _) => "keys",
            (_, 1) => "item",
            (_, _) => "items",
        }
    }
}

/// The document's root: where the first value starts, and what it is.
///
/// Reads only the head of the file — a BOM and whatever whitespace precedes the
/// first value. An empty document has no root.
pub fn root(head: &[u8], base: u64) -> Option<(u64, Shape)> {
    let bom: &[u8] = &[0xef, 0xbb, 0xbf];
    let skip = usize::from(head.starts_with(bom)) * bom.len();
    let at = head[skip..]
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))?;
    let i = skip + at;
    Some((base + i as u64, Shape::of(head[i])))
}

/// A resumable structural walk over one container's immediate members.
///
/// Created just after the opening `[` or `{`; fed chunks in file order; emits a
/// [`Member`] per immediate member and stops on the matching close, recording
/// where the container ends.
#[derive(Clone, Debug)]
pub struct Scan {
    /// Absolute offset of the next byte to consume.
    pos: u64,
    /// The container is an object, so a member starts with a key.
    obj: bool,
    /// Nesting inside the current member. Members are found at depth 0.
    depth: u32,
    in_str: bool,
    esc: bool,
    /// Where the key string began, while it is being read.
    key_open: Option<u64>,
    key: Option<(u64, u64)>,
    colon: bool,
    val: Option<u64>,
    /// One past the last non-whitespace byte of the value so far.
    val_end: u64,
    /// One past the container's closing bracket, once it has been seen.
    end: Option<u64>,
    /// The container ran off the end of the document.
    truncated: bool,
    count: usize,
}

impl Scan {
    /// A scan of the container whose opening bracket is at `open_at`.
    pub fn new(open_at: u64, obj: bool) -> Scan {
        Scan {
            pos: open_at + 1,
            obj,
            depth: 0,
            in_str: false,
            esc: false,
            key_open: None,
            key: None,
            colon: false,
            val: None,
            val_end: 0,
            end: None,
            truncated: false,
            count: 0,
        }
    }

    /// Absolute offset the next chunk must start at.
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// The container's close has been found (or the file ended).
    pub fn done(&self) -> bool {
        self.end.is_some()
    }

    /// One past the container's last byte, once known.
    pub fn end(&self) -> Option<u64> {
        self.end
    }

    /// Members emitted so far. The count a collapsed row shows is kept by the
    /// caller, which knows which container it belongs to; this is the tests'
    /// spelling of the same fact.
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.count
    }

    /// The container was cut off by the end of the document. Nothing above
    /// changes its behaviour for that — a half-written container still shows
    /// the members it has — so only the tests ask.
    #[cfg(test)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Consume `chunk`, which must start at [`Scan::pos`], reporting every
    /// member that ends inside it. Stops at the container's close, leaving the
    /// rest of the chunk — it belongs to the parent, not to us.
    pub fn feed(&mut self, chunk: &[u8], sink: &mut dyn FnMut(Member)) {
        let base = self.pos;
        for (k, &b) in chunk.iter().enumerate() {
            let p = base + k as u64;
            if self.byte(p, b, sink) {
                self.pos = p + 1;
                return;
            }
        }
        self.pos = base + chunk.len() as u64;
    }

    /// One byte. Returns true when it closed the container.
    fn byte(&mut self, p: u64, b: u8, sink: &mut dyn FnMut(Member)) -> bool {
        if self.in_str {
            self.in_string(p, b);
            return false;
        }
        match b {
            b'"' => self.quote(p),
            b'[' | b'{' => {
                self.start_value(p);
                self.depth += 1;
                self.val_end = p + 1;
            }
            b']' | b'}' if self.depth == 0 => {
                self.flush(sink);
                self.end = Some(p + 1);
                return true;
            }
            b']' | b'}' => {
                self.depth -= 1;
                self.val_end = p + 1;
            }
            b',' if self.depth == 0 => self.flush(sink),
            b':' if self.depth == 0 && self.obj && !self.colon && self.val.is_none() => {
                self.colon = true;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => {
                self.start_value(p);
                self.val_end = p + 1;
            }
        }
        false
    }

    /// A byte inside a string: only the escape rules matter, since a `]` or a
    /// `,` in there is data and not structure.
    fn in_string(&mut self, p: u64, b: u8) {
        if self.esc {
            self.esc = false;
        } else if b == b'\\' {
            self.esc = true;
        } else if b == b'"' {
            self.in_str = false;
            if let Some(s) = self.key_open.take() {
                self.key = Some((s, p + 1));
            }
        }
        if self.val.is_some() {
            self.val_end = p + 1;
        }
    }

    /// An opening quote outside a string: the member's key, or its value.
    fn quote(&mut self, p: u64) {
        self.in_str = true;
        if self.obj && self.val.is_none() && !self.colon && self.key.is_none() {
            self.key_open = Some(p);
            return;
        }
        self.start_value(p);
        self.val_end = p + 1;
    }

    fn start_value(&mut self, p: u64) {
        if self.val.is_none() {
            self.val = Some(p);
        }
    }

    /// Emit the member just ended, if there was one, and reset for the next.
    ///
    /// A trailing comma or an empty container flushes nothing. A key with no
    /// value — malformed input, a truncated file — still emits, keyed, with an
    /// **empty** value range starting where the value would have begun. The row
    /// then renders as a parse error naming that offset, which is more use than
    /// the member silently disappearing. It must not borrow the key's own bytes
    /// for the value: those are valid JSON, so `{"beta":` would display as
    /// `"beta": "beta"` — text the document does not contain.
    fn flush(&mut self, sink: &mut dyn FnMut(Member)) {
        let key = self.key.take();
        let val = self.val.take();
        let (start, end) = match (val, key) {
            (Some(s), _) => (s, self.val_end.max(s)),
            (None, Some((_, e))) => (e, e),
            (None, None) => {
                self.reset();
                return;
            }
        };
        sink(Member { key, start, end });
        self.count += 1;
        self.reset();
    }

    fn reset(&mut self) {
        self.key_open = None;
        self.key = None;
        self.colon = false;
        self.val = None;
        self.val_end = 0;
        self.in_str = false;
        self.esc = false;
        self.depth = 0;
    }

    /// The document ended before the container closed. Whatever member was
    /// being read is emitted — half a document is still worth reading — and the
    /// container is marked as ending here.
    pub fn finish(&mut self, at: u64, sink: &mut dyn FnMut(Member)) {
        if self.end.is_some() {
            return;
        }
        self.val_end = self.val_end.min(at);
        self.flush(sink);
        self.end = Some(at);
        self.truncated = true;
    }
}

/// Every member of one container, in document order, compactly.
///
/// Four offsets a member — key start, key end, value start, value end — held as
/// `u32` deltas from the container's own start, so a member costs 16 bytes and
/// the index of a multi-million element array stays in tens of megabytes rather
/// than hundreds. A container whose extent passes 4GiB promotes to `u64` and
/// stays exact. An array element has no key, which is recorded as an empty key
/// range rather than as a fifth field.
#[derive(Debug)]
pub struct Members {
    base: u64,
    store: Store,
}

#[derive(Debug)]
enum Store {
    Narrow(Vec<[u32; 4]>),
    Wide(Vec<[u64; 4]>),
}

impl Members {
    pub fn new(base: u64) -> Members {
        Members { base, store: Store::Narrow(Vec::new()) }
    }

    pub fn len(&self) -> usize {
        match &self.store {
            Store::Narrow(v) => v.len(),
            Store::Wide(v) => v.len(),
        }
    }

    /// True when the container has no members yet. Kept beside
    /// [`Members::len`] because a `len` without an `is_empty` is a lint, and
    /// used by the tests.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record one member. Offsets only grow, so a delta never underflows.
    pub fn push(&mut self, m: Member) {
        let (ks, ke) = m.key.unwrap_or((m.start, m.start));
        let raw = [ks, ke, m.start, m.end].map(|o| o.saturating_sub(self.base));
        if raw.iter().any(|d| *d >= u32::MAX as u64) {
            self.promote();
        }
        match &mut self.store {
            Store::Narrow(v) => v.push(raw.map(|d| d as u32)),
            Store::Wide(v) => v.push(raw),
        }
    }

    fn promote(&mut self) {
        if let Store::Narrow(v) = &self.store {
            self.store = Store::Wide(v.iter().map(|t| t.map(u64::from)).collect());
        }
    }

    /// Member `i`, or `None` past the end.
    pub fn get(&self, i: usize) -> Option<Member> {
        let raw = match &self.store {
            Store::Narrow(v) => v.get(i)?.map(u64::from),
            Store::Wide(v) => *v.get(i)?,
        };
        let [ks, ke, start, end] = raw.map(|d| self.base + d);
        Some(Member { key: (ks != ke).then_some((ks, ke)), start, end })
    }

    /// Approximate heap cost, for the tests that pin the per-member size.
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        match &self.store {
            Store::Narrow(v) => v.capacity() * 16,
            Store::Wide(v) => v.capacity() * 32,
        }
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
