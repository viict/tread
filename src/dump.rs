//! Non-interactive rendering: stream a laid-out document straight to a writer
//! instead of driving the pager.
//!
//! This is the path used by `--no-alt` on a non-terminal stdout, by
//! `cat x.md | tread > file`, and by the golden render tests. It shares the
//! layout engine and the `Frame` writer with the interactive pager, so what
//! you see here is exactly what a frame would contain (SPEC.md §7: all
//! terminal output goes through the frame buffer).
#![deny(unsafe_code)]

use std::fmt::{self, Write};

use crate::render::{slice_spans, Line, Span};
use crate::source::Source;
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

/// Lay out and paint a whole document into `out`, whatever format it is in.
/// `plain` strips all styling; `clip` is the viewport width for horizontally
/// scrollable rows (wide tables, code, a CSV grid): pass `true` when writing to
/// a terminal, `false` when writing to a file or pipe, where full-fidelity rows
/// are more useful than a viewport.
///
/// Nothing whole-document happens here. Rows are laid out a window at a time
/// and each window is written out before the next is asked for, and a lazily
/// indexed format is only pushed as far as the rows already written — so
/// `tread huge.csv | head` prints its first rows immediately and dumping a file
/// too big to hold costs a window of memory rather than a file's
/// (SPEC.md §CSV: nothing may read the whole file on the open path).
/// Painting is per-line and the frame resets its style at every line ending, so
/// the bytes are identical to painting the whole document in one go.
///
/// A write error — the closed pipe `head` leaves behind — stops the walk, which
/// is what keeps that pipeline from indexing the rest of the file for nobody.
pub fn write_source(
    src: &mut dyn Source,
    width: usize,
    plain: bool,
    clip: bool,
    out: &mut dyn Write,
) -> fmt::Result {
    let clip = clip.then_some(width);
    src.set_width(width);
    // A dump is not a viewport: nothing here can be opened, so anything left
    // folded is simply missing from the output. Metadata starts folded for a
    // reader, which would silently drop it from `tread doc.md > out.txt`.
    src.fold_all(false);
    // Blank rows at the end of a window are held back rather than trimmed:
    // only the ones at the end of the *document* are dropped.
    let mut pending = 0usize;
    let mut at = 0usize;
    loop {
        let end = src.len();
        if at >= end {
            // `len` grows as a lazily indexed format discovers more of its
            // file. One bounded slice, then look again: draining the index
            // first would make the first row wait for the last.
            //
            // The slice that finishes the file both *finds* the last rows and
            // reports that there is no more work, so "no more work" cannot end
            // the dump on its own — only "no more work and no new rows" can.
            // Getting this wrong truncates the output silently: a document
            // whose last rows sit past one budget printed the rows before them
            // and exited 0.
            let more = src.extend();
            if !more && src.len() <= end {
                return Ok(());
            }
            continue;
        }
        let window = src.lines(at..end.min(at + WINDOW_ROWS));
        if window.is_empty() {
            return Ok(());
        }
        at += window.len();
        let kept = window
            .iter()
            .rposition(|l| !l.is_blank())
            .map(|i| i + 1)
            .unwrap_or(0);
        if kept > 0 {
            for _ in 0..std::mem::take(&mut pending) {
                out.write_char('\n')?;
            }
            out.write_str(&paint(&window[..kept], plain, clip))?;
        }
        pending += window.len() - kept;
    }
}

/// [`write_source`] into a `String`, for the tests and the callers that want
/// the whole document in hand.
#[cfg(test)]
pub fn render_source(src: &mut dyn Source, width: usize, plain: bool, clip: bool) -> String {
    let mut out = String::new();
    let _ = write_source(src, width, plain, clip, &mut out);
    out
}

/// Rows [`render_source`] lays out per pass.
const WINDOW_ROWS: usize = 1024;

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
    use crate::md::{self, Document};
    use crate::source::markdown::MarkdownSource;

    /// The markdown dump path, as `main` drives it: parse, wrap in a source,
    /// render every row.
    fn dump(doc: &Document, width: usize, plain: bool, clip: bool) -> String {
        let mut src = MarkdownSource::new(doc.clone());
        render_source(&mut src, width, plain, clip)
    }

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

    /// A window boundary is not a document boundary: blank rows held back at
    /// the end of one window are re-emitted exactly once when the next window
    /// has content, never once per window from there on.
    #[test]
    fn streaming_a_document_longer_than_a_window_matches_painting_it_whole() {
        let mut doc = String::new();
        for i in 0..3000 {
            doc.push_str(&format!("para {i}\n\n"));
        }
        let mut src = MarkdownSource::new(md::parse(&doc));
        let streamed = render_source(&mut src, 60, true, false);
        let n = crate::source::Source::len(&src);
        let all = crate::source::Source::lines(&mut src, 0..n);
        assert_eq!(streamed, paint(&all, true, None));
        assert!(streamed.lines().count() > 2 * WINDOW_ROWS);
    }

    /// The last rows of a lazily indexed document must reach the output.
    ///
    /// A JSON member bigger than one scan budget takes several `extend` calls
    /// to step over. The call that finally steps over it both finds the rows
    /// after it and reports that there is no work left — so a dump that stopped
    /// on "no work left" alone printed the rows *before* the big member, the
    /// closing bracket and everything after it silently missing, and exited 0.
    #[test]
    fn a_member_bigger_than_one_scan_budget_does_not_truncate_the_dump() {
        use crate::source::json::JsonSource;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(br#"{"small":1,"blob":""#);
        bytes.extend(std::iter::repeat(b'x').take(6 * 1024 * 1024));
        bytes.extend_from_slice(br#"","tail":2}"#);
        let mut src = JsonSource::from_bytes(bytes);
        let out = render_source(&mut src, 100, true, false);
        let rows: Vec<&str> = out.lines().collect();
        assert_eq!(rows.len(), 5, "every row reaches the output: {rows:#?}");
        assert!(rows[1].contains("\"small\": 1"));
        assert!(rows[2].contains("over the"), "the big member says why: {:?}", rows[2]);
        assert!(rows[3].contains("\"tail\": 2"), "the row after it survives");
        assert_eq!(rows[4].trim(), "}", "and so does the closing bracket");
    }

    /// The dump path must not index a lazily discovered file before it writes
    /// its first row: `tread huge.csv | head` prints at once, and the closed
    /// pipe `head` leaves behind stops the walk instead of scanning the rest of
    /// the file for nobody (SPEC.md §CSV).
    #[test]
    fn a_lazy_source_streams_and_stops_when_the_pipe_closes() {
        use crate::source::csv::CsvSource;

        let mut body = String::from("id,name\n");
        for i in 0..200_000 {
            body.push_str(&format!("{i},name {i}\n"));
        }
        let mut src = CsvSource::from_bytes(body.into_bytes(), None);
        let mut sink = Stops { out: String::new(), left: 1 };
        assert!(write_source(&mut src, 60, true, false, &mut sink).is_err());
        assert!(sink.out.contains("name 0"), "no row was written");
        assert!(
            crate::source::Source::len(&src) < 200_000,
            "the whole file was indexed before the first write"
        );
    }

    /// A writer that fails after `left` writes, as a closed pipe does.
    struct Stops {
        out: String,
        left: usize,
    }

    impl Write for Stops {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if self.left == 0 {
                return Err(fmt::Error);
            }
            self.left -= 1;
            self.out.push_str(s);
            Ok(())
        }
    }

    #[test]
    fn no_row_is_carriage_returned() {
        let doc = md::parse("# T\n\npara\n");
        assert!(!dump(&doc, 40, true, false).contains('\r'));
    }
}

