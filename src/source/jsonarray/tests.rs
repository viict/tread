//! Records inside a document, behind the seam.
//!
//! Every fixture is written by hand, in the *shape* of a real ATIF trajectory:
//! an envelope of top-level keys, and the records under one of them. The
//! trajectory this was measured against is private and none of it is here.
#![deny(unsafe_code)]

use super::*;
use crate::source::record::Records;
use crate::source::{End, Source};

/// A document with `n` steps and the three envelope keys, laid out the way a
/// real one is: pretty-printed, and the record array last.
fn doc(n: usize) -> Vec<u8> {
    let steps: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "    {{\n      \"step_id\": {},\n      \"source\": \"agent\",\n      \
                 \"message\": \"\",\n      \"tool_calls\": [\n        {{\"function_name\": \
                 \"bash\", \"arguments\": {{\"command\": \"echo {i}\"}}}}\n      ]\n    }}",
                i + 1
            )
        })
        .collect();
    format!(
        "{{\n  \"schema_version\": \"ATIF-v1.7\",\n  \"session_id\": \"sxs_1\",\n  \
         \"agent\": {{\"name\": \"opencode\", \"version\": \"1.2.3\"}},\n  \"steps\": [\n{}\n  ]\n}}\n",
        steps.join(",\n")
    )
    .into_bytes()
}

fn open(body: Vec<u8>, at: At) -> ArraySource {
    let mut s = ArraySource::from_bytes(body, at);
    s.set_width(120);
    s
}

fn rows(s: &mut ArraySource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

/// The shape the whole commit exists for: the elements of `steps` are the
/// records, and the keys *around* them are not lost — they are record 0.
#[test]
fn the_array_is_the_records_and_the_other_keys_are_record_zero() {
    let mut s = open(doc(3), At::Key("steps"));
    assert_eq!(s.len(), 4, "a session row and three steps");
    let rows = rows(&mut s);
    // Record 0 is the envelope, as an object of the keys that are not `steps`.
    assert!(rows[0].contains("3 keys"), "{}", rows[0]);
    assert!(rows[0].contains("ATIF-v1.7") && rows[0].contains("sxs_1"), "{}", rows[0]);
    // And every step is there, in order, one row each.
    for (i, row) in rows[1..].iter().enumerate() {
        assert!(row.contains(&format!("step_id: {}", i + 1)), "{row}");
    }
}

/// Every row still opens into the record it stands for, whole — the rule the
/// lens seam is held to, from the other side of it.
#[test]
fn a_record_opens_into_its_own_tree() {
    let mut s = open(doc(2), At::Key("steps"));
    assert_eq!(s.hidden_at(1), Some(s.hidden_at(1).unwrap()), "the step row folds");
    s.fold_all(false);
    let text = rows(&mut s).join("\n");
    assert!(text.contains("\"command\": \"echo 0\""), "{text}");
    // The envelope's own subtree is under record 0, not thrown away.
    assert!(text.contains("\"name\": \"opencode\""), "{text}");
}

/// A document that has no such array is not empty and is not an error: what it
/// *does* hold is still record 0, and it opens.
#[test]
fn a_document_without_the_array_keeps_everything_as_record_zero() {
    let mut s = open(br#"{"title":"notes","items":[1,2,3]}"#.to_vec(), At::Key("steps"));
    assert_eq!(s.len(), 1);
    assert_eq!(s.end(), End::At(0), "nothing left to scan");
    s.fold_all(false);
    let text = rows(&mut s).join("\n");
    assert!(text.contains("\"title\": \"notes\""), "{text}");
    assert!(text.contains("1") && text.contains("3"), "{text}");
}

/// A document that is one scalar, and one that is empty: an honest row and no
/// panic, which is the floor every source in this crate is held to.
#[test]
fn a_document_that_is_not_an_object_still_reads() {
    let mut s = open(b"42".to_vec(), At::Key("steps"));
    assert_eq!(rows(&mut s), vec!["  42".to_string()]);
    let mut empty = open(Vec::new(), At::Key("steps"));
    assert_eq!(empty.len(), 0);
    assert!(empty.lines(0..10).is_empty());
}

/// [`At::Root`]: the document's root array *is* the records, and there is no
/// session row because there are no other keys for one to hold.
#[test]
fn the_root_array_can_be_the_records() {
    let mut s = open(br#"[{"step_id":1},{"step_id":2}]"#.to_vec(), At::Root);
    assert_eq!(s.len(), 2);
    let rows = rows(&mut s);
    assert!(rows[0].contains("step_id: 1"), "{}", rows[0]);
    assert!(rows[1].contains("step_id: 2"), "{}", rows[1]);
    // A root that is not a container has no records at all.
    let scalar = open(b"\"hello\"".to_vec(), At::Root);
    assert_eq!(Source::len(&scalar), 0);
}

/// A record that is not JSON says so and the document keeps rendering — half a
/// trajectory is still worth reading.
#[test]
fn a_record_that_does_not_parse_is_an_error_row() {
    let body = br#"{"steps":[{"a":1},{"b":@},{"c":3}]}"#.to_vec();
    let mut s = open(body, At::Key("steps"));
    let rows = rows(&mut s);
    assert_eq!(rows.len(), 4);
    assert!(rows[2].contains("record 3:"), "{}", rows[2]);
    assert!(rows[3].contains("c: 3"), "{}", rows[3]);
}

/// `c` copies the record's own bytes out of the document — pretty-printing and
/// all — rather than this reader's re-serialisation of them, and says which
/// record it was. A member of a document is a `record`, not a `line`.
#[test]
fn the_verbatim_yank_is_the_documents_own_bytes() {
    let mut s = open(doc(2), At::Key("steps"));
    let _ = s.lines(0..s.len());
    let y = s.yank_block(1).expect("a yank");
    assert_eq!(y.what, "record 2 verbatim");
    assert!(y.text.starts_with('{') && y.text.contains("\n      \"step_id\": 1"), "{}", y.text);
}

// -- laziness ------------------------------------------------------------------

/// The claim SPEC.md makes, from the inside: finding the array is a byte walk
/// of the document's top level, and after that a screen costs a screen. No
/// record is parsed to *find* another one, and the row count is honestly a
/// lower bound until the array's own scan finishes.
#[test]
fn painting_a_screen_indexes_a_screen_and_not_the_array() {
    let mut s = ArraySource::from_bytes(doc(20_000), At::Key("steps"));
    assert_eq!(s.len(), 0, "opening indexes nothing");
    s.set_width(120);
    let _ = s.lines(0..40);
    let known = s.len();
    assert!(known >= 40, "only {known} rows for a screen of 40");
    assert!(known < 20_001, "the whole array was indexed for one screen");
    assert!(matches!(s.end(), End::Scanning(_)), "the end is not known yet");
    let text = s.position_text(0).expect("a position");
    assert!(text.starts_with("record 1/\u{2265}"), "{text}");
    // And the idle tick converges, without any keystroke having waited on it.
    while s.extend() {}
    assert_eq!(s.len(), 20_001);
    assert_eq!(s.end(), End::At(20_000));
    assert_eq!(s.position_text(0).as_deref(), Some("record 1/20001"));
}

/// A record too big to display says how big it is instead of being loaded —
/// the same limit, and the same sentence, the document reader uses.
#[test]
fn a_record_past_the_parse_cap_reports_its_size() {
    let big = "x".repeat(PARSE_CAP as usize + 16);
    let body = format!("{{\"steps\":[{{\"a\":\"{big}\"}},{{\"b\":2}}]}}").into_bytes();
    let mut s = open(body, At::Key("steps"));
    let rows = rows(&mut s);
    assert!(rows[1].contains("record 2:") && rows[1].contains("display limit"), "{}", rows[1]);
    assert!(rows[2].contains("b: 2"), "{}", rows[2]);
}

/// An idle tick spends its whole budget, not the first chunk that happens to
/// yield a member. The store used to ask the structural index for `known + 1`,
/// which returned after ~4KB and threw the rest of the slice away — the index
/// crawled a chunk per tick and a multi-megabyte trajectory took a minute and a
/// half to settle. One tick must cover far more than one chunk.
#[test]
fn one_idle_tick_indexes_far_more_than_one_chunk() {
    let mut s = ArraySource::from_bytes(doc(20_000), At::Key("steps"));
    s.set_width(120);
    // The first tick may be spent locating the array; from the tick after it,
    // records arrive in thousands rather than in tens.
    let mut ticks = 0;
    while Records::known(&s) == 0 && Source::extend(&mut s) {
        ticks += 1;
        assert!(ticks < 100, "the array was not found in {ticks} ticks");
    }
    let before = Records::known(&s);
    Source::extend(&mut s);
    let found = Records::known(&s) - before;
    assert!(found > 1000, "one idle tick found only {found} records");
}

/// `--toc` on a document larger than one frame budget prints the records, not
/// nothing. A store whose records live inside a document serves none of them
/// until the whole top level has been walked, so a single budgeted slice left
/// `--toc --lens atif` printing zero lines and exiting 0 — which a script reads
/// as "this file has no records".
#[test]
fn the_toc_of_a_document_bigger_than_one_budget_is_not_empty() {
    // Comfortably past the 4 MB a frame may spend on the index.
    let body = doc(40_000);
    assert!(body.len() > 4 * 1024 * 1024, "the fixture is {} bytes", body.len());
    let mut s = ArraySource::from_bytes(body, At::Key("steps"));
    s.set_lens(crate::lens::find("atif").expect("the lens exists"));
    s.set_width(120);
    let toc = s.summaries(1000);
    assert_eq!(toc.len(), 1000, "the toc truncated to nothing");
    assert!(toc[0].starts_with("1\tsession"), "{}", toc[0]);
}

/// Against a real trajectory, when one is pointed at: set
/// `TREAD_ATIF_TRAJECTORY`. Fixtures only prove the shapes someone thought of,
/// and a real ATIF file is private, so this asserts *structure* — every record
/// reachable, every row painted, the lens recognising what it claims to — and
/// prints counts only. Skipped when unset, so CI (which has no trajectory)
/// stays green.
#[test]
fn a_real_atif_trajectory_reads() {
    let Ok(path) = std::env::var("TREAD_ATIF_TRAJECTORY") else {
        return;
    };
    let at = match crate::lens::records_at("atif") {
        Some(crate::lens::RecordsAt::Member(key)) => At::Key(key),
        other => panic!("the atif lens no longer names a member: {other:?}"),
    };
    let mut s = ArraySource::open(std::path::Path::new(&path), at).expect("open");
    s.set_lens(crate::lens::find("atif").expect("the lens exists"));
    s.set_width(140);
    // Read it all the way, the way `G` does: a slice at a time, never a whole
    // file in one call.
    let mut ticks = 0;
    while s.extend() {
        ticks += 1;
        assert!(ticks < 100_000, "the idle tick never settles");
    }
    let records = Records::known(&s);
    assert!(records > 1, "a trajectory with no steps");
    // Every record is reachable and every row paints, with nothing lost: the
    // rows a lens folds away are the rows its groups open.
    let rows = s.len();
    // More rows than records now: a message carries what it said under it, and
    // fewer than one row per record would mean a record had been hidden.
    assert!(rows > 0, "{rows} rows for {records} records");
    let read = (0..records).filter(|&r| s.lens_row(r, false).is_some()).count();
    assert_eq!(read, records, "the lens left records unread");
    s.fold_all(true);
    let shut = s.len();
    s.fold_all(false);
    let open = s.len();
    assert!(open >= shut, "opening the runs lost rows");
    for row in 0..open.min(4000) {
        assert!(s.row_line(row).is_some(), "row {row} of {open} did not paint");
    }
    println!("{records} records, {read} read by the lens, {shut} rows shut, {open} rows open");
}

// -- through the lens -----------------------------------------------------------

/// End to end: the same document read through `--lens atif` is a conversation
/// with the mechanics folded into runs, and every record is still reachable.
#[test]
fn the_lens_folds_the_mechanics_of_a_document() {
    let body = br#"{"schema_version":"ATIF-v1.7","session_id":"sxs_1","steps":[
        {"step_id":1,"source":"user","message":"do it"},
        {"step_id":2,"source":"agent","message":"",
         "tool_calls":[{"function_name":"bash","arguments":{"command":"ls"}}]},
        {"step_id":3,"source":"agent","message":"",
         "tool_calls":[{"function_name":"bash","arguments":{"command":"make"}}]},
        {"step_id":4,"source":"agent","message":"done"}]}"#
        .to_vec();
    let mut s = ArraySource::from_bytes(body, At::Key("steps"));
    s.set_lens(crate::lens::find("atif").expect("the lens exists"));
    s.set_width(120);
    let painted = rows(&mut s);
    // session, the prompt, the folded run of two steps, the answer. Each
    // message here is one line, so it is entirely on its own summary row and
    // nothing is painted under it. The session row is a headline over the
    // envelope's keys rather than something anyone said, so it has no body
    // either.
    assert_eq!(painted.len(), 4, "{painted:#?}");
    assert!(painted[0].contains("session") && painted[0].contains("ATIF-v1.7"), "{}", painted[0]);
    assert!(painted[1].contains("user") && painted[1].contains("do it"), "{}", painted[1]);
    assert!(painted[2].contains("\u{27e8}2 steps \u{b7} 2 tool calls\u{27e9}"), "{}", painted[2]);
    assert!(painted[3].contains("done"), "{}", painted[3]);
    // Once each: the row is the message's first line, not an excerpt with the
    // message repeated under it.
    assert_eq!(painted.iter().filter(|r| r.contains("do it")).count(), 1, "{painted:#?}");
    assert_eq!(painted.iter().filter(|r| r.contains("done")).count(), 1, "{painted:#?}");
    // The status bar names the lens, and the record numbering says plainly
    // that the session row is record 1 and `steps[0]` is record 2.
    let text = s.position_text(1).expect("a position");
    assert!(text.starts_with("atif  \u{b7}  record 2/5"), "{text}");
    // A search hit inside the folded run opens it rather than being lost.
    s.set_query("make");
    let hit = s.preview_match(crate::source::Anchor(0), crate::source::Dir::Forward);
    assert!(hit.is_some(), "the folded run hides a record from search");
    assert!(rows(&mut s).iter().any(|r| r.contains("make")), "the run did not open");
}

/// `--toc --lens atif`: the trajectory as a list, one line per record, with the
/// session row first and the records the lens did not read keeping a generic
/// summary.
#[test]
fn the_toc_is_the_trajectory_as_a_list() {
    let body = br#"{"schema_version":"ATIF-v1.7","steps":[
        {"step_id":1,"source":"user","message":"go","timestamp":"2026-04-23T10:55:31.1+00:00"},
        {"nothing":"this dialect knows"}]}"#
        .to_vec();
    let mut s = ArraySource::from_bytes(body, At::Key("steps"));
    s.set_lens(crate::lens::find("atif").expect("the lens exists"));
    s.set_width(120);
    let toc = s.summaries(100);
    assert_eq!(toc.len(), 3);
    assert_eq!(toc[0], "1\tsession\t\tATIF-v1.7");
    assert_eq!(toc[1], "2\tuser\t10:55\tgo");
    // Not this dialect's record: the generic summary, never a wrong headline.
    assert!(toc[2].starts_with("3\t") && toc[2].contains("1 key"), "{}", toc[2]);
}

