//! The open level's rows, on their own: no file, no dialect, no pager — a
//! hand-built `Vec<Part>` in, rows out.
//!
//! What these hold to is the rule the whole seam is held to: a part may be
//! clipped and may never be silent about it. Every assertion below either
//! counts rows or reads the text of one; none of them reads a frame.
#![deny(unsafe_code)]

use super::*;
use crate::lens::{Body, Part};

fn body(text: &str) -> Body {
    Body::new(text, Vec::new())
}

fn call(tool: &str, arg: &str, args: &[(&str, &str)], result: Option<&str>) -> Part {
    Part::Call {
        tool: tool.to_string(),
        arg: arg.to_string(),
        args: args.iter().map(|(k, v)| (k.to_string(), body(v))).collect(),
        result: result.map(body),
    }
}

/// Everything on one row, ANSI-free.
fn text_of(rows: &[Line]) -> Vec<String> {
    rows.iter().map(|r| r.text().trim_end().to_string()).collect()
}

/// The **display column** `what` starts at on `row` — never the byte offset: a
/// cut key ends in a one-column `…` that is three bytes wide, and comparing
/// bytes to columns made a correct layout look two columns out.
fn column_of(row: &str, what: &str) -> Option<usize> {
    Some(str_width(&row[..row.find(what)?]))
}

fn shut(_: usize) -> bool {
    false
}

fn open_all(_: usize) -> bool {
    true
}

#[test]
fn a_shut_call_is_one_row_that_says_what_it_did_and_what_came_back() {
    let parts = vec![call("bash", "cargo test -q parse", &[("command", "cargo test -q parse")], Some("a\nb\nc"))];
    let laid = lay(&parts, &shut, None, 92, body::INDENT, 7);
    assert_eq!(laid.rows.len(), 1, "{:?}", text_of(&laid.rows));
    let row = text_of(&laid.rows).remove(0);
    assert!(row.contains("bash"), "{row:?}");
    assert!(row.contains("cargo test -q parse"), "{row:?}");
    assert!(row.contains("\u{2192} 3 lines"), "{row:?}");
    assert!(row.contains(theme::MARKER_CLOSED), "a shut call carries its own glyph: {row:?}");
}

/// The user's words: "can we make this show the output if Enter on that line".
#[test]
fn an_open_call_shows_every_argument_and_the_output() {
    let parts = vec![call(
        "bash",
        "make",
        &[("command", "make -j8"), ("timeout", "120")],
        Some("one\ntwo\nthree"),
    )];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 7);
    let rows = text_of(&laid.rows);
    assert!(rows[0].contains(theme::MARKER_OPEN), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("command") && r.contains("make -j8")), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("timeout") && r.contains("120")), "{rows:#?}");
    for line in ["one", "two", "three"] {
        assert!(rows.iter().any(|r| r.trim() == line), "the output is under it: {rows:#?}");
    }
    // Every row of it belongs to the one call, so `Enter` anywhere in it shuts
    // the same thing it opened.
    assert!(laid.owner.iter().all(|o| *o == 0), "{:?}", laid.owner);
    assert_eq!(laid.call_at(rows.len() - 1), Some(0));
}

#[test]
fn the_height_is_the_rows_and_the_two_come_from_one_walk() {
    let parts = vec![
        Part::Text { label: "thinking", body: body("a\nb") },
        call("read", "src/x.rs", &[("filePath", "src/x.rs")], Some("x")),
    ];
    for open in [false, true] {
        let f = |_: usize| open;
        let laid = lay(&parts, &f, None, 92, body::INDENT, 1);
        assert_eq!(laid.rows.len(), laid.owner.len(), "one owner per row");
        assert_eq!(laid.opens.len(), parts.len(), "one openable flag per part");
    }
}

/// A named stretch of text is not a call: it has no glyph and `Enter` on it is
/// not its own fold.
#[test]
fn a_text_part_says_its_name_and_then_says_it() {
    let parts = vec![Part::Text { label: "thinking", body: body("The fixture was renamed.") }];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert_eq!(rows[0].trim(), "thinking");
    assert_eq!(rows[1].trim(), "The fixture was renamed.");
    assert_eq!(laid.call_at(0), None, "there is nothing under it to open");
    assert_eq!(laid.opens, vec![false]);
}

/// The open level is "the whole of that text" (SPEC.md §Lenses). A thought
/// beside a message is a `Part::Text`, and clipping it here left it cut at six
/// rows at every rung of the ladder with no key that would ever expand it.
#[test]
fn a_text_part_is_whole_because_parts_are_the_open_level() {
    let thought = (1..=30).map(|n| format!("thought {n}")).collect::<Vec<_>>().join("\n");
    let parts = vec![Part::Text { label: "thinking", body: body(&thought) }];
    let laid = lay(&parts, &shut, None, 100, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert_eq!(rows.len(), 31, "the name and every line of it: {}", rows.len());
    assert_eq!(rows[30].trim(), "thought 30");
    assert!(!rows.iter().any(|r| r.contains('\u{22ef}')), "nothing was left out: {rows:#?}");
}

/// The non-negotiable, at this level too (SPEC.md §Lenses): a clip states what
/// it left out, in the text's own lines.
#[test]
fn a_long_output_is_clipped_and_says_how_much_it_left_out() {
    let out = (1..=40).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let parts = vec![call("bash", "make", &[], Some(&out))];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert!(rows.len() < 12, "the output is clipped: {}", rows.len());
    let note = rows.last().expect("a last row");
    assert!(note.trim().starts_with('\u{22ef}'), "{rows:#?}");
    assert!(note.contains("+34 lines"), "{note:?} out of {rows:#?}");
}

/// And so is an argument: a sixty-line patch is not sixty rows, and the row
/// under it says so.
#[test]
fn a_multi_line_argument_is_clipped_and_says_so() {
    let patch = (1..=60).map(|n| format!("+ added line {n}")).collect::<Vec<_>>().join("\n");
    let parts = vec![call("apply_patch", "", &[("patchText", &patch)], None)];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert!(rows[1].contains("patchText"), "the name is beside its value: {rows:#?}");
    assert!(rows[1].contains("+ added line 1"), "{rows:#?}");
    assert!(
        rows.iter().any(|r| r.contains("\u{22ef} +") && r.contains("lines")),
        "{rows:#?}"
    );
    assert!(rows.len() <= 8, "clipped, not sixty rows: {}", rows.len());
}

/// An argument whose value is one long line has no lines to count off, so the
/// remainder is stated in bytes — the same answer a one-line message gets.
#[test]
fn a_single_long_argument_states_its_remainder_in_bytes() {
    let long = "x".repeat(4000);
    let parts = vec![call("bash", "", &[("command", &long)], None)];
    let laid = lay(&parts, &open_all, None, 40, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    let note = rows.last().expect("a last row");
    assert!(note.contains('\u{22ef}'), "{rows:#?}");
    assert!(!note.contains("lines"), "one line has no lines left: {note:?}");
}

/// A call with no arguments and no result still has a row — and carries **no**
/// glyph, because there is nothing under it. A marker on a row that opens to
/// the same screen is the rung `opens_further` refuses one level up.
#[test]
fn a_call_with_nothing_to_show_is_one_row_and_has_no_fold() {
    let parts = vec![call("todowrite", "", &[], None)];
    for open in [false, true] {
        let f = |_: usize| open;
        let laid = lay(&parts, &f, None, 92, body::INDENT, 1);
        assert_eq!(laid.rows.len(), 1, "open {open}: {:?}", text_of(&laid.rows));
        let row = text_of(&laid.rows).remove(0);
        assert!(!row.contains(theme::MARKER_CLOSED), "no fold to advertise: {row:?}");
        assert!(!row.contains(theme::MARKER_OPEN), "no fold to advertise: {row:?}");
        assert_eq!(laid.opens, vec![false], "and `Enter` is the record's");
        assert_eq!(laid.call_at(0), None);
    }
}

/// A call that returned an empty string still opens: "it returned nothing" is
/// something to say, and the `output` row says it.
#[test]
fn a_call_that_returned_nothing_still_opens_and_says_so() {
    let parts = vec![call("ls", "", &[], Some(""))];
    assert!(parts[0].opens(), "an answer is something to show");
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert_eq!(rows.len(), 2, "{rows:#?}");
    assert!(rows[0].contains("empty"), "the row states the true size: {rows:#?}");
    assert_eq!(rows[1].trim(), "output");
}

/// Every argument's value starts in the column its own continuation rows use.
/// The name column is written *into* the pad the wrap left for it, and a
/// one-column disagreement pushed the first row `KEY + 1` columns right of the
/// rest of the value and off the side of the view.
#[test]
fn an_argument_value_is_laid_at_one_column_only() {
    let long = "one two three four five six seven eight nine ten eleven twelve thirteen";
    let parts = vec![call("bash", "echo", &[("command", long)], None)];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    assert!(rows.len() >= 3, "the value wraps: {rows:#?}");
    let value_at = column_of(&rows[1], "one two").expect("the value is on the name's row");
    let cont_at = str_width(&rows[2]) - str_width(rows[2].trim_start());
    assert_eq!(value_at, cont_at, "one alignment, not two: {rows:#?}");
    assert_eq!(value_at, body::INDENT + 4 + KEY_COL, "{rows:#?}");
    for r in &rows {
        assert!(str_width(r) <= 92, "no row overflows the view: {r:?}");
    }
}

/// A key wider than the column is cut rather than pushing the value out of
/// line — the alignment is measured in display columns, not bytes.
#[test]
fn a_wide_or_multibyte_key_keeps_the_value_column() {
    for key in ["a_very_long_argument_name", "\u{5f15}\u{6570}", "x"] {
        let parts = vec![call("t", "", &[(key, "value")], None)];
        let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
        let rows = text_of(&laid.rows);
        let at = column_of(&rows[1], "value").expect("the value is there");
        assert_eq!(at, body::INDENT + 4 + KEY_COL, "key {key:?}: {rows:#?}");
    }
}

/// The output gets a row of its own naming it, so output whose lines read
/// `key   value` cannot be mistaken for two more arguments.
#[test]
fn the_output_is_named_so_it_cannot_read_as_an_argument() {
    let parts = vec![call("bash", "ls", &[("command", "ls")], Some("timeout   120\nverbose   true"))];
    let laid = lay(&parts, &open_all, None, 92, body::INDENT, 1);
    let rows = text_of(&laid.rows);
    let at = rows.iter().position(|r| r.trim() == "output").expect("an output row: {rows:#?}");
    assert!(rows[at - 1].contains("command"), "it comes after the arguments: {rows:#?}");
    assert!(rows[at + 1].contains("timeout"), "and before the output: {rows:#?}");
}

/// A member of an open run is inset, and everything under its row goes with it.
#[test]
fn a_base_column_moves_the_whole_level() {
    let parts = vec![
        Part::Text { label: "thinking", body: body("a thought") },
        call("bash", "make", &[("command", "make -j8")], Some("ok")),
    ];
    let plain = text_of(&lay(&parts, &open_all, None, 92, body::INDENT, 1).rows);
    let inset = text_of(&lay(&parts, &open_all, None, 92, body::INDENT + 2, 1).rows);
    assert_eq!(plain.len(), inset.len(), "an inset moves rows sideways, not down");
    for (a, b) in plain.iter().zip(&inset) {
        let (a_at, b_at) = (a.len() - a.trim_start().len(), b.len() - b.trim_start().len());
        assert_eq!(a_at + 2, b_at, "{a:?} vs {b:?}");
    }
}

/// Rows are laid at any width, including one narrower than the indent — the
/// column floor in `body::columns` is what keeps that from being zero columns
/// and an endless wrap.
#[test]
fn every_width_lays_out() {
    let parts = vec![
        Part::Text { label: "thinking", body: body("a thought worth having twice over") },
        call("bash", "make", &[("command", "make -j8 all")], Some("done\nok")),
    ];
    for width in [1usize, 10, 21, 40, 92, 200] {
        let laid = lay(&parts, &open_all, None, width, body::INDENT, 1);
        assert!(!laid.rows.is_empty(), "width {width}");
        assert_eq!(laid.rows.len(), laid.owner.len(), "width {width}");
    }
}
