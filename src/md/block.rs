//! Block-level markdown parser: a line-oriented recursive descent over
//! container blocks. No I/O, no unsafe. Inline content is handed to
//! `md::inline::parse_inlines`; line scanners live in `md::scan`.
#![deny(unsafe_code)]

use super::ast::{inline_text, Block, Document, Inline, LinkRefs, SlugSet};
use super::inline::parse_inlines;
use super::scan::{
    atx, closes_fence, collect_refs, fence_at, footnote_def_at, html_start, indent_of,
    interrupts_paragraph, is_blank, is_thematic_break, link_ref_at, quote_start, setext,
    strip_indent, strip_quote, Fence, Ln,
};
use super::{list, table};

/// Parse a whole document: strip YAML frontmatter, collect link reference
/// definitions, parse blocks, then assign unique heading slugs.
pub fn parse_document(src: &str) -> Document {
    let mut lines: Vec<Ln> = Vec::new();
    for (i, raw) in src.split('\n').enumerate() {
        lines.push(Ln::new(raw.trim_end_matches('\r').to_string(), i + 1));
    }
    if src.ends_with('\n') {
        lines.pop();
    }
    let body = strip_frontmatter(&lines);
    let link_refs = collect_refs(body);
    let mut blocks = parse_blocks(body, &link_refs);
    let mut slugs = SlugSet::new();
    assign_ids(&mut blocks, &mut slugs);
    Document { blocks, link_refs }
}

/// Skip a leading `---` … `---` YAML frontmatter block. The codex corpus puts
/// status/owner metadata there; it is metadata, not document content.
fn strip_frontmatter(lines: &[Ln]) -> &[Ln] {
    if lines.first().map(|l| l.text.trim_end()) != Some("---") {
        return lines;
    }
    for (i, l) in lines.iter().enumerate().skip(1) {
        let t = l.text.trim_end();
        if t == "---" || t == "..." {
            return &lines[i + 1..];
        }
    }
    lines
}

/// Assign document-unique GitHub-style slugs in document order.
fn assign_ids(blocks: &mut [Block], slugs: &mut SlugSet) {
    for b in blocks.iter_mut() {
        match b {
            Block::Heading { content, id, .. } => *id = slugs.unique(&inline_text(content)),
            Block::Quote { blocks, .. } | Block::FootnoteDef { blocks, .. } => {
                assign_ids(blocks, slugs)
            }
            Block::List { items, .. } => {
                for it in items.iter_mut() {
                    assign_ids(&mut it.blocks, slugs);
                }
            }
            _ => {}
        }
    }
}

/// The block loop. Order matters: fences and indented code shadow everything,
/// thematic breaks beat list markers, and setext underlines are resolved
/// inside `paragraph` so they beat thematic breaks after a paragraph.
pub(crate) fn parse_blocks(lines: &[Ln], refs: &LinkRefs) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let s = &lines[i].text;
        if is_blank(s) {
            i += 1;
        } else if let Some(f) = fence_at(s) {
            i = fenced_code(lines, i, f, &mut out);
        } else if indent_of(s) >= 4 {
            i = indented_code(lines, i, &mut out);
        } else if is_thematic_break(s) {
            out.push(Block::ThematicBreak {
                source_line: lines[i].num,
            });
            i += 1;
        } else if let Some((level, text)) = atx(s) {
            out.push(heading(level, &text, refs, lines[i].num));
            i += 1;
        } else if quote_start(s) {
            i = quote(lines, i, refs, &mut out);
        } else if let Some((label, rest)) = footnote_def_at(s) {
            i = footnote(lines, i, label, rest, refs, &mut out);
        } else if link_ref_at(s).is_some() {
            i += 1; // gathered by the pre-scan; renders as nothing
        } else if list::marker(s).is_some() {
            i = list::parse_list(lines, i, refs, &mut out);
        } else if html_start(s) {
            i = html(lines, i, &mut out);
        } else if let Some(n) = table::parse_table(lines, i, refs, &mut out) {
            i = n;
        } else {
            i = paragraph(lines, i, refs, &mut out);
        }
    }
    out
}

fn heading(level: u8, text: &str, refs: &LinkRefs, source_line: usize) -> Block {
    Block::Heading {
        level,
        content: parse_inlines(text, refs),
        id: String::new(),
        source_line,
    }
}

fn fenced_code(lines: &[Ln], i: usize, f: Fence, out: &mut Vec<Block>) -> usize {
    let mut body = Vec::new();
    let mut j = i + 1;
    while j < lines.len() {
        if closes_fence(&lines[j].text, f.ch, f.len) {
            j += 1;
            break;
        }
        body.push(strip_indent(&lines[j].text, f.indent));
        j += 1;
    }
    out.push(Block::CodeBlock {
        lang: f.info.split_whitespace().next().map(|s| s.to_string()),
        lines: body,
        fenced: true,
        source_line: lines[i].num,
    });
    j
}

fn indented_code(lines: &[Ln], i: usize, out: &mut Vec<Block>) -> usize {
    let mut body: Vec<String> = Vec::new();
    let mut j = i;
    let mut pending = 0usize;
    while j < lines.len() {
        let s = &lines[j].text;
        if is_blank(s) {
            pending += 1;
        } else if indent_of(s) >= 4 {
            body.extend(std::iter::repeat(String::new()).take(pending));
            pending = 0;
            body.push(strip_indent(s, 4));
        } else {
            break;
        }
        j += 1;
    }
    out.push(Block::CodeBlock {
        lang: None,
        lines: body,
        fenced: false,
        source_line: lines[i].num,
    });
    j - pending
}

fn html(lines: &[Ln], i: usize, out: &mut Vec<Block>) -> usize {
    let comment = lines[i].text.trim_start().starts_with("<!--");
    let mut body = Vec::new();
    let mut j = i;
    while j < lines.len() {
        let s = &lines[j].text;
        if !comment && is_blank(s) {
            break;
        }
        body.push(s.clone());
        j += 1;
        if comment && s.contains("-->") {
            break;
        }
    }
    out.push(Block::Html {
        lines: body,
        source_line: lines[i].num,
    });
    j
}

/// `>` quotes, including nesting (handled by recursion) and lazy
/// continuation lines that omit the marker.
fn quote(lines: &[Ln], i: usize, refs: &LinkRefs, out: &mut Vec<Block>) -> usize {
    let mut inner: Vec<Ln> = Vec::new();
    let mut j = i;
    while j < lines.len() {
        let s = &lines[j].text;
        if quote_start(s) {
            inner.push(Ln::new(strip_quote(s), lines[j].num));
        } else if !interrupts_paragraph(s) && inner.last().is_some_and(|l| !is_blank(&l.text)) {
            inner.push(Ln::new(s.trim_start().to_string(), lines[j].num));
        } else {
            break;
        }
        j += 1;
    }
    out.push(Block::Quote {
        blocks: parse_blocks(&inner, refs),
        source_line: lines[i].num,
    });
    j
}

fn footnote(
    lines: &[Ln],
    i: usize,
    label: String,
    rest: String,
    refs: &LinkRefs,
    out: &mut Vec<Block>,
) -> usize {
    let mut inner = vec![Ln::new(rest, lines[i].num)];
    let mut j = i + 1;
    while j < lines.len() {
        let s = &lines[j].text;
        if is_blank(s) {
            let p = skip_blanks(lines, j);
            if p < lines.len() && indent_of(&lines[p].text) >= 4 {
                push_blanks(&mut inner, lines, j, p);
                j = p;
                continue;
            }
            break;
        } else if indent_of(s) >= 4 {
            inner.push(Ln::new(strip_indent(s, 4), lines[j].num));
        } else if !interrupts_paragraph(s) {
            inner.push(Ln::new(s.trim_start().to_string(), lines[j].num));
        } else {
            break;
        }
        j += 1;
    }
    out.push(Block::FootnoteDef {
        label,
        blocks: parse_blocks(&inner, refs),
        source_line: lines[i].num,
    });
    j
}

pub(crate) fn skip_blanks(lines: &[Ln], from: usize) -> usize {
    let mut p = from;
    while p < lines.len() && is_blank(&lines[p].text) {
        p += 1;
    }
    p
}

pub(crate) fn push_blanks(inner: &mut Vec<Ln>, lines: &[Ln], from: usize, to: usize) {
    for l in &lines[from..to] {
        inner.push(Ln::new(String::new(), l.num));
    }
}

/// A paragraph, which may instead turn out to be a setext heading or be
/// interrupted by a GFM table.
fn paragraph(lines: &[Ln], i: usize, refs: &LinkRefs, out: &mut Vec<Block>) -> usize {
    let mut buf: Vec<&str> = Vec::new();
    let mut j = i;
    while j < lines.len() {
        let s = &lines[j].text;
        if is_blank(s) {
            break;
        }
        if !buf.is_empty() {
            if let Some(level) = setext(s) {
                let text = buf.join("\n");
                out.push(heading(level, text.trim(), refs, lines[i].num));
                return j + 1;
            }
            if interrupts_paragraph(s) || table::detect(lines, j).is_some() {
                break;
            }
        }
        buf.push(s.trim_start());
        j += 1;
    }
    out.push(Block::Paragraph {
        content: inlines_of(&buf, refs),
        source_line: lines[i].num,
    });
    j
}

/// Join continuation lines with `\n` so the inline parser can turn them into
/// soft breaks (and see trailing double spaces as hard breaks).
pub(crate) fn inlines_of(buf: &[&str], refs: &LinkRefs) -> Vec<Inline> {
    parse_inlines(buf.join("\n").trim_end(), refs)
}

#[cfg(test)]
mod tests;
