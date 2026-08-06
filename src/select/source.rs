//! AST -> markdown source text.
//!
//! Yanked text is never scraped off the rendered rows: no ANSI, no gutter, no
//! collapse arrow, no injected wrap. Everything here reconstructs markdown from
//! the parsed [`Block`]s, so a pasted table is still a table and a pasted list
//! is still a list (SPEC.md §Keybindings, `y`/`Y`/`c`).
#![deny(unsafe_code)]

use crate::md::ast::{Align, Block, Inline, ListItem, ListKind};

/// Markdown for a run of blocks, blank-line separated, newline terminated.
pub fn blocks_markdown(blocks: &[Block]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for b in blocks {
        let s = block_markdown(b);
        if !s.is_empty() {
            parts.push(s);
        }
    }
    match parts.is_empty() {
        true => String::new(),
        false => format!("{}\n", parts.join("\n\n")),
    }
}

/// Markdown for one block, without a trailing newline.
pub fn block_markdown(b: &Block) -> String {
    match b {
        Block::Heading { level, content, .. } => {
            format!("{} {}", "#".repeat(*level as usize), inline_markdown(content))
        }
        Block::Paragraph { content, .. } => inline_markdown(content),
        Block::CodeBlock { lang, lines, fenced, .. } => code_markdown(lang.as_deref(), lines, *fenced),
        Block::List { kind, tight, items, .. } => list_markdown(*kind, *tight, items),
        Block::Quote { blocks, .. } => prefix_lines(blocks_markdown(blocks).trim_end(), "> ", "> "),
        Block::Table { align, head, rows, .. } => table_markdown(align, head, rows),
        Block::ThematicBreak { .. } => "---".to_string(),
        Block::Html { lines, .. } => lines.join("\n"),
        Block::FootnoteDef { label, blocks, .. } => {
            let body = blocks_markdown(blocks);
            format!("[^{}]: {}", label, prefix_lines(body.trim_end(), "", "    ").trim_start())
        }
    }
}

/// A fence long enough to survive backticks inside the body.
pub fn code_markdown(lang: Option<&str>, lines: &[String], fenced: bool) -> String {
    if !fenced {
        return prefix_lines(&lines.join("\n"), "    ", "    ");
    }
    let longest = lines
        .iter()
        .flat_map(|l| l.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{}{}\n{}\n{}", fence, lang.unwrap_or(""), lines.join("\n"), fence)
}

/// The verbatim body of a code block: exactly what sat between the fences.
pub fn code_body(lines: &[String]) -> String {
    match lines.is_empty() {
        true => String::new(),
        false => format!("{}\n", lines.join("\n")),
    }
}

/// A tight list keeps its items on adjacent lines; a loose one blank-separates
/// them, exactly as the source did.
fn list_markdown(kind: ListKind, tight: bool, items: &[ListItem]) -> String {
    let sep = if tight { "\n" } else { "\n\n" };
    let mut out: Vec<String> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let marker = match kind {
            ListKind::Bullet => "- ".to_string(),
            ListKind::Ordered { start } => format!("{}. ", start + n as u64),
        };
        let task = match item.task {
            Some(true) => "[x] ",
            Some(false) => "[ ] ",
            None => "",
        };
        let body = item_body(&item.blocks, tight);
        let body = format!("{task}{}", body.trim_end().trim_start_matches(' '));
        let pad = " ".repeat(marker.len());
        out.push(prefix_lines(&body, &marker, &pad));
    }
    out.join(sep)
}

/// The blocks of one list item, separated the way the list was written.
fn item_body(blocks: &[Block], tight: bool) -> String {
    if !tight {
        return blocks_markdown(blocks);
    }
    let parts: Vec<String> = blocks
        .iter()
        .map(block_markdown)
        .filter(|s| !s.is_empty())
        .collect();
    parts.join("\n")
}

fn table_markdown(align: &[Align], head: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>]) -> String {
    let cols = align.len().max(head.len()).max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut out = vec![row_markdown(head, cols), delimiter_row(align, cols)];
    for r in rows {
        out.push(row_markdown(r, cols));
    }
    out.join("\n")
}

fn row_markdown(cells: &[Vec<Inline>], cols: usize) -> String {
    let mut s = String::from("|");
    for c in 0..cols {
        let text = cells.get(c).map(|v| inline_markdown(v)).unwrap_or_default();
        let text = text.replace('\n', " ");
        s.push(' ');
        s.push_str(text.trim());
        s.push_str(" |");
    }
    s
}

fn delimiter_row(align: &[Align], cols: usize) -> String {
    let mut s = String::from("|");
    for c in 0..cols {
        s.push(' ');
        s.push_str(match align.get(c).copied().unwrap_or(Align::None) {
            Align::None => "---",
            Align::Left => ":---",
            Align::Center => ":---:",
            Align::Right => "---:",
        });
        s.push_str(" |");
    }
    s
}

/// Re-prefix a multi-line string: `first` on line one, `rest` on the others.
/// Blank lines keep the prefix trimmed so no trailing whitespace is yanked.
pub fn prefix_lines(text: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let p = if i == 0 { first } else { rest };
        match line.is_empty() {
            true => out.push_str(p.trim_end()),
            false => {
                out.push_str(p);
                out.push_str(line);
            }
        }
    }
    out
}

/// Markdown for inline content.
pub fn inline_markdown(items: &[Inline]) -> String {
    let mut out = String::new();
    for it in items {
        match it {
            Inline::Text(s) => out.push_str(s),
            Inline::Code(s) => out.push_str(&code_span(s)),
            Inline::Emph(k) => out.push_str(&format!("*{}*", inline_markdown(k))),
            Inline::Strong(k) => out.push_str(&format!("**{}**", inline_markdown(k))),
            Inline::Strike(k) => out.push_str(&format!("~~{}~~", inline_markdown(k))),
            Inline::Link { text, url, title } => {
                let t = match title {
                    Some(t) => format!(" \"{t}\""),
                    None => String::new(),
                };
                out.push_str(&format!("[{}]({}{})", inline_markdown(text), url, t));
            }
            Inline::Image { alt, url } => out.push_str(&format!("![{alt}]({url})")),
            Inline::Autolink(u) => out.push_str(&format!("<{u}>")),
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("  \n"),
            Inline::FootnoteRef(l) => out.push_str(&format!("[^{l}]")),
            Inline::Html(h) => out.push_str(h),
        }
    }
    out
}

/// `` `x` ``, widening the delimiter when the content holds backticks.
fn code_span(s: &str) -> String {
    let longest = s.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let tick = "`".repeat(longest + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') { " " } else { "" };
    format!("{tick}{pad}{s}{pad}{tick}")
}
