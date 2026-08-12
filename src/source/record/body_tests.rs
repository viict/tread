//! The message under a row: its wrap, its clip, and what the clip admits to.
//!
//! Fixtures are written here by hand. The one rule every case below is really
//! testing is the non-negotiable one: what is not on screen is *stated*, and
//! the whole message is one keypress away.
//!
//! The wrap is *split*: row 1 is the summary row's `what` ([`first_line`]) and
//! rows 2..N are the body ([`rows`]). Every count below is therefore a count of
//! what is under the summary row, and the head of the message is asserted
//! against `first_line` rather than against the first body row.
use super::*;
use crate::lens::{Body, Step};

/// A body whose text is short enough to be kept whole.
fn body(text: &str) -> Body {
    Body::new(text, vec![Step::Key("message")])
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(|l| l.text().trim_end().to_string()).collect()
}

/// What the summary row paints, and what the body paints under it.
fn split(b: &Body, width: usize, full: bool) -> (String, Vec<String>) {
    let head = first_line(b, b.text_in(None), width).unwrap_or_default();
    (head, texts(&rows(b, b.text_in(None), width, full, 1)))
}

#[test]
fn one_short_line_is_the_summary_row_and_nothing_under_it() {
    let b = body("hello there");
    let (head, laid) = split(&b, 80, false);
    assert_eq!(head, "hello there");
    assert!(laid.is_empty(), "{laid:#?}");
    assert_eq!(height(&b, &b.head, 80, false), 0);
}

#[test]
fn an_empty_message_has_no_rows_under_its_summary() {
    let b = body("");
    assert_eq!(height(&b, &b.head, 80, false), 0);
    // Nothing to wrap at all, so nothing on the summary row either — the actor
    // and the clock still name the record.
    assert_eq!(first_line(&b, &b.head, 80).unwrap_or_default(), "");
}

/// A message that opens with blank lines still says something on its summary
/// row: the headline is its first line with *content*, not the newline someone
/// happened to start with, and the clip is not spent on empty rows.
#[test]
fn leading_blank_lines_do_not_swallow_the_headline() {
    let b = body("\n\n\nAFTER_BLANKS the real first line.\nsecond line here.");
    for width in [20usize, 40, 60, 92, 400] {
        let (head, laid) = split(&b, width, false);
        // At eight columns the word itself is split; what matters at every
        // width is that the headline starts where the message says something.
        assert!(head.starts_with("AFTER_BL"), "{width}: {head:?}");
        assert!(!laid.iter().any(|r| r.trim().is_empty()), "{width}: {laid:#?}");
        assert_eq!(height(&b, &b.head, width, false), laid.len(), "{width}");
        // And past the width where this message needs the clip, nothing is
        // claimed to be missing: the blank lines cost the reader no rows.
        if width >= 40 {
            assert!(!laid.iter().any(|r| r.trim().starts_with('\u{22ef}')), "{width}: {laid:#?}");
        }
    }
}

/// `⋯ +N lines` counts what is genuinely not shown. A message longer than the
/// head that the clip stops six rows into has *seen* those six lines; counting
/// the last of them as hidden — which the head's own cut line does deserve —
/// overstated every long message by one.
#[test]
fn the_clip_counts_only_the_lines_it_did_not_finish() {
    let text = (0..20)
        .map(|n| format!("line {n:02} aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jj"))
        .collect::<Vec<_>>()
        .join("\n");
    let b = body(&text);
    assert!(!b.whole(), "the fixture is longer than the head");
    let (head, laid) = split(&b, 92, false);
    assert!(head.starts_with("line 00"), "{head:?}");
    // Lines 0..=5 are painted whole — one on the summary row, five under it —
    // so fourteen of the twenty are not shown.
    assert_eq!(laid[CLIP - 2].trim(), "line 05 aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jj");
    assert_eq!(laid[CLIP - 1].trim(), "\u{22ef} +14 lines");
}

#[test]
fn text_wraps_to_the_width_less_the_indent() {
    let b = body("alpha beta gamma delta epsilon");
    // 40 columns leaves 19 for the message: "alpha beta gamma" on the summary
    // row, then the rest under it.
    let (head, laid) = split(&b, 40, false);
    assert_eq!(head, "alpha beta gamma");
    assert_eq!(laid.len(), 1);
    assert_eq!(laid[0].trim(), "delta epsilon");
    assert_eq!(height(&b, &b.head, 40, false), laid.len());
}

/// The defect this split exists to fix: the opening words were on the summary
/// row *and* on the first row under it.
#[test]
fn the_first_line_is_never_painted_twice() {
    let text = "I'll build from the sources only, diagnose any failures from local files, \
                then verify the result.\nsecond line\nthird line";
    let b = body(text);
    for width in [40usize, 92, 200] {
        let (head, laid) = split(&b, width, true);
        assert!(!head.is_empty(), "{width}");
        assert!(!laid.iter().any(|r| r.trim() == head), "{width}: {head:?} repeated in {laid:#?}");
        let painted = std::iter::once(head.clone()).chain(laid.iter().map(|r| r.trim().to_string()));
        let once: Vec<String> = painted.filter(|r| *r == head).collect();
        assert_eq!(once.len(), 1, "{width}: {head:?}");
    }
}

#[test]
fn a_narrower_width_makes_a_taller_body() {
    let b = body("alpha beta gamma delta epsilon zeta eta theta");
    let wide = height(&b, &b.head, 120, true);
    let narrow = height(&b, &b.head, 40, true);
    assert!(narrow > wide, "{narrow} should exceed {wide}");
}

#[test]
fn a_word_longer_than_the_column_is_split_rather_than_lost() {
    let b = body(&"x".repeat(40));
    let (head, laid) = split(&b, INDENT + 10, true);
    assert_eq!(laid.len(), 3);
    let all = format!("{head}{}", laid.concat());
    assert_eq!(all.replace(' ', ""), "x".repeat(40));
}

#[test]
fn blank_lines_survive_because_someone_wrote_them() {
    let b = body("one\n\ntwo");
    // "one" is the summary row; the blank line and "two" are under it.
    let (head, laid) = split(&b, 80, true);
    assert_eq!(head, "one");
    assert_eq!(laid, vec![String::new(), format!("{}two", " ".repeat(INDENT))]);
    assert_eq!(height(&b, &b.head, 80, true), 2);
}

/// Six rows for the message: its summary row, and five under it.
#[test]
fn a_clip_stops_at_six_rows_and_states_the_rest() {
    let text = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let b = body(&text);
    assert_eq!(height(&b, &b.head, 80, false), CLIP);
    let (head, laid) = split(&b, 80, false);
    assert_eq!(head, "line 1");
    assert_eq!(laid.len(), CLIP);
    assert_eq!(laid[CLIP - 2].trim(), "line 6");
    // Six lines are on screen, so fourteen are not — counted from the summary
    // row down, which is where the message now starts.
    assert_eq!(laid[CLIP - 1].trim(), "\u{22ef} +14 lines");
}

#[test]
fn opening_the_body_shows_every_line_and_says_nothing_more() {
    let text = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let b = body(&text);
    let (head, laid) = split(&b, 80, true);
    assert_eq!(head, "line 1");
    assert_eq!(laid.len(), 19);
    assert!(laid.last().unwrap().contains("line 20"));
    assert_eq!(height(&b, &b.head, 80, true), 19);
}

#[test]
fn a_single_long_line_clipped_reports_what_is_left_in_bytes() {
    let b = body(&"word ".repeat(2000));
    let laid = texts(&rows(&b, &b.head, 80, false, 1));
    assert_eq!(laid.len(), CLIP);
    let note = laid[CLIP - 1].trim().to_string();
    assert!(note.starts_with("\u{22ef} +"), "{note}");
    assert!(note.ends_with("more"), "{note}");
}

/// The clip may never be silent, and the arithmetic that says how much is left
/// runs on the message's own bytes. Painted bytes are not the message's:
/// `visible` substitutes a two-byte `·` for a one-byte control character, so a
/// line of tabs paints wider than it reads.
#[test]
fn a_line_of_control_characters_still_says_what_the_clip_cut() {
    let mut text = "x".to_string();
    text.push_str(&"\t".repeat(1200));
    let b = body(&text);
    let laid = texts(&rows(&b, b.text_in(None), 300, false, 1));
    let note = laid.last().unwrap().trim().to_string();
    assert!(note.starts_with("\u{22ef} +"), "the clip cut, so it must say so: {laid:#?}");
    assert!(note.ends_with("more"), "{note}");
}

/// The same arithmetic on a line that *fits* the head: `word\t` repeated is
/// mostly control characters, and the tail after them is what must be counted.
#[test]
fn a_clipped_line_counts_its_tail_in_the_message_s_own_bytes() {
    let mut text = "word\t".repeat(94);
    text.push_str(&"z".repeat(79));
    let b = body(&text);
    assert!(b.whole(), "the fixture fits the head");
    let laid = texts(&rows(&b, &b.head, 100, false, 1));
    let note = laid.last().unwrap().trim().to_string();
    assert!(note.starts_with("\u{22ef} +"), "{laid:#?}");
    let left: usize = note
        .trim_start_matches("\u{22ef} +")
        .trim_end_matches(" bytes more")
        .parse()
        .expect("a byte count");
    // Six rows of message text were painted, and every one of those characters
    // is one byte of the message: what is left is the rest of it, counted where
    // the message lives rather than after `visible` widened it.
    assert_eq!(left, text.len() - CLIP * (100 - INDENT), "{note}");
}

/// A message that ends in a newline is entirely on screen when its lines are.
/// `str::lines` drops that last empty segment and the wrap does not, and the
/// difference used to paint `⋯ +6 bytes more` over a message with nothing left.
#[test]
fn a_trailing_newline_is_not_a_line_that_was_left_out() {
    let b = body("aaa\nbbb\nccc\nddd\neee\nfff\n");
    for width in [40usize, 100, 200] {
        let laid = texts(&rows(&b, &b.head, width, false, 1));
        assert_eq!(laid.len(), 5, "{width}: {laid:#?}");
        assert!(
            !laid.iter().any(|r| r.trim().starts_with('\u{22ef}')),
            "{width}: nothing was left out: {laid:#?}"
        );
    }
}

/// And a message that wraps to exactly the clip says nothing more either.
#[test]
fn a_message_that_fills_the_clip_exactly_says_nothing_more() {
    // Three lines, each wrapping to two rows at 40 columns (19 for the text).
    let line = "alpha beta gamma delta epsilon";
    let b = body(&[line, line, line].join("\n"));
    let laid = texts(&rows(&b, &b.head, 40, false, 1));
    assert_eq!(laid.len(), CLIP - 1, "{laid:#?}");
    assert!(!laid.iter().any(|r| r.trim().starts_with('\u{22ef}')), "{laid:#?}");
}

/// The predicate `Enter` is answered with: two states, or one.
#[test]
fn a_body_that_fits_does_not_claim_to_clip() {
    let short = body("hello there");
    assert!(!clips(&short, &short.head, 80));
    let long = body(&(1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n"));
    assert!(clips(&long, &long.head, 80));
    // And width decides it: the same message clips when it is narrow enough.
    let wrapped = body("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu");
    assert!(!clips(&wrapped, &wrapped.head, 200));
    assert!(clips(&wrapped, &wrapped.head, 30));
}

#[test]
fn a_message_longer_than_the_head_says_so_even_when_it_is_open() {
    // Longer than `lens::BODY_KEEP`, so the summary only kept its head — and
    // the caller (a test with no record in hand) can only paint that much.
    let text = (1..=600).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let b = body(&text);
    assert!(!b.whole());
    let laid = texts(&rows(&b, b.text_in(None), 80, true, 1));
    let note = laid.last().unwrap().trim().to_string();
    assert!(note.starts_with("\u{22ef} +"), "{note}");
    assert!(note.ends_with("lines"), "{note}");
    assert_eq!(height(&b, b.text_in(None), 80, true), laid.len());
}

#[test]
fn a_control_character_never_reaches_the_screen() {
    let b = body("before\u{7}after");
    let head = first_line(&b, &b.head, 80).unwrap_or_default();
    assert!(head.contains("before\u{b7}after"), "{head:?}");
}

#[test]
fn a_terminal_narrower_than_the_indent_still_paints_the_message() {
    let b = body("alpha beta gamma");
    let (head, laid) = split(&b, 4, true);
    assert!(!laid.is_empty());
    assert!(format!("{head}{}", laid.concat()).contains("alpha"));
}

#[test]
fn the_height_is_always_the_number_of_rows_painted() {
    let cases = ["", "one line", "a\nb\nc", &"word ".repeat(400), &"z".repeat(9000)];
    for text in cases {
        let b = body(text);
        for width in [20usize, 40, 80, 200] {
            for full in [false, true] {
                let n = height(&b, b.text_in(None), width, full);
                let painted = rows(&b, b.text_in(None), width, full, 1).len();
                assert_eq!(n, painted, "{width} full={full} {}", text.len());
            }
        }
    }
}

/// The height and the painted rows are one walk, and a disagreement of one row
/// moves everything below it. Messages of 1, 2, 7 and 300 lines, at the widths
/// a reader actually has, clipped and whole.
#[test]
fn the_height_and_the_rows_agree_at_every_width_and_length() {
    for lines in [1usize, 2, 7, 300] {
        let text = (1..=lines)
            .map(|n| format!("line {n} of the message, long enough to wrap at forty columns"))
            .collect::<Vec<_>>()
            .join("\n");
        let b = body(&text);
        for width in [40usize, 92, 200] {
            for full in [false, true] {
                let n = height(&b, b.text_in(None), width, full);
                let painted = rows(&b, b.text_in(None), width, full, 1);
                assert_eq!(n, painted.len(), "{lines} lines, {width} cols, full={full}");
                // And the summary row is the wrap's first row, never repeated
                // below: one message, painted once.
                let head = first_line(&b, b.text_in(None), width).unwrap_or_default();
                assert!(head.starts_with("line 1 "), "{width}: {head:?}");
                assert!(
                    !texts(&painted).iter().any(|r| r.trim() == head),
                    "{lines} lines, {width} cols, full={full}: {head:?} painted twice"
                );
            }
        }
    }
}

/// A one-line message is entirely on its summary row at every width wide enough
/// to hold it, and nothing under it claims otherwise.
#[test]
fn a_message_that_fits_the_summary_row_has_no_body_at_all() {
    let b = body("short enough");
    for width in [40usize, 92, 200] {
        assert_eq!(height(&b, &b.head, width, false), 0, "{width}");
        assert!(rows(&b, &b.head, width, false, 1).is_empty(), "{width}");
        assert!(!clips(&b, &b.head, width), "{width}");
    }
}
