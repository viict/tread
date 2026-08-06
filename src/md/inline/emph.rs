//! Emphasis / strong / strikethrough resolution.
//!
//! The scanner in the parent module emits a flat `Vec<Node>`: resolved inlines
//! interleaved with unresolved delimiter runs (`*`, `_`, `~`). This module runs
//! the CommonMark delimiter matching over that list — including the flanking
//! flags computed by the scanner and the "rule of three" — and produces the
//! final `Vec<Inline>`. Anything that fails to match degrades to literal text.
#![deny(unsafe_code)]

use crate::md::ast::Inline;

/// One unresolved delimiter run. `orig` is the run length as written, which the
/// rule of three needs even after `len` has been partly consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Delim {
    pub ch: char,
    pub len: usize,
    pub orig: usize,
    pub open: bool,
    pub close: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    Inl(Inline),
    Delim(Delim),
}

/// An opener that is still looking for its closer, plus everything scanned
/// since it was pushed.
struct Frame {
    d: Delim,
    out: Vec<Inline>,
}

/// Resolve a flat node list into inlines. Never panics.
pub(crate) fn resolve(nodes: Vec<Node>) -> Vec<Inline> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut out: Vec<Inline> = Vec::new();
    for node in nodes {
        match node {
            Node::Inl(i) => top(&mut frames, &mut out).push(i),
            Node::Delim(d) => handle(&mut frames, &mut out, d),
        }
    }
    while !frames.is_empty() {
        unwind_one(&mut frames, &mut out);
    }
    merge(out)
}

/// The list new content is appended to: the innermost open frame, else the root.
fn top<'a>(frames: &'a mut [Frame], out: &'a mut Vec<Inline>) -> &'a mut Vec<Inline> {
    match frames.last_mut() {
        Some(f) => &mut f.out,
        None => out,
    }
}

/// Pop the innermost frame, spilling its delimiter back out as literal text.
fn unwind_one(frames: &mut Vec<Frame>, out: &mut Vec<Inline>) {
    let f = match frames.pop() {
        Some(f) => f,
        None => return,
    };
    let lit = rep(f.d.ch, f.d.len);
    let mut content = f.out;
    let target = top(frames, out);
    push_text(target, &lit);
    target.append(&mut content);
}

fn handle(frames: &mut Vec<Frame>, out: &mut Vec<Inline>, mut d: Delim) {
    while d.close {
        let k = match find_opener(frames, &d) {
            Some(k) => k,
            None => break,
        };
        while frames.len() > k + 1 {
            unwind_one(frames, out);
        }
        let f = match frames.pop() {
            Some(f) => f,
            None => break,
        };
        let n = f.d.len.min(d.len);
        let node = wrap(f.d.ch, n, f.out);
        let leftover = rep(f.d.ch, f.d.len - n);
        let target = top(frames, out);
        push_text(target, &leftover);
        target.push(node);
        d.len -= n;
        if d.len == 0 {
            return;
        }
    }
    if d.open {
        frames.push(Frame { d, out: Vec::new() });
    } else {
        let lit = rep(d.ch, d.len);
        push_text(top(frames, out), &lit);
    }
}

fn find_opener(frames: &[Frame], d: &Delim) -> Option<usize> {
    frames
        .iter()
        .rposition(|f| f.d.ch == d.ch && f.d.open && rule_of_three(&f.d, d))
}

/// CommonMark: if either run can both open and close, the summed original
/// lengths must not be a multiple of three unless both are.
fn rule_of_three(o: &Delim, c: &Delim) -> bool {
    if o.ch == '~' {
        return true;
    }
    if !(o.close || c.open) {
        return true;
    }
    (o.orig + c.orig) % 3 != 0 || (o.orig % 3 == 0 && c.orig % 3 == 0)
}

/// `n` delimiters worth of nesting: pairs become strong, a leftover single
/// becomes emphasis, so `***x***` is `<em><strong>x</strong></em>`.
fn wrap(ch: char, n: usize, content: Vec<Inline>) -> Inline {
    if ch == '~' {
        return Inline::Strike(content);
    }
    let mut cur = content;
    let mut k = n;
    while k >= 2 {
        cur = vec![Inline::Strong(cur)];
        k -= 2;
    }
    if k == 1 {
        cur = vec![Inline::Emph(cur)];
    }
    match cur.pop() {
        Some(one) if cur.is_empty() => one,
        Some(one) => {
            cur.push(one);
            Inline::Emph(cur)
        }
        None => Inline::Text(String::new()),
    }
}

fn rep(ch: char, n: usize) -> String {
    let mut s = String::with_capacity(n);
    for _ in 0..n {
        s.push(ch);
    }
    s
}

pub(crate) fn push_text(out: &mut Vec<Inline>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Inline::Text(last)) = out.last_mut() {
        last.push_str(s);
        return;
    }
    out.push(Inline::Text(s.to_string()));
}

/// Fuse adjacent text runs and drop empty ones.
pub(crate) fn merge(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    for it in items {
        match it {
            Inline::Text(s) => push_text(&mut out, &s),
            other => out.push(other),
        }
    }
    out
}
