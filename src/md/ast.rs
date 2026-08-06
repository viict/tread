//! Markdown AST types shared by the block parser, the inline parser and the
//! renderer. Pure data + a couple of tiny text helpers; no I/O, no unsafe.
#![deny(unsafe_code)]

use std::collections::HashMap;

/// Link reference definitions: normalized label -> (destination, title).
pub type LinkRefs = HashMap<String, (String, Option<String>)>;

/// A fully parsed markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub link_refs: LinkRefs,
}

/// Column alignment of a GFM table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// Bullet or ordered list (ordered lists keep their source start number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Ordered { start: u64 },
}

/// One list item. `task` is `Some(checked)` for GFM task-list items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

/// A block-level element. Every variant carries the 1-based source line it
/// started on so the pager can map search hits and outline entries back to
/// the file.
// `CodeBlock` deliberately keeps the markdown term of art even though it ends
// with the enum name; `Block::Code` would collide conceptually with
// `Inline::Code`, which is a different thing.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
        /// GitHub-style slug, deduplicated document-wide (`foo`, `foo-1`, ...).
        id: String,
        source_line: usize,
    },
    Paragraph {
        content: Vec<Inline>,
        source_line: usize,
    },
    CodeBlock {
        lang: Option<String>,
        lines: Vec<String>,
        fenced: bool,
        source_line: usize,
    },
    List {
        kind: ListKind,
        tight: bool,
        items: Vec<ListItem>,
        source_line: usize,
    },
    Quote {
        blocks: Vec<Block>,
        source_line: usize,
    },
    Table {
        align: Vec<Align>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        source_line: usize,
    },
    ThematicBreak {
        source_line: usize,
    },
    Html {
        lines: Vec<String>,
        source_line: usize,
    },
    FootnoteDef {
        label: String,
        blocks: Vec<Block>,
        source_line: usize,
    },
}

impl Block {
    /// 1-based line in the source file where this block starts.
    pub fn source_line(&self) -> usize {
        match self {
            Block::Heading { source_line, .. }
            | Block::Paragraph { source_line, .. }
            | Block::CodeBlock { source_line, .. }
            | Block::List { source_line, .. }
            | Block::Quote { source_line, .. }
            | Block::Table { source_line, .. }
            | Block::ThematicBreak { source_line }
            | Block::Html { source_line, .. }
            | Block::FootnoteDef { source_line, .. } => *source_line,
        }
    }
}

/// An inline (span-level) element. The parser for these lives in
/// `md::inline`; the type lives here so every module agrees on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        text: Vec<Inline>,
        url: String,
        title: Option<String>,
    },
    Image {
        alt: String,
        url: String,
    },
    Autolink(String),
    SoftBreak,
    HardBreak,
    FootnoteRef(String),
    Html(String),
}

/// Flatten inlines to their plain-text content (used for slugs, search,
/// outline entries and table width measurement fallbacks).
pub fn inline_text(items: &[Inline]) -> String {
    let mut out = String::new();
    push_text(items, &mut out);
    out
}

fn push_text(items: &[Inline], out: &mut String) {
    for it in items {
        match it {
            Inline::Text(s) | Inline::Code(s) | Inline::Autolink(s) => out.push_str(s),
            Inline::Emph(k) | Inline::Strong(k) | Inline::Strike(k) => push_text(k, out),
            Inline::Link { text, .. } => push_text(text, out),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            Inline::FootnoteRef(l) => {
                out.push_str("[^");
                out.push_str(l);
                out.push(']');
            }
            Inline::Html(_) => {}
        }
    }
}

/// GitHub-style anchor slug: lowercase, punctuation dropped, spaces to `-`.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
        } else if ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch == ' ' || ch == '\t' {
            out.push('-');
        }
    }
    out
}

/// Assigns document-unique slugs, GitHub-style (`x`, `x-1`, `x-2`, ...).
#[derive(Debug, Default)]
pub struct SlugSet {
    seen: HashMap<String, usize>,
}

impl SlugSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn unique(&mut self, text: &str) -> String {
        let base = slugify(text);
        match self.seen.get_mut(&base) {
            Some(n) => {
                *n += 1;
                format!("{}-{}", base, *n)
            }
            None => {
                self.seen.insert(base.clone(), 0);
                base
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    #[test]
    fn slug_matches_github_shape() {
        assert_eq!(slugify("How to read this"), "how-to-read-this");
        assert_eq!(slugify("Sample/ supersedes old/"), "sample-supersedes-old");
        assert_eq!(slugify("§2g-i: DNS & tiering!"), "2g-i-dns--tiering");
        assert_eq!(slugify("`provisioner migrate`"), "provisioner-migrate");
        assert_eq!(slugify("Example Codex"), "example-codex");
    }

    #[test]
    fn slug_dedup_suffixes() {
        let mut set = SlugSet::new();
        assert_eq!(set.unique("References"), "references");
        assert_eq!(set.unique("References"), "references-1");
        assert_eq!(set.unique("References"), "references-2");
        assert_eq!(set.unique("Other"), "other");
    }

    #[test]
    fn inline_text_flattens_nesting() {
        let items = vec![
            t("a "),
            Inline::Strong(vec![Inline::Emph(vec![t("b")])]),
            Inline::Link {
                text: vec![t(" c")],
                url: "u".into(),
                title: None,
            },
            Inline::SoftBreak,
            Inline::Code("d".into()),
            Inline::Image {
                alt: "e".into(),
                url: "u".into(),
            },
            Inline::FootnoteRef("1".into()),
        ];
        assert_eq!(inline_text(&items), "a b c de[^1]");
    }

    #[test]
    fn block_source_line_accessor() {
        let b = Block::ThematicBreak { source_line: 7 };
        assert_eq!(b.source_line(), 7);
        let h = Block::Heading {
            level: 2,
            content: vec![t("x")],
            id: "x".into(),
            source_line: 12,
        };
        assert_eq!(h.source_line(), 12);
    }
}
