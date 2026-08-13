//! What a record file must do (SPEC.md §JSON): the row arithmetic, the list
//! view, a bad line not stopping the file, expansion, laziness and the yanks.
#![deny(unsafe_code)]

use std::path::PathBuf;

use super::*;
use crate::source::Source;
use crate::source::{Anchor, Dir, End, Mark};

// -- fixtures ----------------------------------------------------------------

/// A temp file that removes itself.
struct Tmp {
    path: PathBuf,
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn tmp(name: &str, body: &[u8]) -> Tmp {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-jsonl-{}-{nanos}-{name}.jsonl", std::process::id()));
    std::fs::write(&p, body).expect("write fixture");
    Tmp { path: p }
}

/// A source over `text`, laid out 100 columns wide.
fn src(text: &str) -> JsonlSource {
    let mut s = JsonlSource::from_bytes(text.as_bytes().to_vec());
    s.set_width(100);
    s
}

/// The whole document as plain text rows, index driven to the end first.
fn rows(s: &mut JsonlSource) -> Vec<String> {
    while s.extend() {}
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text()).collect()
}

const THREE: &str = concat!(
    "{\"type\":\"user\",\"n\":1}\n",
    "{\"type\":\"assistant\",\"n\":2,\"items\":[10,20]}\n",
    "{\"type\":\"system\",\"n\":3}\n",
);

// -- the list view --------------------------------------------------------------

#[test]
fn the_default_view_is_one_row_per_record() {
    let mut s = src(THREE);
    let got = rows(&mut s);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[0], "\u{25be} {\u{2026}2 keys}  \u{b7} type: \"user\"  \u{b7} n: 1");
    assert!(got[1].starts_with("\u{25be} {\u{2026}3 keys}  \u{b7} type: \"assistant\""));
    assert_eq!(got[2], "\u{25be} {\u{2026}2 keys}  \u{b7} type: \"system\"  \u{b7} n: 3");
}

#[test]
fn a_summary_counts_and_previews_but_never_lies_about_the_count() {
    let mut s = src("{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5}\n[1,2,3]\n\"bare\"\n{}\n");
    let got = rows(&mut s);
    assert_eq!(got[0], "\u{25be} {\u{2026}5 keys}  \u{b7} a: 1  \u{b7} b: 2  \u{b7} c: 3");
    assert_eq!(got[1], "\u{25be} [\u{2026}3 items]  \u{b7} 1  \u{b7} 2  \u{b7} 3");
    // A scalar record has nothing to open, so no fold marker.
    assert_eq!(got[2], "  \"bare\"");
    // An empty container has a bracket to close and the document source shows
    // it the same way, so it opens here rather than being a leaf in one reader.
    assert_eq!(got[3], "\u{25be} {}");
}

#[test]
fn a_long_scalar_is_not_previewed_beside_the_count() {
    let long = "x".repeat(200);
    let mut s = src(&format!("{{\"big\":\"{long}\",\"small\":7}}\n"));
    let got = rows(&mut s);
    assert_eq!(got[0], "\u{25be} {\u{2026}2 keys}  \u{b7} small: 7");
}

#[test]
fn a_null_is_counted_but_never_previewed() {
    // `parentUuid: null` says nothing about which record this is.
    let mut s = src("{\"parentUuid\":null,\"type\":\"user\"}\n");
    let got = rows(&mut s);
    assert_eq!(got[0], "\u{25be} {\u{2026}2 keys}  \u{b7} type: \"user\"");
    let entry = s.section_at(0).unwrap();
    s.set_fold(entry, false);
    let all: Vec<String> = s.lines(0..s.len()).iter().map(|l| l.text()).collect();
    assert_eq!(all[1], "    \"parentUuid\": null", "open, the null is there");
}

#[test]
fn a_bad_line_is_an_error_row_and_the_file_keeps_going() {
    let mut s = src("{\"a\":1}\nnope\n{\"b\":2}\n   \n{\"c\":3}\n");
    let got = rows(&mut s);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got[1].starts_with("  line 2: "), "{:?}", got[1]);
    assert!(got[3].contains("line 4: blank line"), "{:?}", got[3]);
    assert!(got[4].contains("c: 3"), "{:?}", got[4]);
    // An error row is a leaf: nothing to fold, nothing hidden.
    assert_eq!(s.hidden_at(1), None);
    assert_eq!(s.section_at(1), None);
}

#[test]
fn crlf_and_a_byte_order_mark_are_not_part_of_the_record() {
    let mut s = src("\u{feff}{\"a\":1}\r\n{\"b\":2}\r\n");
    let got = rows(&mut s);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].contains("a: 1"), "{:?}", got[0]);
    assert!(got[1].contains("b: 2"), "{:?}", got[1]);
}

#[test]
fn a_quote_does_not_swallow_the_next_line() {
    // The CSV grammar would treat the opening `"` of a key as a quoted field
    // and every newline after it as content. A line scanner must not.
    let mut s = src("{\"a\":\"one\"}\n{\"a\":\"two\"}\n{\"a\":\"three\"}\n");
    assert_eq!(rows(&mut s).len(), 3);
}

// -- expansion ---------------------------------------------------------------

#[test]
fn enter_expands_a_record_into_its_tree() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    assert_eq!(s.len(), 3);
    // Record 1 holds two scalars and an array of two, and expands into the tree
    // the document source draws: both brackets, and no index label.
    assert_eq!(s.hidden_at(1), Some(7));
    let entry = s.section_at(1).expect("record 1 is a section");
    assert!(s.set_fold(entry, false));
    assert_eq!(s.len(), 3 + 7);
    let got: Vec<String> = s.lines(0..s.len()).iter().map(|l| l.text()).collect();
    assert_eq!(
        &got[1..9],
        &[
            "\u{25be} {\u{2026}3 keys}  \u{b7} type: \"assistant\"  \u{b7} n: 2".to_string(),
            "    \"type\": \"assistant\"".to_string(),
            "    \"n\": 2".to_string(),
            "  \u{25be} \"items\": [".to_string(),
            "      10".to_string(),
            "      20".to_string(),
            "    ]".to_string(),
            "  }".to_string(),
        ]
    );
    // Open, it hides nothing.
    assert_eq!(s.hidden_at(1), None);
    let entry = s.section_at(1).expect("still a section");
    assert!(s.set_fold(entry, true));
    assert_eq!(s.len(), 3);
}

/// With **no lens** there is no ladder, so a record row has no fold of its own
/// and `Enter` must reach the outline — where a record's entry is exactly the
/// tree above. Under a lens the same row claims the key instead (the record's
/// two levels, and `r` owning the tree); this is the other side of that fork,
/// and the reason it is asked as "is there a plan" rather than assumed.
#[test]
fn without_a_lens_enter_falls_through_to_the_outline() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    assert_eq!(s.fold_here(1), None, "no rung of its own without a lens");
    assert_eq!(s.len(), 3, "and nothing opened behind the answer");
    let entry = s.section_at(1).expect("the outline still has the record");
    assert!(s.set_fold(entry, false), "which is what opens its tree");
    assert!(s.len() > 3);
}

#[test]
fn fold_state_is_the_open_records_and_survives_a_round_trip() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    let entry = s.section_at(1).unwrap();
    s.set_fold(entry, false);
    let state = s.folds();
    // The shared fold-id vocabulary (`crate::source::jsonrow::ALL_OPEN`).
    assert_eq!(state, vec!["/1".to_string()]);
    let mut other = src(THREE);
    other.set_folds(state);
    assert_eq!(other.len(), 3 + 7);
    other.set_folds(vec!["/9999".to_string(), "nonsense".to_string()]);
    assert_eq!(other.len(), 3, "ids that do not exist are ignored");
}

#[test]
fn zm_shuts_everything_and_zr_opens_what_the_viewport_reaches() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    s.fold_all(false);
    let opened = s.len();
    assert!(opened > 3, "zR opened nothing");
    s.fold_all(true);
    assert_eq!(s.len(), 3);
}

#[test]
fn tab_steps_by_record_even_inside_an_open_one() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    let entry = s.section_at(1).unwrap();
    s.set_fold(entry, false);
    assert_eq!(s.next_landmark(0, true), Some(1));
    // From inside record 1, back goes to its own summary row first.
    assert_eq!(s.next_landmark(3, false), Some(1));
    assert_eq!(s.next_landmark(1, false), Some(0));
    assert_eq!(s.next_landmark(1, true), Some(9));
    assert_eq!(s.next_landmark(9, true), None);
}

#[test]
fn a_record_number_is_an_anchor_id() {
    let mut s = src(THREE);
    assert_eq!(s.goto_id("2"), Some(2));
    assert_eq!(s.goto_id("#0"), Some(0));
    assert_eq!(s.goto_id("99"), None);
    assert_eq!(s.goto_id("not a number"), None);
}

// -- values ------------------------------------------------------------------

#[test]
fn numbers_keep_their_source_text_and_duplicate_keys_are_kept() {
    let mut s = src("{\"a\":1e999,\"b\":0.10,\"a\":123456789012345678901234567890}\n");
    let _ = s.lines(0..1);
    let entry = s.section_at(0).unwrap();
    s.set_fold(entry, false);
    let got: Vec<String> = s.lines(0..s.len()).iter().map(|l| l.text()).collect();
    assert_eq!(
        &got[1..],
        &[
            "    \"a\": 1e999".to_string(),
            "    \"b\": 0.10".to_string(),
            "    \"a\": 123456789012345678901234567890".to_string(),
            "  }".to_string(),
        ]
    );
}

#[test]
fn control_characters_in_a_string_are_neutralised_on_screen() {
    let mut s = src("{\"s\":\"a\\u0007b\\tc\"}\n");
    let got = rows(&mut s);
    assert!(!got[0].contains('\u{7}'), "{:?}", got[0]);
    assert!(!got[0].contains('\t'), "{:?}", got[0]);
}

/// Ten thousand levels in one record. Nothing recurses, and the walk stops at
/// the shared presentation limit — the same place the document source stops, so
/// the two render the same rows (`tests/json_differential.rs`).
#[test]
fn ten_thousand_levels_of_nesting_do_not_blow_the_stack() {
    let depth = 10_000;
    let cap = crate::source::jsonrow::MAX_DEPTH;
    let body = format!("{}{}\n", "[".repeat(depth), "]".repeat(depth));
    let mut s = src(&body);
    // Summarise, count, lay out and index a path — every walker, on the same
    // hostile document, in one thread's stack.
    let got = rows(&mut s);
    assert_eq!(got.len(), 1);
    assert!(got[0].contains("[\u{2026}1 item]"), "{:?}", got[0]);
    // A bracket row and a closing row for every level down to the limit, plus
    // the one note row that stands for everything past it, bar the record's
    // own summary row.
    assert_eq!(s.hidden_at(0), Some(2 * (cap + 1)));
    let entry = s.section_at(0).unwrap();
    assert!(s.set_fold(entry, false));
    assert_eq!(s.len(), 2 * (cap + 1) + 1);
    let window = s.lines(0..40);
    assert_eq!(window.len(), 40);
    assert!(s.lines(cap + 1..cap + 2)[0].text().contains("nested deeper than"));
    assert!(s.position_text(2 * cap).is_some());
    assert!(s.yank_section(0).is_some());
}

/// Nesting to exactly the limit is rendered whole: the refusal above is a
/// bound on hostile input, not a ceiling ordinary records meet.
#[test]
fn nesting_to_the_limit_is_rendered_whole() {
    let depth = crate::source::jsonrow::MAX_DEPTH;
    let body = format!("{}9{}\n", "[".repeat(depth), "]".repeat(depth));
    let mut s = src(&body);
    assert_eq!(rows(&mut s).len(), 1);
    let entry = s.section_at(0).unwrap();
    assert!(s.set_fold(entry, false));
    assert_eq!(s.len(), 2 * depth + 1);
    assert!(s.lines(depth..depth + 1)[0].text().contains('9'));
}

// -- status ------------------------------------------------------------------

#[test]
fn the_status_bar_names_the_record_the_total_and_the_path() {
    let mut s = src(THREE);
    while s.extend() {}
    let _ = s.lines(0..3);
    assert_eq!(s.position_text(0).as_deref(), Some("record 1/3"));
    let entry = s.section_at(1).unwrap();
    s.set_fold(entry, false);
    let _ = s.lines(0..s.len());
    assert_eq!(s.position_text(1).as_deref(), Some("record 2/3"));
    assert_eq!(
        s.position_text(5).as_deref(),
        Some("record 2/3  \u{b7}  .items[0]")
    );
}

#[test]
fn the_total_is_a_lower_bound_while_the_index_is_still_lazy() {
    let body: String = (0..40_000).map(|i| format!("{{\"n\":{i}}}\n")).collect();
    let t = tmp("lazy-total", body.as_bytes());
    let mut s = JsonlSource::open(&t.path).expect("open");
    s.set_width(100);
    let _ = s.lines(0..40);
    let text = s.position_text(0).expect("a position");
    assert!(text.starts_with("record 1/\u{2265}"), "{text}");
    assert!(text.contains("indexing"), "{text}");
    assert!(matches!(s.end(), End::Scanning(_)), "the end is not known yet");
    while s.extend() {}
    assert_eq!(s.position_text(0).as_deref(), Some("record 1/40000"));
    assert_eq!(s.end(), End::At(39_999));
}

// -- laziness ----------------------------------------------------------------

#[test]
fn opening_and_painting_a_screen_reads_a_screen_not_a_file() {
    let pad = "x".repeat(60);
    let body: String =
        (0..200_000).map(|i| format!("{{\"n\":{i},\"pad\":\"{pad}\"}}\n")).collect();
    let t = tmp("lazy-open", body.as_bytes());
    let mut s = JsonlSource::open(&t.path).expect("open");
    assert_eq!(s.known(), 0, "opening indexes nothing");
    s.set_width(100);
    let _ = s.lines(0..40);
    let known = s.known();
    assert!(known >= 40, "only {known} records indexed");
    assert!(known < 200_000, "the whole file was indexed for one screen");
    assert!(matches!(s.end(), End::Scanning(_)));
}

// -- search ------------------------------------------------------------------

#[test]
fn search_finds_a_record_and_wraps() {
    let mut s = src(THREE);
    while s.extend() {}
    let _ = s.lines(0..3);
    s.set_query("system");
    let hit = s.preview_match(Anchor(0), Dir::Forward).expect("a hit");
    assert_eq!(hit.anchor, Anchor(2));
    assert!(!hit.wrapped);
    assert_eq!(s.match_count(), 1);
    let again = s.cycle_match(Anchor(2), Dir::Forward).expect("wraps to itself");
    assert!(again.wrapped);
    assert_eq!(again.anchor, Anchor(2));
    s.set_query("nothing here");
    assert!(s.preview_match(Anchor(0), Dir::Forward).is_none());
    assert_eq!(s.match_count(), 0);
}

#[test]
fn a_match_is_highlighted_on_the_row_it_is_on() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    s.set_query("assistant");
    assert!(!s.matches_on(1).is_empty());
    assert!(s.matches_on(0).is_empty());
}

// -- yank --------------------------------------------------------------------

#[test]
fn y_copies_the_value_under_the_cursor() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    let entry = s.section_at(1).unwrap();
    s.set_fold(entry, false);
    let _ = s.lines(0..s.len());
    // A string is copied as its characters, not re-quoted.
    let y = s.yank_point(2).expect("the type field");
    assert_eq!(y.text, "assistant\n");
    // A container is copied as JSON.
    let y = s.yank_point(4).expect("the items array");
    assert_eq!(y.text, "[10,20]\n");
    assert!(y.what.contains(".items"), "{}", y.what);
}

#[test]
fn capital_y_copies_the_whole_record_as_one_line_of_json() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    let entry = s.section_at(1).unwrap();
    s.set_fold(entry, false);
    let _ = s.lines(0..s.len());
    for row in 1..=6 {
        let y = s.yank_section(row).expect("a record");
        assert_eq!(y.text, "{\"type\":\"assistant\",\"n\":2,\"items\":[10,20]}\n");
        assert_eq!(y.what, "record 2");
    }
}

#[test]
fn c_copies_the_line_verbatim() {
    let text = "{ \"a\" : 1 ,  \"b\" : 2 }\n";
    let mut s = src(text);
    let _ = s.lines(0..1);
    let y = s.yank_block(0).expect("the line");
    assert_eq!(y.text, text, "the file's own bytes, not a re-serialisation");
}

#[test]
fn a_visual_selection_copies_one_value_per_row() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    let y = s.yank_rows(0..3).expect("three records");
    assert_eq!(
        y.text,
        "{\"type\":\"user\",\"n\":1}\n\
         {\"type\":\"assistant\",\"n\":2,\"items\":[10,20]}\n\
         {\"type\":\"system\",\"n\":3}\n"
    );
    assert_eq!(y.what, "3 values");
}

// -- the seam ----------------------------------------------------------------

#[test]
fn an_empty_file_is_empty_rather_than_a_panic() {
    let mut s = src("");
    assert_eq!(s.len(), 0);
    assert!(s.is_empty());
    assert!(s.lines(0..10).is_empty());
    assert_eq!(s.anchor(0), None);
    assert_eq!(s.reveal(Anchor(5)), None);
    assert_eq!(s.locate(Mark(5)), None);
    assert_eq!(s.hidden_at(0), None);
    assert!(s.yank_rows(0..10).is_none());
    assert!(s.yank_section(0).is_none());
    assert!(s.yank_block(0).is_none());
    assert!(s.outline().is_empty());
    assert!(s.links().is_empty());
}

#[test]
fn rows_past_the_end_are_none_rather_than_a_panic() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    assert!(s.lines(100..200).is_empty());
    assert_eq!(s.anchor(99), None);
    assert_eq!(s.row_of(Anchor(99)), None);
    assert_eq!(s.reveal(Anchor(99)), Some(2));
    assert_eq!(s.locate(Mark(99)), Some(2));
    assert!(s.yank_point(99).is_none());
    assert_eq!(s.mark(99), None);
}

#[test]
fn lines_returns_exactly_the_window_it_was_asked_for() {
    let mut s = src(THREE);
    while s.extend() {}
    for start in 0..4 {
        for end in start..5 {
            let want = end.min(s.len()) - start.min(s.len());
            assert_eq!(s.lines(start..end).len(), want, "{start}..{end}");
        }
    }
}

/// The one place a `blocks()` default could silently catch a format: this is
/// the same `impl` that answers `true` under a lens. Without one a record file
/// is the generic tree — one record per row, one node per row — with nothing
/// for a landing to frame, and `Tab` is the next record.
#[test]
fn a_record_file_with_no_lens_does_not_read_in_blocks() {
    let mut s = src(THREE);
    let _ = s.lines(0..3);
    assert!(!s.blocks());
    assert_eq!(s.block_at(0), None);
    assert_eq!(s.next_landmark(0, true), Some(1), "still the next record");
}
