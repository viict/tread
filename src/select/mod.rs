//! Visual line selection and yanking.
//!
//! This is the *keyboard* copy path. The mouse is never captured (SPEC.md
//! §Hard constraints #5), so terminal-native click-drag selection keeps working
//! at all times; `v`/`y`/`Y`/`c` exist alongside it, not instead of it.
//!
//! Everything in this module is pure except [`clip::write_fallback`]: the pager
//! builds a [`Yank`] and `main` delivers it, which keeps the pager free of I/O.
//!
//! Granularity: a line selection yanks every *block* it touches, reconstructed
//! as markdown from the AST. Code and HTML blocks, whose rendered rows map 1:1
//! onto source lines, are yanked line-exact instead.
#![deny(unsafe_code)]

pub mod clip;
pub mod source;

#[cfg(test)]
mod tests;

use crate::md::ast::{Block, Document};
use crate::pager::collapse;
use crate::render::{Line, LineKind};
use source::{block_markdown, code_body};

/// An anchored line selection. Both ends are indices into the pager's
/// `visible` list, so the selection follows the cursor through folds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn new(at: usize) -> Selection {
        Selection { anchor: at, head: at }
    }
    pub fn set_head(&mut self, head: usize) {
        self.head = head;
    }
    /// Inclusive `(low, high)`, whichever way the selection was dragged.
    pub fn range(&self) -> (usize, usize) {
        match self.anchor <= self.head {
            true => (self.anchor, self.head),
            false => (self.head, self.anchor),
        }
    }
    pub fn len(&self) -> usize {
        let (lo, hi) = self.range();
        hi - lo + 1
    }
    /// Test-only: the painter asks `selected_rows` instead, because it needs
    /// the mapping through the visible-line list anyway.
    #[cfg(test)]
    pub fn contains(&self, index: usize) -> bool {
        let (lo, hi) = self.range();
        index >= lo && index <= hi
    }
    /// The status-bar text for visual mode.
    pub fn status(&self) -> String {
        format!("-- VISUAL --  {} selected", clip::line_count(self.len()))
    }
}

/// Text waiting to be put on the clipboard, plus what to call it in the status
/// bar ("3 lines", "section “Install”", ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yank {
    pub text: String,
    pub what: String,
}

impl Yank {
    fn new(text: String, what: String) -> Option<Yank> {
        match text.trim().is_empty() {
            true => None,
            false => Some(Yank { text, what }),
        }
    }
}

// ---------------------------------------------------------------------------
// Line selection
// ---------------------------------------------------------------------------

/// Markdown for the rendered rows `rows` (indices into `lines`, ascending).
pub fn selection_text(doc: &Document, lines: &[Line], rows: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let b = lines[rows[i]].block;
        let mut j = i;
        while j < rows.len() && lines[rows[j]].block == b {
            j += 1;
        }
        if let Some(block) = doc.blocks.get(b) {
            let picked: Vec<usize> = rows[i..j]
                .iter()
                .copied()
                .filter(|r| lines[*r].kind != LineKind::Blank)
                .collect();
            if !picked.is_empty() {
                let text = match picked.len() >= block_row_count(lines, b) {
                    true => block_markdown(block),
                    false => partial_markdown(block, lines, &picked),
                };
                if !text.is_empty() {
                    parts.push(text);
                }
            }
        }
        i = j;
    }
    match parts.is_empty() {
        true => String::new(),
        false => format!("{}\n", parts.join("\n\n")),
    }
}

/// Rows the whole block occupies, blank separators excluded.
fn block_row_count(lines: &[Line], block: usize) -> usize {
    lines
        .iter()
        .filter(|l| l.block == block && l.kind != LineKind::Blank)
        .count()
}

/// A partially selected block. Code and HTML keep their per-row source mapping,
/// so those come back line-exact; anything else is yanked whole rather than
/// handing back half a table.
fn partial_markdown(block: &Block, lines: &[Line], rows: &[usize]) -> String {
    match block {
        Block::CodeBlock { lines: body, source_line, .. } => {
            let picked = pick_source_lines(body, lines, rows, *source_line + 1);
            code_body(&picked).trim_end().to_string()
        }
        Block::Html { lines: body, source_line, .. } => {
            let picked = pick_source_lines(body, lines, rows, *source_line);
            picked.join("\n")
        }
        _ => block_markdown(block),
    }
}

/// The entries of `body` whose source lines are covered by `rows`, where
/// `body[0]` sits on source line `first`.
fn pick_source_lines(
    body: &[String],
    lines: &[Line],
    rows: &[usize],
    first: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows {
        let src = lines[*r].source_line;
        if src < first {
            continue; // the language-label row sits on the fence line
        }
        if let Some(text) = body.get(src - first) {
            out.push(text.clone());
        }
    }
    out
}

/// Build the yank for a visual-mode selection.
pub fn selection_yank(doc: &Document, lines: &[Line], rows: &[usize]) -> Option<Yank> {
    Yank::new(selection_text(doc, lines, rows), clip::line_count(rows.len()))
}

// ---------------------------------------------------------------------------
// Sections (`Y`)
// ---------------------------------------------------------------------------

/// Rendered rows `head..end` of the section enclosing `cursor_line`: the
/// heading itself through the end of its collapse range.
pub fn section_range(lines: &[Line], cursor_line: usize) -> Option<(usize, usize)> {
    let head = collapse::heading_at_or_above(lines, cursor_line)?;
    let level = lines[head].heading.as_ref()?.level;
    Some((head, collapse::section_end(lines, head, level)))
}

/// Whole section under the cursor, heading included.
pub fn section_yank(doc: &Document, lines: &[Line], cursor_line: usize) -> Option<Yank> {
    let (head, end) = section_range(lines, cursor_line)?;
    let title = lines[head].heading.as_ref()?.text.clone();
    let first = lines[head].block;
    // Blank separator rows are attributed to the block that follows them, so a
    // section's last block is the last one that actually rendered content.
    let last = lines[head..end]
        .iter()
        .filter(|l| l.kind != LineKind::Blank)
        .map(|l| l.block)
        .max()?;
    let blocks = doc.blocks.get(first..=last)?;
    let text = source::blocks_markdown(blocks);
    Yank::new(text, format!("section \u{201c}{title}\u{201d}"))
}

// ---------------------------------------------------------------------------
// Code blocks (`c`)
// ---------------------------------------------------------------------------

/// A fenced or indented code block found anywhere in the document tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeRef<'a> {
    /// Source line of the opening fence.
    pub source_line: usize,
    pub lang: Option<&'a str>,
    pub lines: &'a [String],
}

impl CodeRef<'_> {
    /// Source lines the block covers, fences included.
    pub fn covers(&self, src: usize) -> bool {
        src >= self.source_line && src <= self.source_line + self.lines.len() + 1
    }
}

/// Every code block in the document, in source order, nesting included.
pub fn code_blocks(blocks: &[Block]) -> Vec<CodeRef<'_>> {
    let mut out = Vec::new();
    walk_code(blocks, &mut out);
    out.sort_by_key(|c| c.source_line);
    out
}

fn walk_code<'a>(blocks: &'a [Block], out: &mut Vec<CodeRef<'a>>) {
    for b in blocks {
        match b {
            Block::CodeBlock { lang, lines, source_line, .. } => out.push(CodeRef {
                source_line: *source_line,
                lang: lang.as_deref(),
                lines,
            }),
            Block::Quote { blocks, .. } | Block::FootnoteDef { blocks, .. } => {
                walk_code(blocks, out)
            }
            Block::List { items, .. } => {
                for it in items {
                    walk_code(&it.blocks, out);
                }
            }
            _ => {}
        }
    }
}

/// The code block containing `src`, else the nearest one below it.
pub fn code_at_or_below(codes: &[CodeRef], src: usize) -> Option<usize> {
    codes
        .iter()
        .position(|c| c.covers(src))
        .or_else(|| codes.iter().position(|c| c.source_line >= src))
}

/// Verbatim source of the code block under (or nearest below) the cursor:
/// exactly what was between the fences, with no styling, gutter or wrapping.
pub fn code_yank(doc: &Document, lines: &[Line], cursor_line: usize) -> Option<Yank> {
    let src = lines.get(cursor_line).map(|l| l.source_line).unwrap_or(0);
    let codes = code_blocks(&doc.blocks);
    let hit = codes.get(code_at_or_below(&codes, src)?)?;
    let n = hit.lines.len();
    let lang = hit.lang.map(|l| format!("{l} ")).unwrap_or_default();
    Yank::new(
        code_body(hit.lines),
        format!("{lang}code block ({})", clip::line_count(n)),
    )
}

/// Yank a bare link target (SPEC.md §Navigation: external links are not
/// opened, but their URL can be copied).
pub fn link_yank(url: &str) -> Option<Yank> {
    Yank::new(format!("{url}\n"), format!("link {url}"))
}
