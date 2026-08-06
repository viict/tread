//! Bullet / ordered list parsing: nesting by indentation, lazy continuation,
//! tight vs loose detection, task checkboxes, and arbitrary blocks inside
//! items. Pure Rust, no unsafe.
#![deny(unsafe_code)]

use super::ast::{Block, LinkRefs, ListItem, ListKind};
use super::block::parse_blocks;
use super::scan::{indent_of, interrupts_paragraph, is_blank, is_thematic_break, strip_indent, Ln};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MK {
    Bullet(char),
    Ordered(u64, char),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Marker {
    /// Indent columns of the marker itself.
    pub indent: usize,
    /// Column at which the item's content starts.
    pub content: usize,
    /// Byte offset in the source line at which the item's content starts.
    pub offset: usize,
    pub kind: MK,
}

impl MK {
    fn same_family(self, other: MK) -> bool {
        match (self, other) {
            (MK::Bullet(a), MK::Bullet(b)) => a == b,
            (MK::Ordered(_, a), MK::Ordered(_, b)) => a == b,
            _ => false,
        }
    }
}

/// Recognize a list marker at the start of `s`.
pub(crate) fn marker(s: &str) -> Option<Marker> {
    let indent = indent_of(s);
    if indent >= 4 || is_thematic_break(s) {
        return None;
    }
    let lead = s.len() - s.trim_start().len();
    let t = &s[lead..];
    let first = t.chars().next()?;
    let (kind, mlen) = if first == '-' || first == '+' || first == '*' {
        (MK::Bullet(first), 1)
    } else if first.is_ascii_digit() {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 9 {
            return None;
        }
        let delim = t[digits..].chars().next()?;
        if delim != '.' && delim != ')' {
            return None;
        }
        let n: u64 = t[..digits].parse().ok()?;
        (MK::Ordered(n, delim), digits + 1)
    } else {
        return None;
    };
    let after = &t[mlen..];
    let spaces = after.chars().take_while(|&c| c == ' ' || c == '\t').count();
    if after.is_empty() {
        return Some(Marker {
            indent,
            content: indent + mlen + 1,
            offset: lead + mlen,
            kind,
        });
    }
    if spaces == 0 {
        return None;
    }
    let rest_blank = after[spaces..].is_empty();
    let used = if spaces > 4 || rest_blank { 1 } else { spaces };
    Some(Marker {
        indent,
        content: indent + mlen + used,
        offset: lead + mlen + used.min(spaces),
        kind,
    })
}

/// A marker that is allowed to interrupt a paragraph (GFM: non-empty item,
/// and ordered lists only when they start at 1).
pub(crate) fn interrupting_marker(s: &str) -> bool {
    match marker(s) {
        Some(m) => {
            if s[m.offset..].trim().is_empty() {
                return false;
            }
            match m.kind {
                MK::Bullet(_) => true,
                MK::Ordered(n, _) => n == 1,
            }
        }
        None => false,
    }
}

/// Parse a whole list starting at `lines[i]`; returns the index after it.
pub(crate) fn parse_list(lines: &[Ln], i: usize, refs: &LinkRefs, out: &mut Vec<Block>) -> usize {
    let first = match marker(&lines[i].text) {
        Some(m) => m,
        None => return i + 1,
    };
    let kind = match first.kind {
        MK::Bullet(_) => ListKind::Bullet,
        MK::Ordered(start, _) => ListKind::Ordered { start },
    };
    let mut items: Vec<ListItem> = Vec::new();
    let mut loose = false;
    let mut j = i;
    let mut gap = false;
    while j < lines.len() {
        if is_blank(&lines[j].text) {
            gap = true;
            j += 1;
            continue;
        }
        let m = match marker(&lines[j].text) {
            Some(m) if m.kind.same_family(first.kind) && m.indent < first.content => m,
            _ => break,
        };
        if gap && !items.is_empty() {
            loose = true;
        }
        gap = false;
        let (inner, next, internal_blank) = collect_item(lines, j, m);
        loose |= internal_blank;
        items.push(make_item(inner, refs));
        j = next;
    }
    let end = if gap { rewind_blanks(lines, j) } else { j };
    out.push(Block::List {
        kind,
        tight: !loose,
        items,
        source_line: lines[i].num,
    });
    end
}

fn rewind_blanks(lines: &[Ln], j: usize) -> usize {
    let mut k = j;
    while k > 0 && is_blank(&lines[k - 1].text) {
        k -= 1;
    }
    k
}

fn make_item(inner: Vec<Ln>, refs: &LinkRefs) -> ListItem {
    let (task, inner) = split_task(inner);
    ListItem {
        task,
        blocks: parse_blocks(&inner, refs),
    }
}

/// GFM task-list checkbox at the very start of an item.
fn split_task(mut inner: Vec<Ln>) -> (Option<bool>, Vec<Ln>) {
    let checked = match inner.first() {
        Some(l) => {
            let t = l.text.trim_start();
            let mut c = t.chars();
            match (c.next(), c.next(), c.next(), c.next()) {
                (Some('['), Some(mark), Some(']'), rest)
                    if matches!(mark, ' ' | 'x' | 'X')
                        && matches!(rest, None | Some(' ') | Some('\t')) =>
                {
                    Some(mark != ' ')
                }
                _ => None,
            }
        }
        None => None,
    };
    if checked.is_some() {
        let l = &mut inner[0];
        let t = l.text.trim_start();
        l.text = t[3..].strip_prefix(' ').unwrap_or(&t[3..]).to_string();
    }
    (checked, inner)
}

/// Collect one item's lines, dedented to its content column.
/// Returns (lines, next index, whether a blank line occurred inside).
fn collect_item(lines: &[Ln], j: usize, m: Marker) -> (Vec<Ln>, usize, bool) {
    let head = &lines[j];
    let mut inner = vec![Ln::new(head.text[m.offset..].to_string(), head.num)];
    let mut k = j + 1;
    let mut internal_blank = false;
    while k < lines.len() {
        let s = &lines[k].text;
        if is_blank(s) {
            let mut p = k;
            while p < lines.len() && is_blank(&lines[p].text) {
                p += 1;
            }
            if p < lines.len() && indent_of(&lines[p].text) >= m.content {
                internal_blank = true;
                while k < p {
                    inner.push(Ln::new(String::new(), lines[k].num));
                    k += 1;
                }
                continue;
            }
            break;
        }
        if indent_of(s) >= m.content {
            inner.push(Ln::new(strip_indent(s, m.content), lines[k].num));
        } else if marker(s).is_none() && !interrupts_paragraph(s) {
            inner.push(Ln::new(s.trim_start().to_string(), lines[k].num));
        } else {
            break;
        }
        k += 1;
    }
    (inner, k, internal_blank)
}
