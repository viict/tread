//! Bracket constructs: links, images, reference links, footnote references,
//! angle autolinks and inline HTML. Everything here returns `None` on
//! malformed input so the caller can fall back to literal text.
#![deny(unsafe_code)]

use super::{is_punct, Scan};
use crate::md::ast::{inline_text, Inline, LinkRefs};
use crate::md::scan::normalize_label;

/// `[...]` at `i`: footnote reference, inline link or reference link.
pub(crate) fn bracket(st: &mut Scan, chars: &[char], i: usize, refs: &LinkRefs) -> Option<usize> {
    if let Some((label, end)) = footnote(chars, i) {
        st.push(Inline::FootnoteRef(label));
        return Some(end);
    }
    let (text, url, title, end) = link_like(chars, i, refs)?;
    st.push(Inline::Link { text, url, title });
    Some(end)
}

/// `![alt](url)` / `![alt][ref]` at `i` (which points at the `!`).
pub(crate) fn image(st: &mut Scan, chars: &[char], i: usize, refs: &LinkRefs) -> Option<usize> {
    let (text, url, _title, end) = link_like(chars, i + 1, refs)?;
    st.push(Inline::Image {
        alt: inline_text(&text),
        url,
    });
    Some(end)
}

/// `[^label]` with a non-empty, whitespace-free label.
fn footnote(chars: &[char], i: usize) -> Option<(String, usize)> {
    if chars.get(i + 1) != Some(&'^') {
        return None;
    }
    let mut j = i + 2;
    let mut label = String::new();
    while let Some(&c) = chars.get(j) {
        if c == ']' {
            break;
        }
        if c.is_whitespace() || c == '[' {
            return None;
        }
        label.push(c);
        j += 1;
    }
    if label.is_empty() || chars.get(j) != Some(&']') {
        return None;
    }
    Some((label, j + 1))
}

/// Shared body of links and images: text inlines, destination, title, end.
fn link_like(
    chars: &[char],
    i: usize,
    refs: &LinkRefs,
) -> Option<(Vec<Inline>, String, Option<String>, usize)> {
    let close = match_bracket(chars, i)?;
    let raw: String = chars[i + 1..close].iter().collect();
    if chars.get(close + 1) == Some(&'(') {
        let (url, title, end) = inline_dest(chars, close + 1)?;
        return Some((super::parse_nested(&raw, refs), url, title, end));
    }
    let (label, end) = ref_label(chars, close, &raw)?;
    let (url, title) = refs.get(&normalize_label(&label))?.clone();
    Some((super::parse_nested(&raw, refs), url, title, end))
}

/// Index of the `]` closing the `[` at `i`, honoring nesting, backslash
/// escapes and code spans (`[a `]` b]` must not close early).
fn match_bracket(chars: &[char], i: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut j = i;
    while let Some(&c) = chars.get(j) {
        match c {
            '\\' => j += 1,
            '`' => j = skip_code(chars, j) - 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Index just past a backtick run and its matching closer, or just past the
/// run when it never closes.
fn skip_code(chars: &[char], i: usize) -> usize {
    let n = run_len(chars, i, '`');
    let mut j = i + n;
    while j < chars.len() {
        if chars[j] == '`' {
            let m = run_len(chars, j, '`');
            if m == n {
                return j + m;
            }
            j += m;
        } else {
            j += 1;
        }
    }
    i + n
}

pub(crate) fn run_len(chars: &[char], i: usize, ch: char) -> usize {
    let mut n = 0;
    while chars.get(i + n) == Some(&ch) {
        n += 1;
    }
    n
}

/// `(dest "title")` starting at the `(`.
fn inline_dest(chars: &[char], at: usize) -> Option<(String, Option<String>, usize)> {
    let mut j = skip_ws(chars, at + 1);
    let url = if chars.get(j) == Some(&'<') {
        let mut u = String::new();
        j += 1;
        while let Some(&c) = chars.get(j) {
            if c == '>' {
                break;
            }
            if c == '\n' {
                return None;
            }
            u.push(c);
            j += 1;
        }
        chars.get(j)?;
        j += 1;
        u
    } else {
        let (u, n) = bare_dest(chars, j)?;
        j = n;
        u
    };
    let (title, mut j) = dest_title(chars, skip_ws(chars, j));
    j = skip_ws(chars, j);
    if chars.get(j) != Some(&')') {
        return None;
    }
    Some((url, title, j + 1))
}

/// Unbracketed destination: stops at whitespace or the unbalanced `)`.
fn bare_dest(chars: &[char], mut j: usize) -> Option<(String, usize)> {
    let mut depth = 0usize;
    let mut u = String::new();
    while let Some(&c) = chars.get(j) {
        match c {
            '\\' => {
                if let Some(&n) = chars.get(j + 1) {
                    if n.is_ascii_punctuation() {
                        u.push(n);
                        j += 2;
                        continue;
                    }
                }
                u.push('\\');
            }
            '(' => {
                depth += 1;
                u.push(c);
            }
            ')' if depth == 0 => break,
            ')' => {
                depth -= 1;
                u.push(c);
            }
            c if c.is_whitespace() => break,
            _ => u.push(c),
        }
        j += 1;
    }
    Some((u, j))
}

fn dest_title(chars: &[char], j: usize) -> (Option<String>, usize) {
    let q = match chars.get(j) {
        Some(&c @ ('"' | '\'')) => c,
        _ => return (None, j),
    };
    let mut k = j + 1;
    let mut t = String::new();
    while let Some(&c) = chars.get(k) {
        if c == q {
            return (Some(t), k + 1);
        }
        if c == '\\' {
            if let Some(&n) = chars.get(k + 1) {
                if n.is_ascii_punctuation() {
                    t.push(n);
                    k += 2;
                    continue;
                }
            }
        }
        t.push(c);
        k += 1;
    }
    (None, j)
}

fn skip_ws(chars: &[char], mut j: usize) -> usize {
    while chars.get(j).is_some_and(|c| c.is_whitespace()) {
        j += 1;
    }
    j
}

/// Full (`[t][lbl]`), collapsed (`[t][]`) or shortcut (`[t]`) reference label.
fn ref_label(chars: &[char], close: usize, raw: &str) -> Option<(String, usize)> {
    if chars.get(close + 1) != Some(&'[') {
        return Some((raw.to_string(), close + 1));
    }
    let mut j = close + 2;
    let mut label = String::new();
    while let Some(&c) = chars.get(j) {
        if c == ']' {
            let l = if label.trim().is_empty() {
                raw.to_string()
            } else {
                label
            };
            return Some((l, j + 1));
        }
        if c == '[' {
            return None;
        }
        label.push(c);
        j += 1;
    }
    None
}

/// `<...>`: URI autolink, email autolink, or inline HTML.
pub(crate) fn angle(st: &mut Scan, chars: &[char], i: usize) -> Option<usize> {
    if let Some((body, end)) = plain_angle(chars, i) {
        if is_uri(&body) {
            st.push(Inline::Autolink(body));
            return Some(end);
        }
        if is_email(&body) {
            let url = format!("mailto:{}", body);
            st.push(Inline::Link {
                text: vec![Inline::Text(body)],
                url,
                title: None,
            });
            return Some(end);
        }
    }
    let (raw, end) = html_tag(chars, i)?;
    st.push(Inline::Html(raw));
    Some(end)
}

/// `<body>` where body has no whitespace, `<` or `>`.
fn plain_angle(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i + 1;
    let mut body = String::new();
    while let Some(&c) = chars.get(j) {
        match c {
            '>' => {
                return if body.is_empty() {
                    None
                } else {
                    Some((body, j + 1))
                }
            }
            '<' => return None,
            c if c.is_whitespace() => return None,
            _ => body.push(c),
        }
        j += 1;
    }
    None
}

fn is_uri(s: &str) -> bool {
    let p = match s.find(':') {
        Some(p) => p,
        None => return false,
    };
    let scheme = &s[..p];
    if scheme.len() < 2 || scheme.len() > 32 || p + 1 >= s.len() {
        return false;
    }
    let mut cs = scheme.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic())
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

fn is_email(s: &str) -> bool {
    let mut parts = s.split('@');
    let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
        (Some(l), Some(d), None) => (l, d),
        _ => return false,
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c))
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// `<tag ...>`, `</tag>`, `<!-- comment -->`, `<?pi?>`, `<!DOCTYPE ...>`.
fn html_tag(chars: &[char], i: usize) -> Option<(String, usize)> {
    let first = *chars.get(i + 1)?;
    if first != '!' && first != '?' && first != '/' && !first.is_ascii_alphabetic() {
        return None;
    }
    if chars.get(i + 1..i + 4) == Some(&['!', '-', '-'][..]) {
        return comment(chars, i);
    }
    if first == '/' && !chars.get(i + 2).copied().is_some_and(is_name_start) {
        return None;
    }
    let mut j = i + 1;
    let mut quote: Option<char> = None;
    while let Some(&c) = chars.get(j) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '<') => return None,
            (None, '>') => return Some((chars[i..=j].iter().collect(), j + 1)),
            _ => {}
        }
        j += 1;
    }
    None
}

fn comment(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i + 4;
    while j + 3 <= chars.len() {
        if chars.get(j..j + 3) == Some(&['-', '-', '>'][..]) {
            return Some((chars[i..j + 3].iter().collect(), j + 3));
        }
        j += 1;
    }
    None
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// Bare `http://` / `https://` run starting at `i`, with trailing sentence
/// punctuation and unbalanced closers trimmed off.
pub(crate) fn bare_url(st: &mut Scan, chars: &[char], i: usize) -> Option<usize> {
    let head: String = chars.get(i..i + 8).map(|s| s.iter().collect())?;
    let scheme = if head.starts_with("https://") {
        8
    } else if head.starts_with("http://") {
        7
    } else {
        return None;
    };
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '/' || chars[i - 1] == ':') {
        return None;
    }
    let mut j = i + scheme;
    while chars
        .get(j)
        .is_some_and(|&c| !c.is_whitespace() && c != '<' && c != '>' && c != '`')
    {
        j += 1;
    }
    j = trim_url_tail(chars, i, j);
    if j <= i + scheme {
        return None;
    }
    st.push(Inline::Autolink(chars[i..j].iter().collect()));
    Some(j)
}

fn trim_url_tail(chars: &[char], start: usize, mut end: usize) -> usize {
    while end > start {
        let c = chars[end - 1];
        let drop = match c {
            ')' => count(chars, start, end, '(') < count(chars, start, end, ')'),
            ']' => count(chars, start, end, '[') < count(chars, start, end, ']'),
            '*' | '_' | '~' | '"' | '\'' => true,
            _ => is_punct(c) && c != '/' && c != '#' && c != '=' && c != '%',
        };
        if !drop {
            break;
        }
        end -= 1;
    }
    end
}

fn count(chars: &[char], a: usize, b: usize, ch: char) -> usize {
    chars[a..b].iter().filter(|&&c| c == ch).count()
}
