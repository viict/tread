//! Directory listings, against real temporary directories.
#![deny(unsafe_code)]

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::source::Source;

/// A temp directory that removes itself, with the entries a listing must cope
/// with: subdirectories, several formats, and a dotfile.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Tmp {
    let mut p = std::env::temp_dir();
    p.push(format!("tread-dir-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("mkdir");
    Tmp(p)
}

fn populated(name: &str) -> Tmp {
    let t = tmp(name);
    fs::create_dir(t.0.join("models")).unwrap();
    fs::write(t.0.join("README.md"), "# hi\n").unwrap();
    fs::write(t.0.join("data.csv"), "a,b\n1,2\n").unwrap();
    fs::write(t.0.join("log.txt"), "x\n").unwrap();
    fs::write(t.0.join(".secret"), "shh\n").unwrap();
    t
}

fn text(s: &mut DirSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

#[test]
fn directories_come_first_then_files_case_insensitively() {
    let t = populated("order");
    let mut s = DirSource::open(&t.0);
    let rows = text(&mut s);
    let names: Vec<&String> = rows.iter().filter(|r| r.starts_with("  ")).collect();
    assert!(names[0].contains("models/"), "{names:?}");
    // Then the files, alphabetically, ignoring case.
    let files: Vec<String> = names[1..].iter().map(|r| r.trim().to_string()).collect();
    assert!(files[0].starts_with("data.csv"), "{files:?}");
    assert!(files[1].starts_with("log.txt"), "{files:?}");
    assert!(files[2].starts_with("README.md"), "{files:?}");
}

/// Hiding what exists without saying so would be lying about the directory.
#[test]
fn dotfiles_are_hidden_but_counted_and_a_shows_them() {
    let t = populated("hidden");
    let mut s = DirSource::open(&t.0);
    let rows = text(&mut s).join("\n");
    assert!(rows.contains("1 hidden"), "the header counts it: {rows}");
    assert!(!rows.contains(".secret"), "and it is not listed");
    assert!(rows.contains("press a to show 1 hidden entry"), "{rows}");

    // Through the trait, the way `a` reaches it — the pager never knows this
    // is a directory.
    let msg = s.toggle_hidden().expect("a listing has something to toggle");
    assert!(msg.contains("showing 1 hidden entry"), "{msg}");
    assert!(text(&mut s).join("\n").contains(".secret"));
}

#[test]
fn every_entry_is_a_link_so_the_pager_can_walk_them() {
    let t = populated("links");
    let s = DirSource::open(&t.0);
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    assert_eq!(urls, vec!["models/", "data.csv", "log.txt", "README.md"]);
    // A directory keeps its slash, so the text and the target agree.
    assert!(urls[0].ends_with('/'));
}

#[test]
fn a_file_shows_its_size_and_the_format_tread_would_read_it_as() {
    let t = populated("meta");
    let mut s = DirSource::open(&t.0);
    let rows = text(&mut s).join("\n");
    assert!(rows.contains("csv"), "{rows}");
    assert!(rows.contains("markdown"), "{rows}");
    assert!(rows.contains(" B"), "a size is shown: {rows}");
    // Plain text is the default and saying so on every row is noise.
    let txt = text(&mut s).into_iter().find(|r| r.contains("log.txt")).unwrap();
    assert!(!txt.contains("text"), "{txt}");
}

#[test]
fn an_empty_directory_says_so() {
    let t = tmp("empty");
    let mut s = DirSource::open(&t.0);
    let rows = text(&mut s).join("\n");
    assert!(rows.contains("0 entries"), "{rows}");
    assert!(rows.contains("empty"), "{rows}");
    assert!(s.links().is_empty());
}

/// Following a link into a directory that cannot be read must not lose the
/// document the reader came from.
#[test]
fn an_unreadable_directory_is_a_listing_that_says_why() {
    let mut s = DirSource::open(std::path::Path::new("/definitely/not/here"));
    let rows = text(&mut s).join("\n");
    assert!(rows.contains("cannot be read"), "{rows}");
    assert!(s.links().is_empty());
    assert!(s.len() > 0, "still a document");
}

#[test]
fn sizes_read_the_way_a_person_would_write_them() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(512), "512 B");
    assert_eq!(human(1024), "1.0 KB");
    assert_eq!(human(1536), "1.5 KB");
    assert_eq!(human(20 * 1024), "20 KB");
    assert_eq!(human(5 * 1024 * 1024), "5.0 MB");
}

/// `y` on a row copies the name, not the padded display row.
#[test]
fn yanking_an_entry_gives_its_name() {
    let t = populated("yank");
    let s = DirSource::open(&t.0);
    // Row 0 is the header, row 1 blank, row 2 the first entry.
    assert_eq!(s.yank_point(2).expect("a yank").text, "models/\n");
    assert!(s.yank_point(0).is_none(), "the header is not an entry");
    let all = s.yank_section(2).expect("the listing");
    assert!(all.text.contains("data.csv\n"), "{}", all.text);
}

#[test]
fn searching_a_listing_finds_and_highlights_a_name() {
    let t = populated("search");
    let mut s = DirSource::open(&t.0);
    // A full filename, not a bare word: the header row carries the directory's
    // path and is searchable like any other row, and on Windows a temp path
    // lives under `AppData` — which contains "data". A needle that can appear
    // in the path makes this test depend on where the OS puts temp files.
    s.set_query("data.csv");
    assert_eq!(s.match_count(), 1, "rows: {:?}", text(&mut s));
    let hit = s.cycle_match(Anchor(0), crate::source::search::Dir::Forward).expect("hit");
    // The hit is the entry itself, not the header.
    assert!(s.lines(hit.anchor.0..hit.anchor.0 + 1)[0].text().contains("data.csv"));
    let spans = s.matches_on(hit.anchor.0);
    assert_eq!(spans.len(), 1);
    assert!(spans[0].end > spans[0].start);
}

/// The whole point: `Enter` on a directory entry opens that directory, so a
/// tree can be walked with the navigation the pager already has.
#[test]
fn enter_on_a_directory_entry_opens_that_directory() {
    let t = populated("walk");
    fs::write(t.0.join("models").join("A.md"), "# A\n").unwrap();

    let src = DirSource::open(&t.0);
    let mut p = crate::pager::Pager::new(Box::new(src), "root".into(), 80, 24, Some(80));
    p.attach_nav(crate::nav::Navigator::new(&t.0, Some(&t.0), &t.0));

    // The first entry is `models/`.
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Char('n')));
    assert_eq!(p.link_status().as_deref(), Some("models"), "focused entry");

    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Enter));
    let shown = p.visible_text().join("\n");
    assert!(
        shown.contains("A.md"),
        "descended into models/, showing its listing: {shown}\nmessage={:?}",
        p.message
    );
}
