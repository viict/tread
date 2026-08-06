//! [`Reader`] unit tests: the window, the syscall budget, and files that move
//! under us. Fixtures are generated into the temp dir at run time and deleted
//! on drop — nothing large is ever checked in.
#![deny(unsafe_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::*;

/// A temp file that removes itself.
pub struct Tmp {
    pub path: PathBuf,
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Write `body` to a uniquely named temp file.
pub fn tmp(name: &str, body: &[u8]) -> Tmp {
    let t = tmp_path(name);
    std::fs::write(&t.path, body).expect("write fixture");
    t
}

/// A unique temp path with nothing written to it yet.
pub fn tmp_path(name: &str) -> Tmp {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-csv-{}-{nanos}-{name}.csv", std::process::id()));
    Tmp { path: p }
}

/// Append to an existing fixture, as a writer tailing a log would.
pub fn append(path: &Path, body: &[u8]) {
    let mut f = OpenOptions::new().append(true).open(path).expect("append");
    f.write_all(body).expect("append bytes");
}

fn reader(t: &Tmp) -> Reader {
    Reader::open(&t.path).expect("open fixture")
}

#[test]
fn opening_reads_nothing() {
    let t = tmp("open", b"a,b\n1,2\n");
    let r = reader(&t);
    assert_eq!(r.reads(), 0);
    assert_eq!(r.size(), 8);
}

#[test]
fn missing_file_is_an_error_not_a_panic() {
    let t = tmp_path("gone");
    assert!(Reader::open(&t.path).is_err());
}

#[test]
fn chunk_serves_the_window_without_more_syscalls() {
    let body: Vec<u8> = (0..4000).flat_map(|i| format!("row{i},x,y\n").into_bytes()).collect();
    let t = tmp("window", &body);
    let mut r = reader(&t);
    assert_eq!(r.chunk(0, 4), b"row0");
    let first = r.reads();
    assert!(first > 0);
    // Walking the same region again, in row-sized bites, is free: the whole
    // point is that a scroll is not one syscall per row.
    for at in (0..body.len().min(WINDOW - 32) as u64).step_by(9) {
        let _ = r.chunk(at, 9);
    }
    assert_eq!(r.reads(), first, "re-reading inside the window hit the disk");
}

#[test]
fn sequential_scroll_is_a_syscall_per_window_not_per_row() {
    // 40k short rows, ~360 KiB: more than one window, far more than one screen.
    let body: Vec<u8> =
        (0..40_000).flat_map(|i| format!("{:04},x\n", i % 10_000).into_bytes()).collect();
    let t = tmp("scroll", &body);
    let mut r = reader(&t);
    let mut at = 0u64;
    let mut rows = 0;
    while at < body.len() as u64 {
        let got = r.bytes(at, at + 7);
        assert_eq!(got.data.len(), 7);
        at += 7;
        rows += 1;
    }
    assert_eq!(rows, 40_000);
    // Reads are bounded by windows touched, not by rows read.
    let windows = body.len().div_ceil(WINDOW) as u64;
    assert!(r.reads() <= windows + 2, "{} reads for {windows} windows", r.reads());
}

#[test]
fn reading_past_the_end_is_empty_not_a_panic() {
    let t = tmp("past", b"abc\n");
    let mut r = reader(&t);
    assert_eq!(r.chunk(99, 10), b"");
    assert_eq!(r.bytes(99, 200), Span { data: Vec::new(), truncated: true });
    assert_eq!(r.bytes(4, 4), Span { data: Vec::new(), truncated: false });
    assert_eq!(r.bytes(9, 3), Span { data: Vec::new(), truncated: false });
}

#[test]
fn short_read_at_eof_reports_truncated() {
    let t = tmp("short", b"abc");
    let mut r = reader(&t);
    let got = r.bytes(0, 10);
    assert_eq!(got.data, b"abc");
    assert!(got.truncated);
}

#[test]
fn a_huge_row_is_clipped_not_allocated() {
    let mut body = vec![b'x'; MAX_ROW_BYTES + 4096];
    body.push(b'\n');
    let t = tmp("huge", &body);
    let mut r = reader(&t);
    let got = r.bytes(0, body.len() as u64);
    assert_eq!(got.data.len(), MAX_ROW_BYTES);
    assert!(got.truncated);
}

#[test]
fn a_row_larger_than_the_window_bypasses_it() {
    let body = vec![b'y'; WINDOW * 2];
    let t = tmp("big-row", &body);
    let mut r = reader(&t);
    let _ = r.chunk(0, 16); // park the window at the start
    let got = r.bytes(0, (WINDOW as u64) * 2);
    assert_eq!(got.data.len(), WINDOW * 2);
    assert!(!got.truncated);
    // The window still serves its old region.
    assert_eq!(r.chunk(0, 4), b"yyyy");
}

#[test]
fn growth_is_picked_up() {
    let t = tmp("grow", b"abc\n");
    let mut r = reader(&t);
    assert_eq!(r.chunk(0, 4), b"abc\n");
    assert_eq!(r.bytes(0, 8).data, b"abc\n");
    append(&t.path, b"def\n");
    assert_eq!(r.refresh_size(), 8);
    assert_eq!(r.bytes(0, 8).data, b"abc\ndef\n");
    assert_eq!(r.chunk(4, 4), b"def\n");
}

#[test]
fn truncation_is_not_a_panic() {
    let t = tmp("shrink", b"aaaa\nbbbb\ncccc\n");
    let mut r = reader(&t);
    assert_eq!(r.chunk(10, 5), b"cccc\n");
    std::fs::write(&t.path, b"aa\n").expect("truncate");
    r.refresh_size();
    let got = r.bytes(10, 15);
    assert!(got.data.is_empty());
    assert!(got.truncated);
    assert_eq!(r.bytes(0, 3).data, b"aa\n");
}

#[test]
fn an_empty_file_reads_empty() {
    let t = tmp("empty", b"");
    let mut r = reader(&t);
    assert_eq!(r.size(), 0);
    assert_eq!(r.chunk(0, 16), b"");
    assert!(r.bytes(0, 16).data.is_empty());
}
