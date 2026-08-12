//! JSON behind the seam, as the pager sees it.
//!
//! Layout is asserted on the ANSI-stripped text and colour on the spans
//! separately (SPEC.md §Testing), so a palette change does not break the
//! layout tests.
#![deny(unsafe_code)]

use super::*;
use crate::source::{End, Source};
use crate::term::Style;

fn src(text: &str) -> JsonSource {
    let mut s = JsonSource::from_bytes(text.as_bytes().to_vec());
    s.set_width(80);
    s
}

/// Every row's text, with trailing spaces trimmed.
fn shown(s: &mut JsonSource) -> Vec<String> {
    while s.extend() {}
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

const DOC: &str = r#"{
  "name": "ada",
  "age": 36,
  "admin": true,
  "note": null,
  "tags": ["x", "y"],
  "meta": {"a": 1}
}"#;

#[test]
fn a_document_opens_with_the_root_open_and_its_children_folded() {
    let mut s = src(DOC);
    assert_eq!(
        shown(&mut s),
        vec![
            "\u{25be} {",
            "    \"name\": \"ada\"",
            "    \"age\": 36",
            "    \"admin\": true",
            "    \"note\": null",
            "  \u{25b8} \"tags\": [\u{2026}2 items]",
            "  \u{25b8} \"meta\": {\u{2026}1 key}",
            "  }",
        ]
    );
}

/// Strings keep their quotes, because `"1"` and `1` are different values.
#[test]
fn a_string_is_shown_quoted_and_a_number_is_not() {
    let mut s = src(r#"["1", 1]"#);
    let rows = shown(&mut s);
    assert!(rows[1].ends_with("\"1\""), "{:?}", rows[1]);
    assert!(rows[2].ends_with('1') && !rows[2].contains('"'), "{:?}", rows[2]);
}

/// Five kinds, five colours (SPEC.md §JSON, "The tree").
#[test]
fn keys_strings_numbers_booleans_and_null_are_coloured_apart() {
    let mut s = src(DOC);
    let lines = s.lines(0..8);
    let style_of = |row: usize, needle: &str| -> Style {
        lines[row]
            .spans
            .iter()
            .find(|sp| sp.text.contains(needle))
            .map(|sp| sp.style)
            .unwrap_or_else(|| panic!("no span holding {needle} on row {row}"))
    };
    assert_eq!(style_of(1, "\"name\""), crate::theme::json_key());
    assert_eq!(style_of(1, "\"ada\""), crate::theme::json_string());
    assert_eq!(style_of(2, "36"), crate::theme::json_number());
    assert_eq!(style_of(3, "true"), crate::theme::json_bool());
    assert_eq!(style_of(4, "null"), crate::theme::json_null());
    let all = [
        crate::theme::json_key(),
        crate::theme::json_string(),
        crate::theme::json_number(),
        crate::theme::json_bool(),
        crate::theme::json_null(),
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "two kinds share a style");
        }
    }
}

#[test]
fn control_characters_in_a_string_are_shown_as_the_escapes_the_file_holds() {
    let mut s = src(r#"["a\nb\tc d"]"#);
    let row = &shown(&mut s)[1];
    // The file holds `\n` and `\t` as two characters each, so that is what is
    // shown — not a raw control character, and not a `\u{b7}` stand-in that
    // would claim the document contains something it does not.
    assert!(row.contains(r"a\nb\tc d"), "{row:?}");
    assert!(!row.contains('\n'), "never a raw newline");
    assert!(!row.contains('\t'), "never a raw tab");
}

#[test]
fn a_collapsed_container_counts_itself() {
    let mut s = src(r#"{"one": [1], "many": {"a":1,"b":2,"c":3}, "none": []}"#);
    let rows = shown(&mut s);
    assert!(rows[1].contains("[\u{2026}1 item]"), "{:?}", rows[1]);
    assert!(rows[2].contains("{\u{2026}3 keys}"), "{:?}", rows[2]);
    // An empty container says what it is rather than counting to zero, in
    // both JSON readers (`crate::source::jsonrow::summary_text`).
    assert!(rows[3].ends_with("\"none\": []"), "{:?}", rows[3]);
}

#[test]
fn opening_a_node_shows_its_members_indented() {
    let mut s = src(DOC);
    s.lines(0..8);
    let entry = s.section_at(5).expect("the tags row has a section");
    assert!(s.set_fold(entry, false), "open it");
    let rows = shown(&mut s);
    assert_eq!(
        &rows[5..9],
        &[
            "  \u{25be} \"tags\": [".to_string(),
            "      \"x\"".to_string(),
            "      \"y\"".to_string(),
            "    ]".to_string(),
        ]
    );
    // And shutting it again puts the summary back.
    let entry = s.section_at(5).unwrap();
    assert!(s.set_fold(entry, true));
    assert!(shown(&mut s)[5].contains("[\u{2026}2 items]"));
}

/// `za` on a scalar folds the container it sits in, rather than saying there
/// is nothing to fold.
#[test]
fn folding_at_a_scalar_folds_its_parent() {
    let mut s = src(DOC);
    s.lines(0..8);
    let entry = s.section_at(1).expect("a section");
    assert_eq!(s.outline()[entry].id, "", "the root owns the scalar");
    assert!(s.set_fold(entry, true));
    assert_eq!(shown(&mut s), vec!["\u{25b8} {\u{2026}6 keys}"]);
}

#[test]
fn fold_state_survives_being_stored_and_handed_back() {
    let mut s = src(DOC);
    s.lines(0..8);
    let entry = s.section_at(6).unwrap();
    s.set_fold(entry, false);
    let open = shown(&mut s);
    let state = s.folds();
    let mut other = src(DOC);
    other.set_folds(state);
    assert_eq!(shown(&mut other), open);
}

/// `zR` for the dump path: everything open, in one boolean.
#[test]
fn expanding_everything_shows_the_whole_document() {
    let mut s = src(DOC);
    s.fold_all(false);
    assert_eq!(
        shown(&mut s),
        vec![
            "\u{25be} {",
            "    \"name\": \"ada\"",
            "    \"age\": 36",
            "    \"admin\": true",
            "    \"note\": null",
            "  \u{25be} \"tags\": [",
            "      \"x\"",
            "      \"y\"",
            "    ]",
            "  \u{25be} \"meta\": {",
            "      \"a\": 1",
            "    }",
            "  }",
        ]
    );
    s.fold_all(true);
    assert_eq!(shown(&mut s).len(), 1, "zM shuts the root itself");
}

#[test]
fn the_status_bar_names_the_path_under_the_cursor() {
    let mut s = src(r#"{"users": [{"name": "ada"}]}"#);
    s.fold_all(false);
    shown(&mut s);
    let path = |row: usize| {
        s.position_text(row)
            .unwrap()
            .split("\u{b7}")
            .nth(1)
            .unwrap()
            .trim()
            .to_string()
    };
    assert_eq!(path(0), ".");
    assert_eq!(path(1), ".users");
    assert_eq!(path(2), ".users[0]");
    assert_eq!(path(3), ".users[0].name");
}

#[test]
fn the_total_is_honest_while_the_document_is_still_being_walked() {
    let big = format!("[{}]", vec!["1"; 200_000].join(","));
    let mut s = src(&big);
    let text = s.position_text(0).unwrap();
    assert!(text.contains("\u{2265}"), "{text}");
    assert!(text.contains("indexing"), "{text}");
    assert!(matches!(s.end(), End::Scanning(_)));
    while s.extend() {}
    let text = s.position_text(0).unwrap();
    assert!(!text.contains("\u{2265}"), "{text}");
    assert_eq!(s.end(), End::At(200_001));
}

/// Nothing may read the whole file on the open path, at any size.
#[test]
fn opening_a_large_document_reads_almost_none_of_it() {
    let big = format!("[{}]", vec!["\"aaaaaaaaaaaaaaaaaaaa\""; 200_000].join(","));
    let mut s = src(&big);
    s.lines(0..40);
    let walked = s.doc.borrow().walked();
    assert!(
        walked < 64 * 1024,
        "walked {walked} bytes of {} to paint a screen",
        big.len()
    );
    assert!(s.len() >= 40);
}

/// Laziness at *every* level, which is the claim a top-level-only index would
/// pass the test above and fail here: one object holding one enormous array,
/// with the array opened. Painting a screen of it must cost a screen, not the
/// array — and closing it again must not have cost the rest either.
#[test]
fn opening_a_node_inside_a_large_document_reads_only_that_screen() {
    let inner = vec!["{\"i\":123456,\"s\":\"abcdefghijklmnop\"}"; 200_000].join(",");
    let text = format!("{{\"head\": 1, \"big\": [{inner}], \"tail\": 2}}");
    let mut s = src(&text);
    s.lines(0..40);
    // Open `.big` — member 1 of the root, so `/1` in the positional fold ids
    // the flatten uses. The root's own scan has to skip the array to know the
    // member is there, so what is measured is the work *after* that.
    while s.doc.borrow().node(0).member(1).is_none() && s.extend() {}
    let before = s.doc.borrow().walked();
    assert!(s.folds.set("/1", true), "open the array");
    s.refold();
    let rows = s.lines(0..40);
    assert_eq!(rows.len(), 40);
    assert!(rows[2].text().contains("\"big\": ["), "{:?}", rows[2].text());
    let cost = s.doc.borrow().walked() - before;
    assert!(
        cost < 256 * 1024,
        "walked {cost} more bytes of {} to paint a screen inside the array",
        text.len()
    );
}

#[test]
fn a_member_too_large_to_show_says_so_by_size() {
    let huge = "x".repeat((tree::PARSE_CAP + 1024) as usize);
    let mut s = src(&format!("[\"{huge}\", 1]"));
    let rows = s.lines(0..3);
    let text = rows[1].text();
    assert!(text.contains("MB"), "{text}");
    assert!(text.contains("display limit"), "{text}");
    assert!(rows[2].text().contains('1'), "the next member still renders");
}

#[test]
fn a_member_that_is_not_json_is_an_error_row_and_the_file_still_reads() {
    let mut s = src("[1, tru, 3]");
    let rows = shown(&mut s);
    assert!(rows[1].contains('1'));
    assert!(rows[2].contains("not JSON"), "{:?}", rows[2]);
    assert!(rows[3].contains('3'), "{:?}", rows[3]);
}

#[test]
fn yanks_are_the_documents_own_bytes() {
    let mut s = src(r#"{"s": "a b", "n": 1e999, "sub": {"k": [1, 2]}}"#);
    s.lines(0..6);
    // `y`: the value, a string without its quotes.
    assert_eq!(s.yank_point(1).unwrap().text, "a b");
    assert_eq!(s.yank_point(2).unwrap().text, "1e999", "the source text");
    // `Y`: the subtree as valid JSON.
    let y = s.yank_section(3).unwrap();
    assert_eq!(y.text, r#"{"k":[1,2]}"#);
    assert_eq!(y.what, ".sub");
    assert!(crate::json::parse(y.text.as_bytes()).is_ok());
    // `c`: verbatim, exactly as written.
    assert_eq!(s.yank_block(3).unwrap().text, r#"{"k": [1, 2]}"#);
    // A string is still quoted when it is copied as JSON.
    assert_eq!(s.yank_section(1).unwrap().text, "\"a b\"");
}

#[test]
fn a_selection_yanks_the_source_it_covers() {
    let mut s = src("[1, 2, 3, 4]");
    s.lines(0..6);
    let y = s.yank_rows(1..4).unwrap();
    assert_eq!(y.text, "1, 2, 3");
    assert_eq!(y.what, "3 rows");
}

#[test]
fn search_finds_a_row_and_reports_its_columns() {
    let mut s = src(r#"{"alpha": 1, "beta": 2}"#);
    shown(&mut s);
    s.set_query("beta");
    let hit = s.cycle_match(Anchor(0), Dir::Forward).expect("a hit");
    assert_eq!(hit.anchor, Anchor(2));
    assert_eq!(s.match_count(), 1);
    let spans = s.matches_on(2);
    assert_eq!(spans.len(), 1);
    assert!(spans[0].current);
    assert!(s.matches_on(1).is_empty());
}

#[test]
fn tab_moves_between_open_containers() {
    let mut s = src(DOC);
    s.fold_all(false);
    shown(&mut s);
    assert_eq!(s.next_landmark(0, true), Some(5), "the tags array");
    assert_eq!(s.next_landmark(5, true), Some(9), "the meta object");
    assert_eq!(s.next_landmark(9, true), None);
    assert_eq!(s.next_landmark(9, false), Some(5));
}

#[test]
fn an_empty_document_is_empty_rather_than_a_panic() {
    let mut s = src("   ");
    assert_eq!(s.len(), 0);
    assert!(s.lines(0..10).is_empty());
    assert_eq!(s.anchor(0), None);
    assert_eq!(s.section_at(0), None);
    assert!(s.yank_point(0).is_none());
    assert!(s.position_text(0).unwrap().contains("row 1"));
}

#[test]
fn a_scalar_document_is_one_row() {
    let mut s = src("  42  ");
    assert_eq!(shown(&mut s), vec!["  42"]);
}

/// Ten thousand levels of `[[[[`, all the way through the seam: flatten,
/// render, fold and yank. An iterative parser behind a recursive renderer is
/// still a stack overflow.
///
/// Past [`MAX_DEPTH`] the document is a *flat render* rather than a deeper one
/// (SPEC.md §JSON): one note row stands for everything below the limit, which
/// is also what stops the per-level byte re-walk from turning hostile nesting
/// into a hang.
#[test]
fn a_document_nested_ten_thousand_deep_renders() {
    const DEPTH: usize = 10_000;
    let cap = crate::source::json::tree::MAX_DEPTH as usize;
    let text = format!("{}\"deep\"{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    let mut s = src(&text);
    s.fold_all(false);
    while s.extend() {}
    assert_eq!(s.len(), (cap + 1) * 2 + 1);
    let rows = s.lines(cap..cap + 2);
    assert!(rows[0].text().contains('['), "{:?}", rows[0].text());
    assert!(rows[1].text().contains("nested deeper than"), "{:?}", rows[1].text());
    assert!(s.yank_section(0).is_some());
    assert!(s.position_text(cap).unwrap().len() > 10);
}

/// A document nested to exactly the limit still shows its innermost value: the
/// refusal above is a limit on hostile nesting, not a ceiling ordinary
/// documents run into.
#[test]
fn a_document_nested_to_the_limit_still_shows_its_deepest_value() {
    let depth = crate::source::json::tree::MAX_DEPTH as usize;
    let text = format!("{}\"deep\"{}", "[".repeat(depth), "]".repeat(depth));
    let mut s = src(&text);
    s.fold_all(false);
    while s.extend() {}
    assert_eq!(s.len(), depth * 2 + 1);
    assert!(s.lines(depth..depth + 1)[0].text().contains("\"deep\""));
}

/// The rows of a document opened from a file are the rows of the same document
/// held in memory: the file path is a different `Reader`, not a different
/// reader.
#[test]
fn a_file_and_a_pipe_render_the_same_document() {
    let mut path = std::env::temp_dir();
    path.push(format!("tread-json-{}.json", std::process::id()));
    std::fs::write(&path, DOC.as_bytes()).expect("write");
    let mut from_file = JsonSource::open(&path).expect("open");
    from_file.set_width(80);
    let mut from_pipe = src(DOC);
    assert_eq!(shown(&mut from_file), shown(&mut from_pipe));
    std::fs::remove_file(&path).ok();
}

/// `--toc` lists the root's members and walks nothing else.
#[test]
fn the_outline_flag_lists_the_roots_members() {
    let mut s = src(DOC);
    assert_eq!(
        s.toc(),
        vec![".name", ".age", ".admin", ".note", ".tags", ".meta"]
    );
    // Capped, so a million-element array is not a table of contents.
    let big = format!("[{}]", vec!["1"; 5_000].join(","));
    let mut s = src(&big);
    assert_eq!(s.toc().len(), 1000);
    assert_eq!(s.toc()[999], "[999]");
}

#[test]
fn a_missing_file_is_an_error_not_a_panic() {
    assert!(JsonSource::open(std::path::Path::new("/definitely/not/here.json")).is_err());
}




/// A tree row is one node, already its own unit. Making `j` jump top-level
/// members would make a nested document unreadable — that is what `Tab` is for.
#[test]
fn a_tree_does_not_read_in_blocks() {
    let mut s = src(DOC);
    let _ = s.lines(0..s.len());
    assert!(!s.blocks());
    assert_eq!(s.block_at(0), None);
}
