//! Layout engine: `Document` + width -> `Vec<Line>`.
//!
//! A [`Line`] is one physical terminal row: a list of styled [`Span`]s plus the
//! metadata the pager needs (originating block, source line, heading info,
//! whether the row is horizontally scrollable).
//!
//! Layout never measures with `.len()`; everything goes through
//! [`char_width`] / [`str_width`] (SPEC.md §Width & unicode).
#![deny(unsafe_code)]

mod block;
mod code;
mod inline;
mod list;
mod table;
mod width;
mod wrap;

#[cfg(test)]
mod tests;

// Re-exported for the pager/select modules; a `pub use` in a binary crate does
// not itself count as a use.
#[allow(unused_imports)]
pub use width::{char_width, pad_right, repeat, str_width, take_width, truncate_width};

use crate::md::ast::Document;
use crate::term::Style;

/// A run of text sharing one style, optionally part of a hyperlink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// Target URL when this run is a link: painted as OSC 8 by the pager and
    /// shown in the status bar when it is the cursor target.
    pub link: Option<String>,
}

impl Span {
    pub fn new(text: impl Into<String>, style: Style) -> Span {
        Span { text: text.into(), style, link: None }
    }
    pub fn plain(text: impl Into<String>) -> Span {
        Span::new(text, Style::new())
    }
    pub fn linked(text: impl Into<String>, style: Style, url: impl Into<String>) -> Span {
        Span { text: text.into(), style, link: Some(url.into()) }
    }
    pub fn width(&self) -> usize {
        str_width(&self.text)
    }
}

/// What kind of content produced a line. The pager uses this for yanking
/// (`c` yanks a code block), selection and search scoping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    Heading,
    Paragraph,
    List,
    Code,
    Quote,
    Table,
    Rule,
    Html,
    Footnote,
}

/// Present on the first row of a heading; drives the collapse tree.
///
/// Whether the section is *folded* is deliberately not recorded here. Folding
/// is a pager concern, computed over the rendered line list by
/// [`crate::pager::collapse`], so that fold state survives a re-layout and the
/// renderer stays a pure `Document -> Vec<Line>` function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingLine {
    pub level: u8,
    pub id: String,
    pub text: String,
}

/// One physical row of output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<Span>,
    /// Index into `Document::blocks` of the top-level block this came from.
    pub block: usize,
    /// 1-based source line of the originating block.
    pub source_line: usize,
    /// `Some` only on the row that starts a heading.
    pub heading: Option<HeadingLine>,
    /// True when the row is wider than the viewport and must be scrolled
    /// horizontally rather than wrapped (code blocks, wide tables).
    pub scroll: bool,
    pub kind: LineKind,
}

impl Line {
    /// The ANSI-stripped text of the row (what layout tests assert on).
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
    pub fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }
    pub fn is_blank(&self) -> bool {
        self.text().trim().is_empty()
    }
    /// The link URL covering display column `col`, if any. Test-only: the
    /// pager tracks links by [`Line::links`] sites, not by column probe.
    #[cfg(test)]
    pub fn link_at(&self, col: usize) -> Option<&str> {
        let mut x = 0;
        for s in &self.spans {
            let w = s.width();
            if col < x + w {
                return s.link.as_deref();
            }
            x += w;
        }
        None
    }
    /// Every distinct link on the row, in order, as `(start_col, url)`.
    pub fn links(&self) -> Vec<(usize, &str)> {
        let mut out: Vec<(usize, &str)> = Vec::new();
        let mut x = 0;
        for s in &self.spans {
            if let Some(u) = s.link.as_deref() {
                if out.last().map(|(_, p)| *p != u).unwrap_or(true) {
                    out.push((x, u));
                }
            }
            x += s.width();
        }
        out
    }
}

/// Clip a span list to the display-column window `[offset, offset + width)`.
pub fn slice_spans(spans: &[Span], offset: usize, width: usize) -> Vec<Span> {
    let end = offset.saturating_add(width);
    let mut out: Vec<Span> = Vec::new();
    let mut x = 0;
    for s in spans {
        let mut text = String::new();
        for c in s.text.chars() {
            let w = char_width(c);
            let (cs, ce) = (x, x + w);
            x = ce;
            if ce <= offset || cs >= end {
                continue;
            }
            if cs < offset || ce > end {
                // straddles the window edge: keep the grid, drop the glyph
                let overlap = ce.min(end).saturating_sub(cs.max(offset));
                for _ in 0..overlap {
                    text.push(' ');
                }
            } else {
                text.push(c);
            }
        }
        if !text.is_empty() {
            out.push(Span { text, style: s.style, link: s.link.clone() });
        }
    }
    out
}

/// Seam for future syntax highlighting (SPEC.md §Blocks: "leave a seam").
///
/// Implementors return byte ranges of `line` to restyle; ranges outside the
/// line or overlapping earlier ones are ignored by the renderer.
pub trait Highlighter {
    fn spans(&self, lang: Option<&str>, line: &str) -> Vec<(usize, usize, Style)>;
}

/// The v1 highlighter: none.
pub struct NoHighlight;

impl Highlighter for NoHighlight {
    fn spans(&self, _lang: Option<&str>, _line: &str) -> Vec<(usize, usize, Style)> {
        Vec::new()
    }
}

static NO_HIGHLIGHT: NoHighlight = NoHighlight;

/// Layout knobs.
///
/// There is deliberately no `collapsed` list: the renderer always lays out the
/// whole document and the pager hides folded rows afterwards
/// ([`crate::pager::collapse`]). Two collapse implementations would drift —
/// notably over headings nested inside block quotes, which the pager folds and
/// a block-level skip never sees.
#[derive(Clone, Copy)]
pub struct RenderOpts<'a> {
    pub width: usize,
    pub highlighter: &'a dyn Highlighter,
    /// Reserve the two-column collapse gutter on the left.
    pub gutter: bool,
}

impl Default for RenderOpts<'_> {
    fn default() -> Self {
        RenderOpts { width: 80, highlighter: &NO_HIGHLIGHT, gutter: true }
    }
}

impl<'a> RenderOpts<'a> {
    pub fn new(width: usize) -> Self {
        RenderOpts { width, ..RenderOpts::default() }
    }
    /// Install a syntax highlighter. The v1 binary never calls this — it is the
    /// seam SPEC.md §Blocks asks for, exercised by `render::code`'s tests.
    #[allow(dead_code)]
    pub fn with_highlighter(mut self, h: &'a dyn Highlighter) -> Self {
        self.highlighter = h;
        self
    }
    /// Columns available to content, after the gutter.
    pub fn content_width(&self) -> usize {
        let g = if self.gutter { GUTTER_W } else { 0 };
        self.width.saturating_sub(g).max(MIN_WIDTH)
    }
}

/// Width of the collapse gutter (`▾` + space).
pub const GUTTER_W: usize = 2;
/// Never lay out narrower than this, however small the terminal claims to be.
pub const MIN_WIDTH: usize = 8;

/// Lay out a whole document.
pub fn render_document(doc: &Document, opts: &RenderOpts) -> Vec<Line> {
    block::render_document(doc, opts)
}
