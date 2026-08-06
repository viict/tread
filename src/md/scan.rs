//! Line-level scanners shared by the block parser: indentation, leaf-block
//! recognizers, and link-reference / footnote definition parsing.
//! Split out of `block.rs` to keep both files under the size limit.
#![deny(unsafe_code)]

use super::ast::LinkRefs;
use super::list;

/// A source line paired with its 1-based line number. Container parsers hand
/// dedented copies down to the recursive call, so `num` survives nesting.
#[derive(Debug, Clone)]
pub(crate) struct Ln {
    pub text: String,
    pub num: usize,
}

impl Ln {
    pub(crate) fn new(text: String, num: usize) -> Ln {
        Ln { text, num }
    }
}

/// Indentation of a line in columns; tabs advance to the next multiple of 4.
pub(crate) fn indent_of(s: &str) -> usize {
    let mut col = 0usize;
    for c in s.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => break,
        }
    }
    col
}

pub(crate) fn is_blank(s: &str) -> bool {
    s.chars().all(|c| c == ' ' || c == '\t')
}

/// Remove up to `n` columns of leading whitespace, re-padding with spaces if
/// a tab straddles the boundary.
pub(crate) fn strip_indent(s: &str, n: usize) -> String {
    let mut col = 0usize;
    let mut idx = 0usize;
    for (i, c) in s.char_indices() {
        if col >= n {
            idx = i;
            break;
        }
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => {
                idx = i;
                break;
            }
        }
        idx = i + c.len_utf8();
    }
    let mut out = String::new();
    for _ in n..col {
        out.push(' ');
    }
    out.push_str(&s[idx..]);
    out
}

pub(crate) fn is_thematic_break(s: &str) -> bool {
    if indent_of(s) >= 4 {
        return false;
    }
    let t = s.trim();
    let c = match t.chars().next() {
        Some(c @ ('-' | '_' | '*')) => c,
        _ => return false,
    };
    let mut n = 0;
    for ch in t.chars() {
        if ch == c {
            n += 1;
        } else if ch != ' ' && ch != '\t' {
            return false;
        }
    }
    n >= 3
}

/// ATX heading -> (level, text with any closing hash run removed).
pub(crate) fn atx(s: &str) -> Option<(u8, String)> {
    if indent_of(s) >= 4 {
        return None;
    }
    let t = s.trim_start();
    let level = t.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let mut body = rest.trim();
    // A closing hash run is only a closing run when it is preceded by a space
    // (or is the whole body). `trim_end_matches` keeps us on char boundaries;
    // byte arithmetic here used to panic on multi-byte headings like `# テスト`.
    let stripped = body.trim_end_matches('#');
    if stripped.len() < body.len() {
        if stripped.is_empty() {
            body = "";
        } else if stripped.ends_with(' ') || stripped.ends_with('\t') {
            body = stripped.trim_end();
        }
    }
    Some((level as u8, body.to_string()))
}

pub(crate) struct Fence {
    pub ch: char,
    pub len: usize,
    pub indent: usize,
    pub info: String,
}

pub(crate) fn fence_at(s: &str) -> Option<Fence> {
    let indent = indent_of(s);
    if indent >= 4 {
        return None;
    }
    let t = s.trim_start();
    let ch = match t.chars().next() {
        Some(c @ ('`' | '~')) => c,
        _ => return None,
    };
    let len = t.chars().take_while(|&c| c == ch).count();
    if len < 3 {
        return None;
    }
    let info = t[len..].trim().to_string();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some(Fence {
        ch,
        len,
        indent,
        info,
    })
}

/// A closing fence for an open fence of `(ch, len)`.
pub(crate) fn closes_fence(s: &str, ch: char, len: usize) -> bool {
    match fence_at(s) {
        Some(f) => f.ch == ch && f.len >= len && f.info.is_empty(),
        None => false,
    }
}

/// Setext underline -> heading level.
pub(crate) fn setext(s: &str) -> Option<u8> {
    if indent_of(s) >= 4 {
        return None;
    }
    let t = s.trim();
    let c = t.chars().next()?;
    if (c == '=' || c == '-') && t.chars().all(|x| x == c) {
        Some(if c == '=' { 1 } else { 2 })
    } else {
        None
    }
}

pub(crate) fn quote_start(s: &str) -> bool {
    indent_of(s) < 4 && s.trim_start().starts_with('>')
}

/// Strip one `>` marker plus at most one following space from a quote line.
pub(crate) fn strip_quote(s: &str) -> String {
    let rest = &s.trim_start()[1..];
    if let Some(r) = rest.strip_prefix(' ') {
        r.to_string()
    } else if let Some(r) = rest.strip_prefix('\t') {
        format!("  {}", r)
    } else {
        rest.to_string()
    }
}

pub(crate) fn html_start(s: &str) -> bool {
    if indent_of(s) >= 4 {
        return false;
    }
    let mut c = s.trim_start().chars();
    if c.next() != Some('<') {
        return false;
    }
    match c.next() {
        Some('!') | Some('?') | Some('/') => true,
        Some(x) => x.is_ascii_alphabetic(),
        None => false,
    }
}

/// True when `s` cannot be a lazy continuation line of a paragraph.
pub(crate) fn interrupts_paragraph(s: &str) -> bool {
    is_blank(s)
        || is_thematic_break(s)
        || atx(s).is_some()
        || fence_at(s).is_some()
        || quote_start(s)
        || html_start(s)
        || list::interrupting_marker(s)
}

/// `[label]: destination "title"` on a single line. Footnote definitions
/// (`[^x]:`) are deliberately excluded.
pub(crate) fn link_ref_at(s: &str) -> Option<(String, String, Option<String>)> {
    if indent_of(s) >= 4 {
        return None;
    }
    let inner = s.trim_start().strip_prefix('[')?;
    let end = close_bracket(inner)?;
    let label = &inner[..end];
    if label.is_empty() || label.starts_with('^') {
        return None;
    }
    let rest = inner[end + 1..].strip_prefix(':')?.trim();
    if rest.is_empty() {
        return None;
    }
    let (dest, tail) = split_destination(rest)?;
    Some((normalize_label(label), dest, ref_title(tail)?))
}

fn close_bracket(s: &str) -> Option<usize> {
    let mut esc = false;
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ']' {
            return Some(i);
        }
    }
    None
}

fn split_destination(rest: &str) -> Option<(String, &str)> {
    if let Some(r) = rest.strip_prefix('<') {
        let e = r.find('>')?;
        Some((r[..e].to_string(), r[e + 1..].trim()))
    } else {
        match rest.find(char::is_whitespace) {
            Some(p) => Some((rest[..p].to_string(), rest[p..].trim())),
            None => Some((rest.to_string(), "")),
        }
    }
}

/// `Some(None)` = no title; `None` = trailing junk, so not a definition.
fn ref_title(tail: &str) -> Option<Option<String>> {
    match tail.chars().next() {
        None => Some(None),
        Some(q @ ('"' | '\'' | '(')) => {
            let close = if q == '(' { ')' } else { q };
            if tail.chars().count() >= 2 && tail.ends_with(close) {
                Some(Some(tail[1..tail.len() - close.len_utf8()].to_string()))
            } else {
                None
            }
        }
        Some(_) => None,
    }
}

/// Reference labels match case-insensitively with whitespace collapsed.
pub(crate) fn normalize_label(label: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in label.trim().chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Fence-aware pre-scan: the inline parser needs every definition up front,
/// and definitions inside code fences are not definitions.
pub(crate) fn collect_refs(lines: &[Ln]) -> LinkRefs {
    let mut refs = LinkRefs::new();
    let mut open: Option<(char, usize)> = None;
    for l in lines {
        match open {
            Some((ch, len)) => {
                if closes_fence(&l.text, ch, len) {
                    open = None;
                }
            }
            None => match fence_at(&l.text) {
                Some(f) => open = Some((f.ch, f.len)),
                None => {
                    if let Some((k, u, t)) = link_ref_at(&l.text) {
                        refs.entry(k).or_insert((u, t));
                    }
                }
            },
        }
    }
    refs
}

/// `[^label]: rest-of-line`.
pub(crate) fn footnote_def_at(s: &str) -> Option<(String, String)> {
    if indent_of(s) >= 4 {
        return None;
    }
    let inner = s.trim_start().strip_prefix("[^")?;
    let end = inner.find("]:")?;
    let label = inner[..end].trim();
    if label.is_empty() {
        return None;
    }
    Some((label.to_string(), inner[end + 2..].trim_start().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indent_and_strip_are_tab_aware() {
        assert_eq!(indent_of("    x"), 4);
        assert_eq!(indent_of("\tx"), 4);
        assert_eq!(indent_of("  \tx"), 4);
        assert_eq!(strip_indent("    code", 4), "code");
        assert_eq!(strip_indent("\tcode", 2), "  code");
        assert_eq!(strip_indent("no indent", 4), "no indent");
    }

    #[test]
    fn atx_variants() {
        assert_eq!(atx("## Two"), Some((2, "Two".into())));
        assert_eq!(atx("### Three ###"), Some((3, "Three".into())));
        assert_eq!(atx("# foo#"), Some((1, "foo#".into())));
        assert_eq!(atx("#"), Some((1, String::new())));
        assert_eq!(atx("#hash"), None);
        assert_eq!(atx("####### 7"), None);
        assert_eq!(atx("    # indented"), None);
    }

    #[test]
    fn fences_and_breaks() {
        let f = fence_at("  ```rust,no_run").unwrap();
        assert_eq!(
            (f.ch, f.len, f.indent, f.info.as_str()),
            ('`', 3, 2, "rust,no_run")
        );
        assert!(fence_at("``` a ` b").is_none());
        assert!(closes_fence("````", '`', 3));
        assert!(!closes_fence("```", '`', 4));
        assert!(!closes_fence("~~~", '`', 3));
        assert!(is_thematic_break("- - -") && is_thematic_break("***"));
        assert!(!is_thematic_break("-- "));
        assert_eq!(setext("==="), Some(1));
        assert_eq!(setext("---"), Some(2));
        assert_eq!(setext("- -"), None);
    }

    #[test]
    fn link_ref_shapes() {
        assert_eq!(
            link_ref_at("[Foo Bar]: /a/b \"T\""),
            Some(("foo bar".into(), "/a/b".into(), Some("T".into())))
        );
        assert_eq!(
            link_ref_at("  [x]: <a b.md>"),
            Some(("x".into(), "a b.md".into(), None))
        );
        assert_eq!(link_ref_at("[^1]: a footnote"), None);
        assert_eq!(link_ref_at("[x]: /a trailing junk"), None);
        assert_eq!(link_ref_at("[x] not a def"), None);
        assert_eq!(
            footnote_def_at("[^note]: body"),
            Some(("note".into(), "body".into()))
        );
    }

    #[test]
    fn quote_and_html_scanners() {
        assert!(quote_start("> a") && quote_start("  >a"));
        assert!(!quote_start("    > a"));
        assert_eq!(strip_quote("> a"), "a");
        assert_eq!(strip_quote(">"), "");
        assert_eq!(strip_quote(">> b"), "> b");
        assert!(html_start("<div>") && html_start("<!-- c -->") && html_start("</p>"));
        assert!(!html_start("< div") && !html_start("a<div>"));
    }
}
