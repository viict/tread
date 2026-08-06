//! Parsing an index document into a navigable corpus listing.
//!
//! The codex README is the shape this targets: H2 sections, each holding a
//! table whose first column is `[path/DOC.md](path/DOC.md) — description` and
//! whose remaining columns are metadata (status, owner, date). Links outside
//! tables (paragraphs, lists) are picked up too, so a plainer index still
//! works.
//!
//! Entries are deduplicated by resolved target: the codex links some ADRs from
//! both "Future directions" and "Decisions", and the list view — plus `]`/`[`
//! sequential reading — wants each document exactly once, under the first
//! section that mentioned it.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use super::link::{self, Fs, Target};
use crate::md::ast::{inline_text, Block, Document, Inline, ListItem};
use crate::render::{pad_right, str_width, truncate_width};

/// One linked document in the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The H2 (or H1, before the first H2) the link appeared under.
    pub section: String,
    /// Link text.
    pub title: String,
    /// Resolved absolute path of the target document.
    pub path: PathBuf,
    pub anchor: Option<String>,
    /// Trailing text after the link, or the row's other columns.
    pub desc: String,
}

/// Columns the section name gets in the list view before it is elided.
const SECTION_W: usize = 14;

impl Entry {
    /// One list-view row: `section · title — desc`.
    ///
    /// The section column is measured and padded in *display* columns, not
    /// chars: `{:<14}` counts chars, so a CJK section name (two cells per char)
    /// would push every title on that row out of alignment
    /// (SPEC.md §Width & unicode).
    pub fn row(&self) -> String {
        let section = truncate(&self.section, SECTION_W);
        let mut s = format!("{}  {}", pad_right(&section, SECTION_W), self.title);
        if !self.desc.is_empty() {
            s.push_str("  \u{2014} ");
            s.push_str(&self.desc);
        }
        s
    }

    /// Text `/` filters on: title, description and path all count.
    pub fn haystack(&self) -> String {
        format!(
            "{} {} {} {}",
            self.section,
            self.title,
            self.desc,
            self.path.to_string_lossy()
        )
        .to_lowercase()
    }
}

/// `s` cut to at most `n` display columns, with `…` marking the cut. Wide
/// characters count as two, so the result never overflows the column.
fn truncate(s: &str, n: usize) -> String {
    if str_width(s) <= n {
        return s.to_string();
    }
    let mut out = truncate_width(s, n.saturating_sub(1)).to_string();
    out.push('\u{2026}');
    out
}

/// Parse an index document. `index_dir` is the directory holding it, `root` the
/// corpus root; both must be normalized absolute paths.
pub fn parse(doc: &Document, index_dir: &Path, root: &Path, fs: &dyn Fs) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut section = String::new();
    for block in &doc.blocks {
        match block {
            Block::Heading { level, content, .. } if *level <= 2 => {
                section = inline_text(content).trim().to_string();
            }
            Block::Table { rows, .. } => {
                for row in rows {
                    if let Some(e) = row_entry(&section, row, index_dir, root, fs) {
                        push_unique(&mut out, e);
                    }
                }
            }
            Block::Paragraph { content, .. } => {
                collect(&mut out, &section, content, index_dir, root, fs);
            }
            Block::List { items, .. } => list_entries(&mut out, &section, items, index_dir, root, fs),
            _ => {}
        }
    }
    out
}

fn list_entries(
    out: &mut Vec<Entry>,
    section: &str,
    items: &[ListItem],
    dir: &Path,
    root: &Path,
    fs: &dyn Fs,
) {
    for item in items {
        for b in &item.blocks {
            if let Block::Paragraph { content, .. } = b {
                collect(out, section, content, dir, root, fs);
            }
        }
    }
}

/// The first document link in a table row, described by that row.
fn row_entry(
    section: &str,
    row: &[Vec<Inline>],
    dir: &Path,
    root: &Path,
    fs: &dyn Fs,
) -> Option<Entry> {
    let (col, mut entry) = row
        .iter()
        .enumerate()
        .find_map(|(i, cell)| first_doc_link(section, cell, dir, root, fs).map(|e| (i, e)))?;
    if entry.desc.is_empty() {
        let rest: Vec<String> = row
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != col)
            .map(|(_, c)| inline_text(c).trim().to_string())
            .filter(|s| !s.is_empty() && s != "\u{2014}" && s != "-")
            .collect();
        entry.desc = rest.join("  \u{b7}  ");
    }
    Some(entry)
}

/// Every document link in an inline run (used outside tables).
fn collect(
    out: &mut Vec<Entry>,
    section: &str,
    items: &[Inline],
    dir: &Path,
    root: &Path,
    fs: &dyn Fs,
) {
    if let Some(e) = first_doc_link(section, items, dir, root, fs) {
        push_unique(out, e);
    }
}

/// The first link in `items` that resolves to a markdown document, with the
/// text that trails it as its description.
fn first_doc_link(
    section: &str,
    items: &[Inline],
    dir: &Path,
    root: &Path,
    fs: &dyn Fs,
) -> Option<Entry> {
    for (i, it) in items.iter().enumerate() {
        let (text, url) = match it {
            Inline::Link { text, url, .. } => (inline_text(text), url.as_str()),
            _ => continue,
        };
        let (path, anchor) = match link::resolve(url, dir, root, fs) {
            Target::Doc { path, anchor } => (path, anchor),
            _ => continue,
        };
        return Some(Entry {
            section: section.to_string(),
            title: text.trim().to_string(),
            path,
            anchor,
            desc: trailing(&items[i + 1..]),
        });
    }
    None
}

/// Text after the link, with the codex's `— ` separator stripped.
fn trailing(items: &[Inline]) -> String {
    let raw = inline_text(items);
    let t = raw.trim();
    let t = t
        .trim_start_matches(['\u{2014}', '\u{2013}', '-', ':'])
        .trim();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One row per *document*: a second link to the same file (with or without an
/// anchor) is the same entry as far as the list view and `]`/`[` are concerned.
fn push_unique(out: &mut Vec<Entry>, e: Entry) {
    // `same_path`, not `==`: on Windows two links that differ only in case or
    // separator flavour point at one file and must not become two rows.
    if out.iter().any(|x| link::same_path(&x.path, &e.path)) {
        return;
    }
    out.push(e);
}

/// Every raw link destination in a document, in order. Used by root discovery
/// to ask "does this README link to the file we opened?".
pub fn raw_links(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for b in &doc.blocks {
        block_links(b, &mut out);
    }
    out
}

fn block_links(b: &Block, out: &mut Vec<String>) {
    match b {
        Block::Heading { content, .. } | Block::Paragraph { content, .. } => {
            inline_links(content, out)
        }
        Block::Table { head, rows, .. } => {
            for cell in head {
                inline_links(cell, out);
            }
            for row in rows {
                for cell in row {
                    inline_links(cell, out);
                }
            }
        }
        Block::List { items, .. } => {
            for it in items {
                for b in &it.blocks {
                    block_links(b, out);
                }
            }
        }
        Block::Quote { blocks, .. } | Block::FootnoteDef { blocks, .. } => {
            for b in blocks {
                block_links(b, out);
            }
        }
        _ => {}
    }
}

fn inline_links(items: &[Inline], out: &mut Vec<String>) {
    for it in items {
        match it {
            Inline::Link { url, text, .. } => {
                out.push(url.clone());
                inline_links(text, out);
            }
            Inline::Autolink(u) => out.push(u.clone()),
            Inline::Emph(k) | Inline::Strong(k) | Inline::Strike(k) => inline_links(k, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    fn entry(section: &str, title: &str, desc: &str) -> Entry {
        Entry {
            section: section.to_string(),
            title: title.to_string(),
            path: PathBuf::from("/corpus/x.md"),
            anchor: None,
            desc: desc.to_string(),
        }
    }

    #[test]
    fn the_section_column_is_padded_in_display_columns() {
        let ascii = entry("Models", "DNS", "");
        assert_eq!(str_width(&ascii.row()[..SECTION_W]), SECTION_W);
        // Seven wide chars are fourteen columns: exactly full, no padding.
        let wide = entry("\u{6a21}\u{578b}\u{6a21}\u{578b}\u{6a21}\u{578b}\u{6a21}", "DNS", "");
        let row = wide.row();
        let head: String = row.strip_suffix("  DNS").expect("title suffix").to_string();
        assert_eq!(str_width(&head), SECTION_W);
    }

    #[test]
    fn titles_line_up_across_ascii_and_wide_sections() {
        let a = entry("Models", "DNS", "");
        let b = entry("\u{6a21}\u{578b}", "DNS", "");
        let col = |row: &str| str_width(row.split("DNS").next().unwrap());
        assert_eq!(col(&a.row()), col(&b.row()));
    }

    #[test]
    fn an_overlong_section_is_elided_to_the_column_width() {
        let e = entry("An Extremely Long Section Name", "T", "");
        let head = e.row();
        let cut = head.split("  T").next().unwrap();
        assert!(str_width(cut) <= SECTION_W, "{cut:?}");
        assert!(cut.contains('\u{2026}'));
    }

    #[test]
    fn a_wide_section_is_never_cut_mid_column() {
        // Eight wide chars is sixteen columns; the elision must land on a char
        // boundary and still fit.
        let e = entry(&"\u{6a21}".repeat(8), "T", "");
        let cut = e.row();
        let head = cut.split("  T").next().unwrap();
        assert!(str_width(head) <= SECTION_W, "{head:?} too wide");
    }

    #[test]
    fn the_description_follows_the_title() {
        let e = entry("S", "Title", "a note");
        assert!(e.row().ends_with("Title  \u{2014} a note"));
        assert!(entry("S", "Title", "").row().ends_with("Title"));
    }
}
