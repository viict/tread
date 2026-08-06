//! Painting: turn pager state into one buffered [`Frame`].
//!
//! Everything here is additive over the laid-out spans — search highlights, the
//! cursor row tint, fold summaries and the horizontal-cut indicators are
//! applied as cell edits so the layout engine stays unaware of them.
#![deny(unsafe_code)]

use super::{keys, search, Mode, Pager};
use crate::render::{slice_spans, str_width, truncate_width, Span};
use crate::term::{Frame, Style};
use crate::theme;

mod cells;
use cells::Cells;

/// Left/right "content continues" markers for horizontally scrolled rows.
const CUT_LEFT: char = '\u{2039}';
const CUT_RIGHT: char = '\u{203a}';

pub fn paint(p: &Pager, frame: &mut Frame) {
    frame.reset();
    let body = p.body_rows();
    match p.mode {
        Mode::Outline => overlay(p, frame, "Outline", &outline_rows(p), p.outline_sel),
        Mode::Help => overlay(p, frame, "Keys", &help_rows(), p.help_top),
        Mode::Index => overlay(p, frame, &index_title(p), &index_rows(p), p.index_row_pos()),
        _ => document(p, frame, body),
    }
    if p.rows > 0 {
        frame.move_to(p.rows as u16, 1);
        status(p, frame);
    }
}

fn document(p: &Pager, frame: &mut Frame, body: usize) {
    for r in 0..body {
        frame.move_to(r as u16 + 1, 1);
        if let Some(li) = p.visible.get(p.top + r).copied() {
            let spans = row_spans(p, li);
            paint_spans(frame, &spans);
        }
        frame.reset_style();
        frame.clear_to_eol();
    }
}

/// The final spans of one document row: folded-heading summary, horizontal
/// window, search highlights, cursor tint and cut indicators.
pub(super) fn row_spans(p: &Pager, li: usize) -> Vec<Span> {
    let line = &p.lines[li];
    let width = p.cols.max(1);
    let scroll = super::scrollable(line, width);
    let off = if scroll { p.hoff } else { 0 };
    let base = match fold_summary(p, li) {
        Some(spans) => spans,
        None => line.spans.clone(),
    };
    let full_width = base.iter().map(Span::width).sum::<usize>();
    let mut cells = Cells::from_spans(&slice_spans(&base, off, width));
    focus_link(p, &mut cells, li, off);
    highlight(p, &mut cells, li, off);
    if p.cursor_line() == Some(li) || selected(p, li) {
        cells.tint(theme::selection());
    }
    if scroll {
        if off > 0 {
            cells.set(0, CUT_LEFT, theme::muted());
        }
        if full_width > off + width {
            cells.set(width.saturating_sub(1), CUT_RIGHT, theme::muted());
        }
    }
    cells.into_spans()
}

/// True when the row is inside the visual selection (`v`).
fn selected(p: &Pager, li: usize) -> bool {
    let sel = match p.select {
        Some(s) => s,
        None => return false,
    };
    let (lo, hi) = sel.range();
    p.visible
        .get(lo..=hi.min(p.visible.len().saturating_sub(1)))
        .map(|rows| rows.contains(&li))
        .unwrap_or(false)
}

/// A folded heading shows `\u{25b8} Title  (N lines)`.
fn fold_summary(p: &Pager, li: usize) -> Option<Vec<Span>> {
    p.lines[li].heading.as_ref()?;
    let hidden = p.counts.iter().find(|(h, _)| *h == li).map(|(_, n)| *n)?;
    let mut spans = p.lines[li].spans.clone();
    if let Some(first) = spans.first_mut() {
        if first.text.starts_with(theme::MARKER_OPEN) {
            first.text = format!("{} ", theme::MARKER_CLOSED);
        }
    }
    let unit = if hidden == 1 { "line" } else { "lines" };
    spans.push(Span::new(format!("  ({hidden} {unit})"), theme::muted()));
    Some(spans)
}

/// The focused link is drawn reversed, so it reads differently from the other
/// (blue, underlined) links sharing the row.
fn link_focus_style() -> Style {
    Style::new().fg(theme::LINK).bold().reverse()
}

/// Tint the focused link on this row, if it is here.
fn focus_link(p: &Pager, cells: &mut Cells, li: usize, off: usize) {
    let site = match p.focused_link() {
        Some(s) if s.line == li => s,
        _ => return,
    };
    let width = link_width(p, li, &site.url, site.col);
    if width == 0 {
        return;
    }
    let start = site.col.saturating_sub(off);
    cells.restyle(start, start + width, link_focus_style());
}

/// Display width of the run of spans carrying `url`, starting at column `col`.
fn link_width(p: &Pager, li: usize, url: &str, col: usize) -> usize {
    let mut x = 0;
    let mut w = 0;
    for s in &p.lines[li].spans {
        if x >= col && s.link.as_deref() == Some(url) {
            w += s.width();
        } else if w > 0 {
            break;
        }
        x += s.width();
    }
    w
}

fn highlight(p: &Pager, cells: &mut Cells, li: usize, off: usize) {
    if p.query.is_empty() {
        return;
    }
    let current = p.current.and_then(|i| p.matches.get(i)).copied();
    for (start, end) in search::on_line(&p.matches, li) {
        let is_current = current
            .map(|c| c.line == li && c.start == start)
            .unwrap_or(false);
        let style = if is_current { theme::search_current() } else { theme::search() };
        cells.restyle(start.saturating_sub(off), end.saturating_sub(off), style);
    }
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn outline_rows(p: &Pager) -> Vec<(String, Style)> {
    p.outline
        .iter()
        .map(|h| {
            let indent = "  ".repeat((h.level as usize).saturating_sub(1));
            let folded = p.collapsed.contains(&h.id);
            let marker = if folded { theme::MARKER_CLOSED } else { theme::MARKER_OPEN };
            (
                format!("{indent}{marker} {}", h.text),
                theme::heading(h.level),
            )
        })
        .collect()
}

/// `Index 106 docs` plus the live `/` filter, when one is being typed.
fn index_title(p: &Pager) -> String {
    let total = p.nav.as_ref().map(|n| n.entries().len()).unwrap_or(0);
    let shown = p.index_rows().len();
    let mut t = match shown == total {
        true => format!("Index \u{b7} {total} docs"),
        false => format!("Index \u{b7} {shown}/{total} docs"),
    };
    match p.index_typing || !p.index_filter.is_empty() {
        true => {
            t.push_str(&format!("  /{}", p.index_filter));
            if p.index_typing {
                t.push('\u{2588}');
            }
        }
        false => t.push_str("  (/ filters)"),
    }
    t
}

/// One row per linked document: section, title, trailing description.
fn index_rows(p: &Pager) -> Vec<(String, Style)> {
    let nav = match &p.nav {
        Some(n) => n,
        None => return Vec::new(),
    };
    p.index_rows()
        .into_iter()
        .filter_map(|i| nav.entries().get(i))
        .map(|e| {
            let here = nav.is_current(&e.path);
            let style = if here { theme::heading(3) } else { theme::text() };
            (e.row(), style)
        })
        .collect()
}

fn help_rows() -> Vec<(String, Style)> {
    let w = keys::help_key_width();
    keys::help_rows()
        .into_iter()
        .map(|(k, d)| {
            let pad = w.saturating_sub(str_width(k));
            (format!("{k}{}  {d}", " ".repeat(pad)), theme::text())
        })
        .collect()
}

/// A full-body list overlay with a title row and a scroll window on `sel`.
fn overlay(p: &Pager, frame: &mut Frame, title: &str, rows: &[(String, Style)], sel: usize) {
    let body = p.body_rows();
    let width = p.cols.max(1);
    if body == 0 {
        return;
    }
    frame.move_to(1, 1);
    let head = format!(" {title} \u{2014} j/k move, Enter jump, Esc close");
    frame.span(theme::status(), truncate_width(&head, width));
    frame.reset_style();
    frame.clear_to_eol();
    let list_rows = body.saturating_sub(1);
    let top = window_top(sel, rows.len(), list_rows);
    for r in 0..list_rows {
        frame.move_to(r as u16 + 2, 1);
        if let Some((text, style)) = rows.get(top + r) {
            let chosen = top + r == sel;
            let style = if chosen { style.reverse() } else { *style };
            frame.span(style, truncate_width(text, width));
        }
        frame.reset_style();
        frame.clear_to_eol();
    }
}

/// Scroll a list so `sel` stays inside a window of `rows` entries.
pub(crate) fn window_top(sel: usize, len: usize, rows: usize) -> usize {
    if rows == 0 || len <= rows {
        return 0;
    }
    let half = rows / 2;
    sel.saturating_sub(half).min(len - rows)
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

/// `file.md  ·  42%  ·  line 120/840` plus transient messages and the search
/// prompt (SPEC.md §Status bar).
pub(crate) fn status_text(p: &Pager) -> String {
    if let Mode::Search(dir) = p.mode {
        let lead = if dir == search::Dir::Forward { '/' } else { '?' };
        return format!("{lead}{}", p.query);
    }
    if let Some(m) = &p.message {
        return m.clone();
    }
    // Visual mode owns the bar: `-- VISUAL --  N lines selected`.
    if let Some(s) = &p.select {
        return s.status();
    }
    let total = p.visible.len();
    let cur = if total == 0 { 0 } else { p.cursor + 1 };
    let pct = if total <= 1 { 100 } else { p.cursor * 100 / (total - 1) };
    let mut s = format!("{}  \u{b7}  {pct}%  \u{b7}  line {cur}/{total}", p.label);
    let depth = p.nav.as_ref().map(|n| n.depth()).unwrap_or(0);
    if depth > 0 {
        s.push_str(&format!("  \u{b7}  [{depth} back]"));
    }
    // The focused link's target: resolved path for internal, raw URL for
    // external (SPEC.md §Status bar).
    if let Some(target) = p.link_status() {
        s.push_str("  \u{b7}  ");
        s.push_str(&target);
    }
    s
}

fn status(p: &Pager, frame: &mut Frame) {
    let width = p.cols.max(1);
    let text = status_text(p);
    let shown = truncate_width(&text, width);
    let pad = width.saturating_sub(str_width(shown));
    frame.span(theme::status(), shown);
    frame.span(theme::status(), &" ".repeat(pad));
    frame.reset_style();
}

// ---------------------------------------------------------------------------
// Span writing
// ---------------------------------------------------------------------------

fn paint_spans(frame: &mut Frame, spans: &[Span]) {
    let mut open: Option<String> = None;
    for span in spans {
        if open.as_deref() != span.link.as_deref() {
            if open.is_some() {
                frame.link_close();
            }
            if let Some(url) = &span.link {
                frame.link_open(url);
            }
            open = span.link.clone();
        }
        frame.span(span.style, &span.text);
    }
    if open.is_some() {
        frame.link_close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keeps_the_selection_inside() {
        assert_eq!(window_top(0, 10, 5), 0);
        assert_eq!(window_top(2, 10, 5), 0);
        assert_eq!(window_top(5, 10, 5), 3);
        assert_eq!(window_top(9, 10, 5), 5);
        assert_eq!(window_top(3, 3, 5), 0);
        assert_eq!(window_top(3, 10, 0), 0);
    }

    #[test]
    fn help_overlay_lists_every_binding() {
        let rows = help_rows();
        assert_eq!(rows.len(), keys::BINDINGS.len());
        assert!(rows.iter().any(|(t, _)| t.contains("quit")));
        assert!(rows.iter().any(|(t, _)| t.contains("collapse every section")));
    }
}
