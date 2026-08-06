//! Non-interactive rendering: paint a laid-out document straight to a byte
//! buffer instead of driving the pager.
//!
//! This is the path used by `--no-alt` on a non-terminal stdout, by
//! `cat x.md | mdr > file`, and by the golden render tests. It shares the
//! layout engine and the `Frame` writer with the interactive pager, so what
//! you see here is exactly what a frame would contain (SPEC.md §7: all
//! terminal output goes through the frame buffer).
#![deny(unsafe_code)]

use crate::md::Document;
use crate::render::{render_document, slice_spans, Line, RenderOpts, Span};
use crate::term::Frame;
use crate::theme;

/// Default wrap width when neither `--width` nor a terminal size is available.
pub const DEFAULT_WIDTH: usize = 80;
/// Terminals wider than this get a reading measure rather than full-bleed text.
pub const MAX_AUTO_WIDTH: usize = 120;

/// Pick the layout width: explicit `--width` wins, then the detected terminal
/// width (capped for readability), then [`DEFAULT_WIDTH`].
pub fn layout_width(explicit: Option<usize>, detected: Option<usize>) -> usize {
    match explicit {
        Some(w) => w.max(crate::render::MIN_WIDTH),
        None => detected
            .filter(|w| *w >= crate::render::MIN_WIDTH)
            .map(|w| w.min(MAX_AUTO_WIDTH))
            .unwrap_or(DEFAULT_WIDTH),
    }
}

/// Lay out and paint a document. `plain` strips all styling. `clip` is the
/// viewport width for horizontally scrollable rows (wide tables, code): pass
/// `Some(width)` when writing to a terminal, `None` when writing to a file or
/// pipe, where full-fidelity rows are more useful than a viewport.
pub fn dump(doc: &Document, width: usize, plain: bool, clip: bool) -> String {
    let opts = RenderOpts::new(width);
    let lines = render_document(doc, &opts);
    paint(&lines, plain, if clip { Some(width) } else { None })
}

/// Paint pre-laid-out lines. Trailing blank rows are dropped so piped output
/// does not end in a run of empty lines.
pub fn paint(lines: &[Line], plain: bool, clip: Option<usize>) -> String {
    let mut frame = Frame::new(plain);
    let end = lines
        .iter()
        .rposition(|l| !l.is_blank())
        .map(|i| i + 1)
        .unwrap_or(0);
    for line in &lines[..end] {
        match clip.filter(|w| line.width() > *w) {
            Some(w) => paint_spans(&mut frame, &clipped(line, w)),
            None => paint_spans(&mut frame, &line.spans),
        }
    }
    // The frame writer targets raw mode, where a bare newline does not return
    // the carriage. Cooked stdout wants plain LF.
    frame.as_str().replace("\r\n", "\n")
}

/// Clip a row that overflows the viewport, marking the cut with `\u{203a}` so
/// the reader can see content continues to the right.
fn clipped(line: &Line, width: usize) -> Vec<Span> {
    let keep = width.saturating_sub(1);
    let mut spans = slice_spans(&line.spans, 0, keep);
    spans.push(Span::new("\u{203a}", theme::muted()));
    spans
}

fn paint_spans(frame: &mut Frame, spans: &[Span]) {
    let mut open_link: Option<&str> = None;
    let last = spans.len().saturating_sub(1);
    for (i, span) in spans.iter().enumerate() {
        match (open_link, span.link.as_deref()) {
            (Some(a), Some(b)) if a == b => {}
            (prev, next) => {
                if prev.is_some() {
                    frame.link_close();
                }
                if let Some(url) = next {
                    frame.link_open(url);
                }
                open_link = next;
            }
        }
        let text = trim_trailing(&span.text, i == last && span.style.bg.is_none());
        frame.span(span.style, text);
    }
    if open_link.is_some() {
        frame.link_close();
    }
    frame.end_line();
}

/// Strip trailing padding from the last span of a row when it carries no
/// background: untinted trailing whitespace is invisible on screen and only
/// clutters a redirected file. Tinted padding (code blocks) is load-bearing —
/// it is what makes the block look like a block — so it stays.
fn trim_trailing(text: &str, trim: bool) -> &str {
    if trim {
        text.trim_end_matches(' ')
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md;

    fn strip(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC ... ST (ESC \)
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn width_precedence() {
        assert_eq!(layout_width(Some(40), Some(200)), 40);
        assert_eq!(layout_width(None, Some(200)), MAX_AUTO_WIDTH);
        assert_eq!(layout_width(None, Some(64)), 64);
        assert_eq!(layout_width(None, None), DEFAULT_WIDTH);
        assert_eq!(layout_width(None, Some(0)), DEFAULT_WIDTH);
        assert_eq!(layout_width(Some(1), None), crate::render::MIN_WIDTH);
    }

    #[test]
    fn plain_dump_has_no_escapes() {
        let doc = md::parse("## Title\n\nSome *text* with [a link](x.md).\n");
        let out = dump(&doc, 60, true, false);
        assert!(!out.contains('\x1b'), "escape leaked: {out:?}");
        assert!(out.contains("Title"));
        assert!(out.contains("a link"));
        assert!(!out.ends_with("\n\n"));
    }

    #[test]
    fn styled_dump_strips_back_to_the_plain_dump() {
        let src = "# Hi\n\n| a | b |\n| --- | ---: |\n| 1 | 2 |\n\n- one\n- two\n";
        let doc = md::parse(src);
        let styled = dump(&doc, 50, false, false);
        let plain = dump(&doc, 50, true, false);
        assert!(styled.contains('\x1b'));
        assert_eq!(strip(&styled), plain);
    }

    #[test]
    fn every_styled_row_ends_reset() {
        let doc = md::parse("## H\n\ntext\n");
        for row in dump(&doc, 40, false, false).lines().filter(|r| r.contains('\x1b')) {
            assert!(row.ends_with("\x1b[0m"), "row not reset: {row:?}");
        }
    }

    #[test]
    fn links_emit_one_osc8_pair() {
        let doc = md::parse("See [docs](models/SAMPLE_MODEL.md) now.\n");
        let out = dump(&doc, 60, false, false);
        assert_eq!(out.matches("\x1b]8;;").count(), 2);
        assert!(out.contains("\x1b]8;;models/SAMPLE_MODEL.md\x1b\\"));
    }

    #[test]
    fn no_mouse_tracking_is_ever_emitted() {
        let doc = md::parse("# T\n\n```rs\nfn x() {}\n```\n\n> quote\n");
        let out = dump(&doc, 60, false, false);
        for bad in ["?1000", "?1002", "?1003", "?1006", "?1015"] {
            assert!(!out.contains(bad), "mouse sequence {bad} leaked");
        }
    }

    #[test]
    fn clipping_bounds_every_row_and_marks_the_cut() {
        let src = "| a | b |\n| --- | --- |\n| a very long cell indeed | another long one |\n";
        let doc = md::parse(src);
        let out = dump(&doc, 30, true, true);
        for row in out.lines() {
            assert!(crate::render::str_width(row) <= 30, "row too wide: {row:?}");
        }
        assert!(out.contains('\u{203a}'));
        // Unclipped, the same table keeps its full width.
        assert!(dump(&doc, 30, true, false)
            .lines()
            .any(|r| crate::render::str_width(r) > 30));
    }

    #[test]
    fn code_block_tint_keeps_its_padding() {
        let doc = md::parse("```\nx\n```\n");
        let row = dump(&doc, 40, false, false)
            .lines()
            .find(|r| r.contains('x'))
            .expect("code row")
            .to_string();
        // The tinted run pads out to the block width instead of being trimmed.
        assert!(row.contains("48;5;236"));
        assert!(row.ends_with("  \u{1b}[0m"), "padding trimmed: {row:?}");
    }

    #[test]
    fn no_row_is_carriage_returned() {
        let doc = md::parse("# T\n\npara\n");
        assert!(!dump(&doc, 40, true, false).contains('\r'));
    }
}
