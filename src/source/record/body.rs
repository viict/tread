//! The message under a summary row: how tall it is, and what it paints.
//!
//! A lens row is a headline — `who · when · what`, one line, never wrapped —
//! and for a record that said something **the headline is the message's own
//! first line**. What follows it goes *under* it, wrapped to the view width and
//! indented to the `what` column, so a conversation reads as a conversation
//! rather than as a list of first lines.
//!
//! ```text
//! ▾ assistant  10:55  Reading the failing test first. The suite names a
//!                     fixture that no longer exists, so that is where
//!                     this starts.
//!                     ⋯ +37 lines
//! ```
//!
//! One wrap, split in two. [`first_line`] is row 1 — the summary row's `what`,
//! never wrapped because it is already one line's worth — and [`rows`] paints
//! rows 2..N. The message's opening words therefore appear once, which they did
//! not when the row painted [`crate::lens::Summary::what`] and the body then
//! started again from the top. `what` itself is unchanged: `--toc` still prints
//! the one-line excerpt, which is the right answer for a list.
//!
//! # The two states, and why a clip may never hide
//!
//! A body is **clipped** to [`CLIP`] rows until the reader opens it
//! (`Enter` / `za`), and the last row then says what is not on screen — in the
//! message's own lines when it has more of them, in bytes when the remainder is
//! the tail of one long line. Opening it paints the whole message, and the raw
//! record is still one `zt` away either way. A lens never hides anything
//! (SPEC.md §Lenses), so a clip that did not say what it left out would be the
//! one thing this seam is not allowed to do.
//!
//! # Its own wrapper
//!
//! `render::wrap` wraps *atoms* — styled runs, links, clusters that may not be
//! split — because it lays out inline markdown. A message here is plain text
//! that happens to contain newlines, and the only decisions are the word break
//! and the hard split of a word longer than the column. That is small enough to
//! state directly, and doing so keeps the markdown layout engine free to make
//! markdown's decisions.
//!
//! Nothing here reads a file, parses a record or knows a dialect: it is text,
//! a width and a flag in, rows out.
#![deny(unsafe_code)]

use crate::lens::Body;
use crate::render::{str_width, take_width, visible, Line, LineKind, Span};
use crate::theme;

/// Rows a clipped message occupies in total — the summary row it starts on,
/// and the body rows under it — before it says how much more there is.
pub const CLIP: usize = 6;

/// Columns before the message text: the gutter, the actor column, the clock,
/// and the two spaces before `what` — so a body sits exactly under the words
/// it belongs to.
pub const INDENT: usize = 21;

/// Columns of message text at a given view width. Never zero: a terminal
/// narrower than the indent still gets a body rather than an empty column.
fn columns(width: usize) -> usize {
    width.saturating_sub(INDENT).max(8)
}

/// Rows the body occupies **under** the summary row, at `width`, clipped or
/// whole. Zero for a message the summary row already shows all of.
///
/// The one number the row arithmetic runs on ([`super::plan::Plan`]), which is
/// why it is derived from the same walk that paints — the same `lay`, the same
/// `skip(1)`: a height that disagreed with the painted rows by one would move
/// every row below it.
pub fn height(body: &Body, text: &str, width: usize, full: bool) -> usize {
    let laid = lay(body, text, width, full);
    under(&laid) + usize::from(laid.note.is_some())
}

/// The body's rows, ready to paint: the wrap from its **second** line on,
/// because the first is the summary row above them.
pub fn rows(body: &Body, text: &str, width: usize, full: bool, source_line: usize) -> Vec<Line> {
    let laid = lay(body, text, width, full);
    let pad = " ".repeat(INDENT);
    let mut out: Vec<Line> = laid
        .rows
        .iter()
        .skip(1)
        .map(|r| row(&pad, r, theme::lens_body(), source_line))
        .collect();
    if let Some(note) = laid.note {
        out.push(row(&pad, &note, theme::lens_more(), source_line));
    }
    out
}

/// The message's first wrapped line: what the **summary row** paints in its
/// `what` column, at the width the body is laid out for.
///
/// `None` only where there is nothing to wrap at all — an empty message, or one
/// written entirely in blank lines. The row is never wrapped itself: this is
/// already one row's worth of text, and it scrolls sideways like every other
/// summary row.
///
/// `text` must be the text [`rows`] is given for the same record, or the two
/// wraps are of different strings and the split between them loses whatever
/// falls between their first rows.
pub fn first_line(body: &Body, text: &str, width: usize) -> Option<String> {
    lay(body, text, width, false).rows.into_iter().next()
}

/// Rows of the wrap that fall under the summary row rather than on it.
fn under(laid: &Laid) -> usize {
    laid.rows.len().saturating_sub(1)
}

/// Does the clip leave anything out at this width?
///
/// What `Enter` / `za` opens — and, when this is false, what that key must
/// *not* consume: a message that is entirely on screen has one state rather
/// than two, and the row's fold marker is the record's own tree.
pub fn clips(body: &Body, text: &str, width: usize) -> bool {
    lay(body, text, width, false).note.is_some()
}

/// One painted row of a body. Not a heading and not foldable: the summary row
/// above it owns the fold, and `Tab` walks items rather than lines.
fn row(pad: &str, text: &str, style: crate::term::Style, source_line: usize) -> Line {
    // A blank line in a message is a blank row, not twenty-one spaces: this
    // goes down a pipe as often as it goes on a screen.
    let spans = match text.is_empty() {
        true => Vec::new(),
        false => vec![Span::plain(pad), Span::new(text.to_string(), style)],
    };
    Line {
        spans,
        block: 0,
        source_line,
        heading: None,
        scroll: true,
        kind: LineKind::Paragraph,
    }
}

/// A laid-out body: the rows it shows, and what it admits to leaving out.
struct Laid {
    rows: Vec<String>,
    note: Option<String>,
}

/// How far the walk got through the text it was given, in the text's *own*
/// bytes — never in painted ones.
///
/// Painted bytes cannot answer for a remainder: `visible` substitutes a
/// two-byte `·` for a one-byte control character, so a row of tabs paints wider
/// than it reads and a remainder computed from it goes negative — which, being
/// a `saturating_sub`, came out as "nothing left to show" over a message that
/// had been cut. Characters survive that substitution one for one, and this
/// counts them off against the source.
struct Walk {
    /// Whole source lines the walk got through — painted, or blank and walked
    /// over. Never a line a head cut off in the middle.
    lines: usize,
    /// Source bytes those rows account for. Exact while every line was painted
    /// whole — the walk jumps to the next line's offset — and a floor inside a
    /// line the clip cut, because the wrap drops the space it broke on. A floor
    /// is the safe direction: it can only make the remainder look bigger.
    bytes: usize,
}

/// Wrap `text` — as much of the message as the caller has — to the width, stop
/// where the state says to stop, and state the remainder.
fn lay(body: &Body, text: &str, width: usize, full: bool) -> Laid {
    let cols = columns(width);
    let limit = match full {
        true => usize::MAX,
        false => CLIP,
    };
    // A clip never walks more of the message than it could possibly show: at
    // four bytes to a column this is past any width's worth of [`CLIP`] rows,
    // and it is what keeps painting a clipped 40 MB message a screenful of
    // work. The height comes off this same walk, so the two cannot disagree.
    let text = match full {
        true => text,
        false => head_of(text, 4 * (limit + 1).saturating_mul(cols)),
    };
    // The caller may only have the head of the message.
    let short = text.len() < body.bytes;
    let (rows, walk) = walk_text(text, cols, limit, short);
    Laid { note: note(body, &walk, text.len(), short), rows }
}

/// Wrap every line of `text` to `cols`, stopping at `limit` rows, and count off
/// against the source what those rows accounted for.
///
/// The one walk: [`rows`] paints what this returns and [`height`] counts it, so
/// anything done to the row list here is done to the height by construction.
fn walk_text(text: &str, cols: usize, limit: usize, short: bool) -> (Vec<String>, Walk) {
    let mut rows: Vec<String> = Vec::new();
    let mut walk = Walk { lines: 0, bytes: 0 };
    let mut at = 0usize;
    for line in text.split('\n') {
        if rows.len() >= limit {
            break;
        }
        // Where the line after this one starts — the newline included, so a
        // message that ends in one is fully accounted for and says nothing
        // more. `split` yields a last empty segment for it, and counting that
        // segment's absence as missing content was a `⋯` row over a message
        // every byte of which was on screen.
        let next = (at + line.len() + 1).min(text.len());
        // The head cut its last line where the head ended, not where the line
        // does, so that segment is not a line anyone has seen in full however
        // many rows it took — and it is the *only* line the -1 ever applied to.
        // Subtracting one whenever the caller held a head made the six-row clip
        // of a long message claim one more hidden line than it had.
        let cut = short && at + line.len() >= text.len();
        // A message that opens with blank lines has no headline in them: the
        // summary row would paint an empty `what` column and the clip would
        // spend its rows on nothing. They are walked over, not painted — they
        // carry no content to hide, and the walk still counts them off.
        if rows.is_empty() && line.trim().is_empty() {
            walk.lines += usize::from(!cut);
            walk.bytes = next;
            at = next;
            continue;
        }
        let wrapped = fold_line(&visible(line), cols);
        let room = limit - rows.len();
        let whole_line = wrapped.len() <= room;
        let mut chars = 0usize;
        for r in wrapped.into_iter().take(room) {
            chars += r.chars().count();
            rows.push(r);
        }
        if !whole_line {
            walk.bytes = at + prefix_bytes(line, chars);
            break;
        }
        walk.lines += usize::from(!cut);
        walk.bytes = next;
        at = next;
    }
    (rows, walk)
}

/// What the body is not showing: its own lines when whole ones are missing, and
/// bytes when what is left is the tail of a line already begun. `None` when
/// every byte of the message is on screen.
fn note(body: &Body, walk: &Walk, seen: usize, short: bool) -> Option<String> {
    // Whether anything is missing is structural: the walk stopped short of the
    // text it was given, or that text was only the head. A byte count decides
    // *what* the row says, never whether there is one.
    if walk.bytes >= seen && !short {
        return None;
    }
    // Lines the walk saw whole — never the one a head cut short, which the walk
    // itself declines to count ([`walk_text`]).
    let lines = body.lines.saturating_sub(walk.lines);
    // A message written as one long line has no lines to count off, and
    // "+1 line" would say nothing about the ten kilobytes left of it.
    if lines == 0 || body.lines == 1 {
        // Source bytes against source bytes, and never silent: a clip that hid
        // something and said nothing is the one thing this seam may not do
        // (SPEC.md §Lenses).
        let left = body.bytes.saturating_sub(walk.bytes).max(1);
        return Some(format!("\u{22ef} +{} more", crate::source::jsonrow::size(left as u64)));
    }
    Some(match lines {
        1 => "\u{22ef} +1 line".to_string(),
        n => format!("\u{22ef} +{n} lines"),
    })
}

/// Bytes the first `chars` characters of `line` occupy.
fn prefix_bytes(line: &str, chars: usize) -> usize {
    line.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(line.len())
}

/// The first `bytes` of `text`, cut on a character boundary — a message may be
/// any UTF-8 at all, and cutting mid-character would panic.
fn head_of(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut cut = bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

/// One source line, wrapped to `cols` columns on word boundaries, splitting a
/// word that cannot fit on a line of its own. An empty line stays a row: a
/// blank line in a message is a paragraph break, and dropping it would re-flow
/// what someone wrote.
fn fold_line(line: &str, cols: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut used = 0usize;
    for word in line.split(' ') {
        let w = str_width(word);
        if used > 0 && used + 1 + w > cols {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        if used > 0 {
            cur.push(' ');
            used += 1;
        }
        if w <= cols {
            cur.push_str(word);
            used += w;
            continue;
        }
        // A word wider than the column: split it, because the alternative is a
        // row that runs off the screen and takes the rest of the message with
        // it. Every piece is whole characters — `take_width` never cuts one.
        let mut rest = word;
        while !rest.is_empty() {
            let (piece, wide) = take_width(rest, cols - used);
            if piece.is_empty() {
                out.push(std::mem::take(&mut cur));
                used = 0;
                continue;
            }
            cur.push_str(piece);
            used += wide;
            rest = &rest[piece.len()..];
            if !rest.is_empty() {
                out.push(std::mem::take(&mut cur));
                used = 0;
            }
        }
    }
    out.push(cur);
    out
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
