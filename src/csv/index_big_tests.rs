//! The parts of the [`RowIndex`] contract that need big fixtures: laziness on
//! open, progress and interruption during a forced scan, and a file over 100MB
//! generated at run time to prove open time does not scale with size. Split
//! out of `index_tests.rs` to keep both files under the size limit; the
//! fixture helpers come from the parent module.
#![deny(unsafe_code)]

use std::io::Write;
use std::time::Instant;

use super::*;
use crate::csv::read::MAX_ROW_BYTES;

// -- laziness, progress, interruption ---------------------------------------

#[test]
fn opening_indexes_nothing() {
    let body = gen(50_000, 0);
    assert!(body.len() > 4 * WINDOW);
    let t = tmp("lazy", &body);
    let s = store(&t);
    assert_eq!(s.known(), 0);
    assert!(!s.complete());
    assert_eq!(s.progress().bytes, 0);
    assert!(s.reader.reads() <= 2, "opening cost {} reads", s.reader.reads());
}

#[test]
fn painting_a_screen_indexes_a_screen() {
    let body = gen(50_000, 0);
    let t = tmp("screen", &body);
    let mut s = store(&t);
    assert!(s.ensure(50) >= 50);
    let p = s.progress();
    assert!(!p.complete);
    assert!(p.bytes <= WINDOW as u64, "indexed {} bytes for 50 rows", p.bytes);
    assert!(p.bytes * 4 < body.len() as u64);
}

#[test]
fn a_forced_scan_reports_progress_and_can_be_interrupted() {
    let body = gen(300_000, 0);
    let t = tmp("interrupt", &body);
    let mut s = store(&t);
    let mut seen: Vec<Progress> = Vec::new();
    // Give up at the first tick, as `G` does when a key arrives.
    let p = s.scan_all(&mut |p| {
        seen.push(p);
        true
    });
    assert_eq!(seen.len(), 1, "a long scan must report before it finishes");
    assert!(!p.complete, "the scan was cancelled, so it is not complete");
    assert!(p.rows > 0);
    assert!(p.percent() < 100);
    let partial = p.rows;
    // Resuming picks up where it stopped and finishes.
    let done = s.scan_all(&mut |_| false);
    assert!(done.complete);
    assert!(done.rows > partial);
    assert_eq!(done.percent(), 100);
    assert_eq!(Some(done.rows), s.index.total());
    assert_eq!(done.rows, reference(&body).len());
}

#[test]
fn ten_thousand_rows_read_back_exactly() {
    let mut body = Vec::new();
    for i in 0..10_000 {
        body.extend_from_slice(format!("{i},value {i},\"quoted\n{i}\"\n").as_bytes());
    }
    let t = tmp("10k", &body);
    let mut s = store(&t);
    assert_eq!(s.ensure(usize::MAX), 10_000);
    assert!(s.complete());
    for i in [0usize, 1, 999, 5_000, 9_999] {
        let want = format!("{i},value {i},\"quoted\n{i}\"");
        assert_eq!(row(&mut s, i).as_deref(), Some(want.as_str()), "row {i}");
    }
    assert_eq!(row(&mut s, 10_000), None);
}

#[test]
fn random_access_is_two_reads_not_a_rescan() {
    let mut body = Vec::new();
    for i in 0..10_000 {
        body.extend_from_slice(format!("{i},{i},{i}\n").as_bytes());
    }
    let t = tmp("random", &body);
    let mut s = store(&t);
    s.scan_all(&mut |_| false);
    let before = s.reader.reads();
    assert_eq!(row(&mut s, 9_999).as_deref(), Some("9999,9999,9999"));
    assert!(s.reader.reads() - before <= 1, "a row cost more than one read");
    let before = s.reader.reads();
    for i in 9_000..9_100 {
        assert!(row(&mut s, i).is_some());
    }
    assert_eq!(s.reader.reads(), before, "neighbouring rows left the window");
}

// -- files that move under us -----------------------------------------------

#[test]
fn a_file_appended_to_grows_more_rows() {
    let t = tmp("append", b"a\nb\n");
    let mut s = store(&t);
    assert_eq!(s.ensure(usize::MAX), 2);
    crate::csv::read::tests::append(&t.path, b"c\nd\n");
    assert!(s.refresh());
    assert_eq!(s.ensure(usize::MAX), 4);
    assert_eq!(row(&mut s, 3).as_deref(), Some("d"));
}

#[test]
fn appending_to_an_unterminated_last_row_extends_that_row() {
    let t = tmp("append-open", b"a\nbb");
    let mut s = store(&t);
    assert_eq!(s.ensure(usize::MAX), 2);
    assert_eq!(row(&mut s, 1).as_deref(), Some("bb"));
    crate::csv::read::tests::append(&t.path, b"cc\ndd\n");
    assert!(s.refresh());
    assert_eq!(s.ensure(usize::MAX), 3);
    assert_eq!(row(&mut s, 1).as_deref(), Some("bbcc"));
    assert_eq!(row(&mut s, 2).as_deref(), Some("dd"));
}

#[test]
fn a_file_truncated_under_us_drops_its_lost_rows() {
    let t = tmp("shrink", b"aaa\nbbb\nccc\nddd\n");
    let mut s = store(&t);
    assert_eq!(s.ensure(usize::MAX), 4);
    std::fs::write(&t.path, b"aaa\nbb").expect("truncate");
    assert!(s.refresh());
    assert_eq!(s.ensure(usize::MAX), 2);
    assert_eq!(row(&mut s, 0).as_deref(), Some("aaa"));
    assert_eq!(row(&mut s, 1).as_deref(), Some("bb"));
    assert_eq!(row(&mut s, 2), None);
}

#[test]
fn truncating_to_nothing_leaves_no_rows() {
    let t = tmp("shrink-all", b"aaa\nbbb\n");
    let mut s = store(&t);
    assert_eq!(s.ensure(usize::MAX), 2);
    std::fs::write(&t.path, b"").expect("truncate");
    assert!(s.refresh());
    assert_eq!(s.ensure(usize::MAX), 0);
    assert_eq!(row(&mut s, 0), None);
}

#[test]
fn an_unchanged_file_needs_no_reconciliation() {
    let t = tmp("stable", b"a\nb\n");
    let mut s = store(&t);
    s.ensure(usize::MAX);
    assert!(!s.refresh());
}

// -- offset encoding --------------------------------------------------------

#[test]
fn offsets_survive_block_boundaries() {
    let mut offs = Offsets::default();
    for i in 0..(BLOCK * 3 + 7) {
        offs.push(i as u64 * 37);
    }
    for i in 0..offs.len() {
        assert_eq!(offs.get(i), Some(i as u64 * 37));
    }
    assert_eq!(offs.get(offs.len()), None);
}

#[test]
fn offsets_beyond_a_u32_delta_spill_and_stay_exact() {
    let mut offs = Offsets::default();
    let huge = [0u64, 5, 1 << 33, (1 << 33) + 9, u64::MAX / 2];
    for &o in &huge {
        offs.push(o);
    }
    for (i, &o) in huge.iter().enumerate() {
        assert_eq!(offs.get(i), Some(o), "offset {i}");
    }
}

#[test]
fn truncating_offsets_forgets_the_spill_too() {
    let mut offs = Offsets::default();
    offs.push(0);
    offs.push(1 << 34);
    offs.push((1 << 34) + 1);
    offs.truncate(1);
    assert_eq!(offs.len(), 1);
    assert!(offs.spill.is_empty());
    offs.push(9);
    assert_eq!(offs.get(1), Some(9));
}

// -- the big one ------------------------------------------------------------

/// Resident set size in bytes, when the platform makes it cheap to ask.
fn rss() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

/// Write a >100MB CSV a megabyte at a time — never held in memory whole, so
/// the test measures the reader rather than the fixture builder. Returns the
/// file and its size.
fn huge_fixture() -> (Tmp, u64) {
    let t = tmp_path("huge");
    let mut chunk = Vec::with_capacity(1 << 20);
    let mut i = 0u64;
    while chunk.len() < (1 << 20) {
        chunk.extend_from_slice(format!("{i},name {i},\"quoted\nvalue {i}\",x\n").as_bytes());
        i += 1;
    }
    {
        let mut f = std::fs::File::create(&t.path).expect("create huge fixture");
        for _ in 0..110 {
            f.write_all(&chunk).expect("write huge fixture");
        }
        f.flush().expect("flush huge fixture");
    }
    let size = std::fs::metadata(&t.path).expect("stat").len();
    assert!(size > 100 * 1024 * 1024, "fixture is only {size} bytes");
    (t, size)
}

#[test]
fn a_hundred_megabyte_file_opens_instantly() {
    let (t, size) = huge_fixture();
    let before = rss();
    let t0 = Instant::now();
    let mut s = store(&t);
    let open = t0.elapsed();
    assert_eq!(s.known(), 0);
    assert!(open.as_millis() < 250, "opening {size} bytes took {open:?}");

    let t1 = Instant::now();
    assert!(s.ensure(60) >= 60);
    let first_screen = t1.elapsed();
    assert!(first_screen.as_millis() < 250, "first screen took {first_screen:?}");
    assert!(s.progress().bytes <= WINDOW as u64);
    assert!(row(&mut s, 0).is_some());

    // A bounded slice of the scan, as a frame's worth of background indexing.
    let t2 = Instant::now();
    let did = s.index.ensure_bytes(8 << 20, &mut s.reader);
    let slice = t2.elapsed();
    assert!(did >= 8 << 20);
    assert!(!s.complete(), "8MB of a 100MB file is not the whole file");

    let grew = match (before, rss()) {
        (Some(a), Some(b)) => b.saturating_sub(a),
        _ => 0,
    };
    eprintln!(
        "100MB fixture: size {size}B, open {open:?}, first screen {first_screen:?}, \
         {} rows indexed in {slice:?} for {did}B, rss +{grew}B",
        s.known()
    );
    // Nothing may hold the file: the index is offsets and the reader is one
    // window, so a 115MB file costs ~4 bytes per indexed row. The bound is
    // deliberately loose — `rss()` is process-wide and the other tests in this
    // binary are allocating on their own threads at the same time — but it is
    // still two orders of magnitude below "the file is in memory".
    let budget = 64 << 20;
    if grew > 0 {
        assert!(grew < budget, "rss grew {grew}B, budget {budget}B");
    }
}

// -- a file whose tail holds no terminator ----------------------------------

/// A big file holding exactly one row: no `LF`, no `CR`, anywhere. A `.bin`, a
/// `.zip`, a minified bundle — since SPEC.md §Plain text made every unknown
/// extension a text file, this is a shape the reader now opens routinely.
///
/// Written a megabyte at a time so the test measures the reader and not the
/// fixture builder.
fn no_terminator_fixture(name: &str) -> (Tmp, u64) {
    let t = tmp_path(name);
    let chunk = vec![b'x'; 1 << 20];
    {
        let mut f = std::fs::File::create(&t.path).expect("create fixture");
        for _ in 0..64 {
            f.write_all(&chunk).expect("write fixture");
        }
        f.flush().expect("flush fixture");
    }
    let size = std::fs::metadata(&t.path).expect("stat").len();
    assert_eq!(size, 64 << 20);
    (t, size)
}

/// Regression, SPEC.md §CSV — inherited verbatim by SPEC.md §Plain text: "a
/// multi-GB file must open instantly and quit instantly; nothing may read the
/// whole file on the open path, and `q` must never wait on a scan."
///
/// Every *budgeted* entry point obeyed that. [`RowStore::row`] did not: it called
/// the unbounded `RowIndex::ensure(i + 2)`, and to paint the last known row the
/// index has to prove row `i + 1` does not exist — which on a file with no
/// terminator in its tail is a scan to end-of-file, in one call, on the paint
/// path. The first frame was never flushed and the keystroke queue was never
/// read: measured through a real pty on a 2GB `.bin`, the alt-screen bytes
/// appeared at 3ms, `q` was written at 2s and ignored, and the first and only
/// frame arrived 17.3s later having read all 2GiB.
///
/// The fix is that a row is clipped at [`MAX_ROW_BYTES`] anyway, so its end only
/// has to be looked for within that budget.
#[test]
fn painting_a_row_never_scans_a_file_that_holds_no_terminator() {
    let (t, size) = no_terminator_fixture("nolf");
    let mut s = RowStore::lines(Reader::open(&t.path).expect("open fixture"));
    let t0 = Instant::now();
    let got = s.row(0).expect("row 0 is paintable");
    let took = t0.elapsed();

    let read = s.progress().bytes;
    assert!(!s.complete(), "the file must not have been indexed to its end");
    assert_eq!(s.known(), 1, "one row, and it is the one we asked for");
    assert!(read <= MAX_ROW_BYTES as u64, "painting one row read {read} bytes");
    assert!(read * 8 < size, "read {read} of {size} bytes");
    // Clipped and flagged, exactly as a genuinely megabyte-long row is.
    assert!(got.truncated, "an unsettled row is a clipped row");
    assert_eq!(got.data.len(), MAX_ROW_BYTES);
    assert!(got.data.iter().all(|&b| b == b'x'), "and it is the file's bytes");
    // Asking again spends at most one more budget: the index kept what it
    // scanned, and no call can turn into a full scan however often it is made.
    let again = s.row(0).expect("row 0 again");
    assert_eq!(again.data.len(), MAX_ROW_BYTES);
    assert!(s.progress().bytes <= read + MAX_ROW_BYTES as u64);
    eprintln!("{size}B with no terminator: painted row 0 in {took:?}, read {read}B");
}

/// The same file through the CSV grammar rather than the line grammar: the
/// defect and the fix both live in the shared access layer, so neither format
/// gets its own answer.
#[test]
fn the_same_bound_applies_to_the_csv_grammar() {
    let (t, size) = no_terminator_fixture("nolf-csv");
    let mut s = store(&t);
    assert!(s.row(0).expect("row 0").truncated);
    assert!(!s.complete());
    assert!(s.progress().bytes * 8 < size);
    // And row 1 does not exist, which must also be answerable without a scan:
    // the budget is spent, no boundary was found, so there is no row 1 to serve.
    let before = s.progress().bytes;
    assert!(s.row(1).is_none(), "there is no second row");
    assert!(
        s.progress().bytes <= before + MAX_ROW_BYTES as u64,
        "asking for a row past the end must not scan the file either"
    );
    assert!(!s.complete());
}

/// The bound must not make a *normal* file lie. Every row of a file whose rows
/// are ordinary comes back settled — terminator stripped, `truncated` clear —
/// which is what says the clipped path is only ever reached by a row that really
/// is longer than the cap.
#[test]
fn ordinary_rows_are_never_reported_as_clipped() {
    let body = gen(5_000, 0);
    let t = tmp("settled", &body);
    let mut s = store(&t);
    for i in 0..5_000 {
        let sp = s.row(i).unwrap_or_else(|| panic!("row {i} must exist"));
        assert!(!sp.truncated, "row {i} was reported clipped");
        assert!(!sp.data.ends_with(b"\n"), "row {i} kept its terminator");
    }
    assert!(s.row(5_000).is_none(), "and there is no row past the last");
}
