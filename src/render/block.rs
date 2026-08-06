//! Block-level layout: the document walk plus headings, paragraphs, quotes,
//! rules, HTML and footnotes. Lists live in `list.rs`, fenced code in
//! `code.rs`, tables in `table.rs`.
#![deny(unsafe_code)]

use super::inline::flatten;
use super::width::{repeat, str_width};
use super::wrap::{with_prefix, wrap};
use super::{code, list, table};
use super::{HeadingLine, Line, LineKind, RenderOpts, Span, GUTTER_W};
use crate::md::ast::{inline_text, Block, Document, Inline};
use crate::term::Style;
use crate::theme;

/// The left prefix of a block: `first` decorates its first line, `cont` every
/// continuation line (that is what makes hanging indents hang).
#[derive(Clone, Debug, Default)]
pub(crate) struct Pfx {
    pub first: Vec<Span>,
    pub cont: Vec<Span>,
}

impl Pfx {
    pub fn empty() -> Pfx {
        Pfx::default()
    }
    pub fn uniform(spans: Vec<Span>) -> Pfx {
        Pfx { first: spans.clone(), cont: spans }
    }
    pub fn first_width(&self) -> usize {
        self.first.iter().map(Span::width).sum()
    }
    pub fn cont_width(&self) -> usize {
        self.cont.iter().map(Span::width).sum()
    }
    /// Extend both halves with the same spans (nesting a quote in a list, ...).
    pub fn nest(&self, add: &[Span]) -> Pfx {
        let mut p = self.clone();
        p.first.extend(add.iter().cloned());
        p.cont.extend(add.iter().cloned());
        p
    }
}

pub(crate) fn spaces(n: usize) -> Span {
    Span::plain(repeat(' ', n))
}

/// Columns of text a nested block is always guaranteed, however deep it sits.
pub(crate) const MIN_TEXT_W: usize = 8;

/// Accumulates rendered lines for one document.
pub(crate) struct Ctx<'a> {
    pub opts: &'a RenderOpts<'a>,
    pub lines: Vec<Line>,
    pub block: usize,
    pub width: usize,
}

impl<'a> Ctx<'a> {
    fn new(opts: &'a RenderOpts<'a>) -> Ctx<'a> {
        Ctx { opts, lines: Vec::new(), block: 0, width: opts.content_width() }
    }

    /// Emit one row. The collapse gutter is added here so no other code has to
    /// remember it; blank rows stay truly empty (no trailing spaces).
    pub fn emit(
        &mut self,
        spans: Vec<Span>,
        kind: LineKind,
        source_line: usize,
        scroll: bool,
        heading: Option<HeadingLine>,
    ) {
        let mut row = Vec::with_capacity(spans.len() + 1);
        if self.opts.gutter && !spans.is_empty() {
            match &heading {
                // Always the open marker: the pager swaps in `\u{25b8}` when it
                // paints a folded heading, because it is the only thing that
                // knows the fold state.
                Some(_) => row.push(Span::new(
                    format!("{} ", theme::MARKER_OPEN),
                    theme::gutter(),
                )),
                None => row.push(spaces(GUTTER_W)),
            }
        }
        row.extend(spans);
        self.lines.push(Line {
            spans: row,
            block: self.block,
            source_line,
            heading,
            scroll,
            kind,
        });
    }

    pub fn line(&mut self, spans: Vec<Span>, kind: LineKind, source_line: usize) {
        self.emit(spans, kind, source_line, false, None);
    }

    /// Nest a prefix, refusing to grow it past the indent budget.
    ///
    /// Without this, a pathological document (`>>>>>...` five hundred deep, or
    /// a list nested two hundred levels) pushes the text off the right edge and
    /// every line degenerates to a single character. Once the budget is spent
    /// the extra levels simply stop indenting, which keeps the document
    /// readable instead of destroying it.
    pub fn nest(&self, pfx: &Pfx, add: &[Span]) -> Pfx {
        let add_w: usize = add.iter().map(Span::width).sum();
        if pfx.cont_width() + add_w > self.indent_budget() {
            return pfx.clone();
        }
        pfx.nest(add)
    }

    /// Widest prefix we will allow, leaving room for readable text.
    pub fn indent_budget(&self) -> usize {
        self.width.saturating_sub(MIN_TEXT_W)
    }

    /// A blank separator row; collapses runs of blanks and never leads.
    pub fn blank(&mut self, source_line: usize) {
        if self.lines.is_empty() || self.lines.last().map(Line::is_blank).unwrap_or(false) {
            return;
        }
        self.emit(Vec::new(), LineKind::Blank, source_line, false, None);
    }

    /// Wrap inline content under `pfx` and emit it.
    pub fn flow(
        &mut self,
        content: &[Inline],
        base: Style,
        pfx: &Pfx,
        kind: LineKind,
        source_line: usize,
    ) {
        let atoms = flatten(content, base);
        let first = self.width.saturating_sub(pfx.first_width());
        let rest = self.width.saturating_sub(pfx.cont_width());
        for (i, l) in wrap(&atoms, first, rest).into_iter().enumerate() {
            let p = if i == 0 { &pfx.first } else { &pfx.cont };
            self.line(with_prefix(p, l), kind, source_line);
        }
    }
}

/// Lay out a whole document.
///
/// Every block is emitted, folded or not: hiding rows is the pager's job
/// (see [`RenderOpts`]).
pub(crate) fn render_document(doc: &Document, opts: &RenderOpts) -> Vec<Line> {
    let mut ctx = Ctx::new(opts);
    for (i, b) in doc.blocks.iter().enumerate() {
        ctx.block = i;
        ctx.blank(b.source_line());
        render_block(&mut ctx, b, &Pfx::empty());
    }
    while ctx.lines.last().map(Line::is_blank).unwrap_or(false) {
        ctx.lines.pop();
    }
    ctx.lines
}

pub(crate) fn render_blocks(ctx: &mut Ctx, blocks: &[Block], pfx: &Pfx, spaced: bool) {
    for (n, b) in blocks.iter().enumerate() {
        if spaced && n > 0 {
            ctx.blank(b.source_line());
        }
        render_block(ctx, b, pfx);
    }
}

pub(crate) fn render_block(ctx: &mut Ctx, b: &Block, pfx: &Pfx) {
    match b {
        Block::FrontMatter { fields, source_line } => {
            super::frontmatter::render(ctx, fields, *source_line, pfx)
        }
        Block::Heading { level, content, id, source_line } => {
            heading(ctx, *level, content, id, *source_line, pfx)
        }
        Block::Paragraph { content, source_line } => {
            ctx.flow(content, Style::new(), pfx, LineKind::Paragraph, *source_line)
        }
        Block::CodeBlock { lang, lines, source_line, .. } => {
            code::render(ctx, lang.as_deref(), lines, *source_line, pfx)
        }
        Block::List { kind, tight, items, source_line } => {
            list::render(ctx, *kind, *tight, items, *source_line, pfx, 0)
        }
        Block::Quote { blocks, source_line } => quote(ctx, blocks, *source_line, pfx),
        Block::Table { align, head, rows, source_line } => {
            table::render(ctx, align, head, rows, *source_line, pfx)
        }
        Block::ThematicBreak { source_line } => rule(ctx, *source_line, pfx),
        Block::Html { lines, source_line } => html(ctx, lines, *source_line, pfx),
        Block::FootnoteDef { label, blocks, source_line } => {
            footnote(ctx, label, blocks, *source_line, pfx)
        }
    }
}

// ---------------------------------------------------------------------------
// Headings
// ---------------------------------------------------------------------------

fn heading(ctx: &mut Ctx, level: u8, content: &[Inline], id: &str, src: usize, pfx: &Pfx) {
    let text = inline_text(content);
    let info = HeadingLine { level, id: id.to_string(), text: text.clone(), summarised: false };
    let indent = theme::heading_indent(level);
    let hp = pfx.nest(&[spaces(indent)]);
    let avail = ctx.width.saturating_sub(hp.first_width());
    if level == 1 {
        ctx.blank(src);
        match theme::banner(&text, avail) {
            Some(rows) => banner_rows(ctx, &rows, &hp, src, info),
            None => fallback_h1(ctx, &text, &hp, src, info, avail),
        }
        ctx.blank(src);
        return;
    }
    let atoms = flatten(content, theme::heading(level));
    let mut head = Some(info);
    for l in wrap(&atoms, avail, avail) {
        ctx.emit(with_prefix(&hp.first, l), LineKind::Heading, src, false, head.take());
    }
    if level == 2 {
        let bar = Span::new(repeat('\u{2500}', avail), theme::rule());
        ctx.line(with_prefix(&hp.first, vec![bar]), LineKind::Heading, src);
    }
}

fn banner_rows(ctx: &mut Ctx, rows: &[String], pfx: &Pfx, src: usize, info: HeadingLine) {
    let mut head = Some(info);
    for row in rows {
        let spans = vec![Span::new(row.clone(), theme::banner_style())];
        ctx.emit(with_prefix(&pfx.first, spans), LineKind::Heading, src, false, head.take());
    }
}

/// SPEC.md §Headings: bold uppercase plus a rule when the banner will not fit.
fn fallback_h1(
    ctx: &mut Ctx,
    text: &str,
    pfx: &Pfx,
    src: usize,
    info: HeadingLine,
    avail: usize,
) {
    let upper: String = text.to_uppercase();
    let atoms = flatten(&[Inline::Text(upper)], theme::heading(1));
    let mut head = Some(info);
    for l in wrap(&atoms, avail, avail) {
        ctx.emit(with_prefix(&pfx.first, l), LineKind::Heading, src, false, head.take());
    }
    let bar = Span::new(repeat('\u{2501}', avail), theme::rule());
    ctx.line(with_prefix(&pfx.first, vec![bar]), LineKind::Heading, src);
}

// ---------------------------------------------------------------------------
// Quotes, rules, HTML, footnotes
// ---------------------------------------------------------------------------

fn quote(ctx: &mut Ctx, blocks: &[Block], src: usize, pfx: &Pfx) {
    let bar = vec![Span::new("\u{258f} ", theme::quote_bar())];
    let inner = ctx.nest(pfx, &bar);
    if blocks.is_empty() {
        ctx.line(inner.first.clone(), LineKind::Quote, src);
        return;
    }
    let before = ctx.lines.len();
    render_blocks(ctx, blocks, &inner, true);
    // Paragraph text inside a quote is muted; headings/code keep their own
    // colours, so only untouched default-styled spans are recoloured.
    for line in ctx.lines[before..].iter_mut() {
        if line.kind == LineKind::Paragraph {
            line.kind = LineKind::Quote;
            for s in line.spans.iter_mut() {
                if s.style.fg.is_none() && s.style.bg.is_none() && !s.text.trim().is_empty() {
                    s.style = s.style.fg(theme::QUOTE_FG);
                }
            }
        }
    }
}

fn rule(ctx: &mut Ctx, src: usize, pfx: &Pfx) {
    let avail = ctx.width.saturating_sub(pfx.first_width());
    let bar = Span::new(repeat('\u{2500}', avail), theme::rule());
    ctx.line(with_prefix(&pfx.first, vec![bar]), LineKind::Rule, src);
}

/// HTML blocks render as dim literals, never wrapped.
fn html(ctx: &mut Ctx, lines: &[String], src: usize, pfx: &Pfx) {
    let avail = ctx.width.saturating_sub(pfx.first_width());
    for (i, l) in lines.iter().enumerate() {
        let scroll = str_width(l) > avail;
        let spans = with_prefix(&pfx.first, vec![Span::new(l.clone(), theme::muted().dim())]);
        ctx.emit(spans, LineKind::Html, src + i, scroll, None);
    }
}

/// Footnote definitions render as text: a bold label, then the body indented.
fn footnote(ctx: &mut Ctx, label: &str, blocks: &[Block], src: usize, pfx: &Pfx) {
    let tag = Span::new(format!("[^{}] ", label), theme::muted().bold());
    let width = tag.width();
    let inner = Pfx {
        first: with_prefix(&pfx.first, vec![tag]),
        cont: with_prefix(&pfx.cont, vec![spaces(width)]),
    };
    if blocks.is_empty() {
        ctx.line(inner.first.clone(), LineKind::Footnote, src);
        return;
    }
    let before = ctx.lines.len();
    render_block(ctx, &blocks[0], &inner);
    let rest = Pfx::uniform(with_prefix(&pfx.cont, vec![spaces(width)]));
    render_blocks(ctx, &blocks[1..], &rest, true);
    for line in ctx.lines[before..].iter_mut() {
        if line.kind == LineKind::Paragraph {
            line.kind = LineKind::Footnote;
        }
    }
}
