//! Inline (span-level) markdown parsing: source text -> `Vec<Inline>`.
//!
//! Two passes. `scan` walks the character slice left to right and resolves
//! everything whose extent is locally decidable — code spans, escapes, links,
//! images, autolinks, HTML, footnote refs, line breaks — emitting the `*`/`_`/
//! `~` delimiter runs it cannot resolve as `Node::Delim`. `emph::resolve` then
//! matches those runs into emphasis, strong and strikethrough.
//!
//! The whole module works over `&[char]`, so no index can ever land inside a
//! multi-byte character. Malformed input never panics: it degrades to text.
#![deny(unsafe_code)]

mod emph;
mod link;

use self::emph::{Delim, Node};
use super::ast::{Inline, LinkRefs};

/// Parse inline markdown. `link_refs` holds normalized link reference
/// definitions collected by the block parser; unresolved references stay
/// literal text.
pub fn parse_inlines(src: &str, link_refs: &LinkRefs) -> Vec<Inline> {
    let chars: Vec<char> = src.chars().collect();
    emph::resolve(scan(&chars, link_refs, false))
}

/// Parse the text of a link or image: same grammar, minus the constructs that
/// cannot nest inside a link (further links and bare URLs).
fn parse_nested(src: &str, link_refs: &LinkRefs) -> Vec<Inline> {
    let chars: Vec<char> = src.chars().collect();
    emph::resolve(scan(&chars, link_refs, true))
}

/// Scanner state: finished nodes plus the pending literal-text run.
pub(crate) struct Scan {
    nodes: Vec<Node>,
    buf: String,
}

impl Scan {
    fn flush(&mut self) {
        if !self.buf.is_empty() {
            let s = std::mem::take(&mut self.buf);
            self.nodes.push(Node::Inl(Inline::Text(s)));
        }
    }

    /// Finish the pending text run and append a resolved inline.
    pub(crate) fn push(&mut self, inl: Inline) {
        self.flush();
        self.nodes.push(Node::Inl(inl));
    }
}

fn scan(chars: &[char], refs: &LinkRefs, in_link: bool) -> Vec<Node> {
    let mut st = Scan {
        nodes: Vec::new(),
        buf: String::new(),
    };
    let mut i = 0;
    while i < chars.len() {
        i = match step(&mut st, chars, i, refs, in_link) {
            Some(next) if next > i => next,
            _ => {
                st.buf.push(chars[i]);
                i + 1
            }
        };
    }
    st.flush();
    st.nodes
}

/// Try every construct that can start at `i`. `None` means "plain character".
fn step(st: &mut Scan, chars: &[char], i: usize, refs: &LinkRefs, in_link: bool) -> Option<usize> {
    match chars[i] {
        '\\' => Some(escape(st, chars, i)),
        '`' => Some(code_span(st, chars, i)),
        '\n' => Some(line_break(st, chars, i)),
        '<' => link::angle(st, chars, i),
        '!' if !in_link && chars.get(i + 1) == Some(&'[') => link::image(st, chars, i, refs),
        '[' if !in_link => link::bracket(st, chars, i, refs),
        '*' | '_' | '~' => Some(delim_run(st, chars, i)),
        'h' if !in_link => link::bare_url(st, chars, i),
        _ => None,
    }
}

/// `\x`: any ASCII punctuation becomes literal; `\` before a newline is a hard
/// break; anything else is a literal backslash.
fn escape(st: &mut Scan, chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        Some('\n') => {
            st.push(Inline::HardBreak);
            skip_indent(chars, i + 2)
        }
        Some(&c) if c.is_ascii_punctuation() => {
            st.buf.push(c);
            i + 2
        }
        _ => {
            st.buf.push('\\');
            i + 1
        }
    }
}

/// A backtick run closed by a run of exactly the same length. One leading and
/// one trailing space are stripped when both are present (CommonMark).
fn code_span(st: &mut Scan, chars: &[char], i: usize) -> usize {
    let n = link::run_len(chars, i, '`');
    let mut j = i + n;
    while j < chars.len() {
        if chars[j] != '`' {
            j += 1;
            continue;
        }
        let m = link::run_len(chars, j, '`');
        if m == n {
            st.push(Inline::Code(code_text(&chars[i + n..j])));
            return j + m;
        }
        j += m;
    }
    for _ in 0..n {
        st.buf.push('`');
    }
    i + n
}

fn code_text(body: &[char]) -> String {
    let mut s: String = body
        .iter()
        .map(|&c| if c == '\n' { ' ' } else { c })
        .collect();
    let trimmable = s.starts_with(' ') && s.ends_with(' ') && s.chars().any(|c| c != ' ');
    if trimmable {
        s.pop();
        s.remove(0);
    }
    s
}

/// Two trailing spaces (or a trailing backslash, handled in `escape`) make a
/// hard break; otherwise the newline is a soft break. Surrounding whitespace
/// is dropped either way.
fn line_break(st: &mut Scan, chars: &[char], i: usize) -> usize {
    let keep = st.buf.trim_end_matches([' ', '\t']).len();
    let hard = st.buf.len() - keep >= 2;
    st.buf.truncate(keep);
    st.push(if hard {
        Inline::HardBreak
    } else {
        Inline::SoftBreak
    });
    skip_indent(chars, i + 1)
}

fn skip_indent(chars: &[char], mut j: usize) -> usize {
    while chars.get(j).is_some_and(|&c| c == ' ' || c == '\t') {
        j += 1;
    }
    j
}

/// A run of `*`, `_` or `~`. Runs that can neither open nor close (notably the
/// `_` inside `snake_case`) collapse straight into literal text.
fn delim_run(st: &mut Scan, chars: &[char], i: usize) -> usize {
    let ch = chars[i];
    let n = link::run_len(chars, i, ch);
    let prev = if i > 0 { Some(chars[i - 1]) } else { None };
    let next = chars.get(i + n).copied();
    let (mut open, mut close) = flanking(ch, prev, next);
    if ch == '~' && n > 2 {
        open = false;
        close = false;
    }
    if !open && !close {
        for _ in 0..n {
            st.buf.push(ch);
        }
        return i + n;
    }
    st.flush();
    st.nodes.push(Node::Delim(Delim {
        ch,
        len: n,
        orig: n,
        open,
        close,
    }));
    i + n
}

/// CommonMark left/right flanking, with the extra intraword restriction that
/// makes `_` inert inside identifiers.
fn flanking(ch: char, prev: Option<char>, next: Option<char>) -> (bool, bool) {
    let (pw, nw) = (
        prev.map_or(true, char::is_whitespace),
        next.map_or(true, char::is_whitespace),
    );
    let (pp, np) = (prev.is_some_and(is_punct), next.is_some_and(is_punct));
    let left = !nw && (!np || pw || pp);
    let right = !pw && (!pp || nw || np);
    if ch == '_' {
        (left && (!right || pp), right && (!left || np))
    } else {
        (left, right)
    }
}

/// ASCII punctuation plus non-ASCII symbols/punctuation (em dashes, quotes).
pub(crate) fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

#[cfg(test)]
mod tests;
