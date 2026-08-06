//! Format detection and CSV rendering through the real binary
//! (SPEC.md §Multi-format reading, §CSV).
//!
//! The unit tests cover the detector and the source; these cover the *wiring*:
//! that `tread data.csv`, `cat data.csv | tread`, `--format` and `--delim` all
//! reach the CSV path and paint a grid, and that a markdown document is
//! untouched by any of it.

mod harness;

use harness::{render, render_stdin, strip};
use std::path::PathBuf;

const BODY: &str = "id,name,city\n1,alice,berlin\n2,bo,rome\n3,carolina,oslo\n";

/// A temp file with an arbitrary extension (`harness::temp_doc` always makes
/// `.md`, which is exactly what must *not* decide things here).
fn temp(name: &str, ext: &str, body: &[u8]) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-cli-{}-{nanos}-{name}{ext}", std::process::id()));
    std::fs::write(&p, body).expect("write temp doc");
    p
}

struct Doc(PathBuf);

impl Drop for Doc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn grid_of(out: &str) -> Vec<String> {
    strip(out).lines().map(|l| l.to_string()).collect()
}

fn assert_is_grid(out: &str) {
    let rows = grid_of(out);
    assert!(rows[0].starts_with('\u{250c}'), "not a grid: {:?}", rows.first());
    assert!(rows[1].contains("name"), "{:?}", rows[1]);
    assert!(rows[3].contains("alice"), "{:?}", rows[3]);
    assert!(rows.last().unwrap().starts_with('\u{2514}'));
    // Every row of the grid is the same display width.
    let w = rows[0].chars().count();
    for r in &rows {
        assert_eq!(r.chars().count(), w, "{r:?}");
    }
}

#[test]
fn the_extension_selects_csv() {
    let doc = Doc(temp("ext", ".csv", BODY.as_bytes()));
    assert_is_grid(&render(&doc.0, &["--no-alt", "--width", "60"]));
}

#[test]
fn a_tsv_is_read_with_its_own_delimiter() {
    let body = BODY.replace(',', "\t");
    let doc = Doc(temp("tsv", ".tsv", body.as_bytes()));
    assert_is_grid(&render(&doc.0, &["--no-alt", "--width", "60"]));
}

#[test]
fn piped_input_is_sniffed() {
    assert_is_grid(&render_stdin(BODY.as_bytes(), &["--no-alt", "--width", "60"]));
    // Prose on the same pipe is still markdown.
    let md = strip(&render_stdin(
        b"# Title\n\nOne, two, three.\n",
        &["--no-alt", "--width", "60"],
    ));
    assert!(!md.contains('\u{250c}'), "{md}");
    assert!(md.contains("One, two, three."));
}

#[test]
fn the_format_flag_overrides_the_extension_both_ways() {
    let csv_named_md = Doc(temp("as-md", ".md", BODY.as_bytes()));
    // Without the flag it is markdown: a plain paragraph, no grid.
    let as_md = strip(&render(&csv_named_md.0, &["--no-alt", "--width", "60"]));
    assert!(!as_md.contains('\u{250c}'), "{as_md}");
    assert_is_grid(&render(
        &csv_named_md.0,
        &["--no-alt", "--width", "60", "--format", "csv"],
    ));

    let md_named_csv = Doc(temp("as-csv", ".csv", b"# Title\n\nprose here\n"));
    let forced = strip(&render(
        &md_named_csv.0,
        &["--no-alt", "--width", "60", "--format=md"],
    ));
    assert!(!forced.contains('\u{250c}'), "{forced}");
    // An H1 renders as a block-glyph banner, so look for the body text.
    assert!(forced.contains("prose here"), "{forced}");
}

#[test]
fn the_delimiter_can_be_forced() {
    let doc = Doc(temp("semi", ".csv", b"a;b\n1;2\n3;4\n"));
    let sniffed = grid_of(&render(&doc.0, &["--no-alt", "--width", "40"]));
    assert!(sniffed[1].contains('a') && sniffed[1].contains('b'));
    let forced = grid_of(&render(
        &doc.0,
        &["--no-alt", "--width", "40", "--delim", "comma"],
    ));
    // With the wrong delimiter it is one column, not an error.
    assert!(forced[1].contains("a;b"), "{:?}", forced[1]);
}

#[test]
fn toc_on_a_csv_lists_the_columns() {
    let doc = Doc(temp("toc", ".csv", BODY.as_bytes()));
    let out = strip(&render(&doc.0, &["--toc", "--width", "60"]));
    assert_eq!(out, "id\nname\ncity\n", "{out:?}");
}

#[test]
fn a_quoted_field_with_a_comma_and_a_newline_survives_the_grid() {
    let body = "a,b\n\"one,two\",\"line\nbreak\"\n";
    let doc = Doc(temp("quoted", ".csv", body.as_bytes()));
    let rows = grid_of(&render(&doc.0, &["--no-alt", "--width", "60"]));
    // Top border, header, separator, ONE data row, bottom border: the
    // newline inside the quotes is not a row boundary.
    assert_eq!(rows.len(), 5, "one data row, not two: {rows:?}");
    assert!(rows[3].contains("one,two"), "{:?}", rows[3]);
    assert!(rows[3].contains('\u{b7}'), "the newline is shown, not emitted");
}

#[test]
fn a_malformed_csv_renders_rather_than_failing() {
    for body in [
        "a,b\n\"unterminated,1\n2,3\n".to_string(),
        "a,b\r\n1,2\r\n".to_string(),
        "\u{feff}a,b\n1,2\n".to_string(),
        "a,b\n1,2,3,4\n5\n".to_string(),
        format!("a,b\n{},2\n", "x".repeat(100_000)),
    ] {
        let doc = Doc(temp("bad", ".csv", body.as_bytes()));
        let out = render(&doc.0, &["--no-alt", "--width", "60"]);
        let rows = grid_of(&out);
        assert!(rows[0].starts_with('\u{250c}'), "{:?}", rows.first());
        let w = rows[0].chars().count();
        assert!(rows.iter().all(|r| r.chars().count() == w), "{rows:?}");
    }
}

#[test]
fn a_big_csv_dumps_every_row() {
    let mut body = String::from("id,v\n");
    for i in 0..5000 {
        body.push_str(&format!("{i},value{i}\n"));
    }
    let doc = Doc(temp("big", ".csv", body.as_bytes()));
    let rows = grid_of(&render(&doc.0, &["--no-alt", "--width", "60"]));
    // 3 rows of header furniture, 5000 data rows, 1 bottom border.
    assert_eq!(rows.len(), 3 + 5000 + 1);
    assert!(rows[3].contains("value0"));
    // Widths came from the first 1000 rows, where "value999" is the widest, so
    // a later "value4999" is truncated with a marker rather than widening the
    // grid halfway down (SPEC.md §CSV).
    assert!(rows[5002].contains("value49\u{2026}"), "{:?}", rows[5002]);
    let w = rows[3].chars().count();
    assert_eq!(rows[5002].chars().count(), w);
}

/// Run tread expecting it to refuse: returns (exit code, stderr).
fn refuses(args: &[&str], stdin: Option<&[u8]>) -> (i32, String) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tread"));
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = match stdin {
        None => cmd.output().expect("run tread"),
        Some(body) => {
            let mut child = cmd.stdin(Stdio::piped()).spawn().expect("spawn tread");
            child.stdin.take().expect("stdin").write_all(body).expect("write");
            child.wait_with_output().expect("wait tread")
        }
    };
    assert!(out.stdout.is_empty(), "a refused file still painted something");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// UTF-16 is half NUL bytes: rendering it "leniently" is a screen of
/// replacement characters that says nothing. It is refused by name instead,
/// with the conversion to run.
#[test]
fn a_utf16_file_is_refused_by_name_rather_than_rendered_as_mojibake() {
    let text = "id,name\n1,alice\n2,bob\n3,carol\n";
    let mut le: Vec<u8> = vec![0xff, 0xfe];
    let mut be: Vec<u8> = vec![0xfe, 0xff];
    for c in text.encode_utf16() {
        le.extend_from_slice(&c.to_le_bytes());
        be.extend_from_slice(&c.to_be_bytes());
    }
    for (name, body) in [("le", le), ("be", be)] {
        // Named file, extension known...
        let doc = Doc(temp(name, ".csv", &body));
        let (code, err) = refuses(&["--no-alt", "--width", "60", doc.0.to_str().unwrap()], None);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("UTF-16"), "{err}");
        assert!(err.contains("iconv"), "{err}");
        // ... a markdown extension ...
        let md = Doc(temp(name, ".md", &body));
        let (code, err) = refuses(&["--no-alt", md.0.to_str().unwrap()], None);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("UTF-16"), "{err}");
        // ... and stdin, which has no name at all.
        let (code, err) = refuses(&["--no-alt", "-"], Some(&body));
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("UTF-16"), "{err}");
    }
}

#[test]
fn a_utf32_bom_is_refused_and_a_utf8_bom_is_not() {
    let mut u32le = vec![0xff, 0xfe, 0x00, 0x00];
    u32le.extend_from_slice(&[0x69, 0, 0, 0, 0x64, 0, 0, 0]);
    let doc = Doc(temp("u32", ".csv", &u32le));
    let (code, err) = refuses(&["--no-alt", doc.0.to_str().unwrap()], None);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("UTF-32"), "{err}");

    // A UTF-8 BOM is ordinary and still opens (SPEC.md §CSV: "BOM").
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(BODY.as_bytes());
    let ok = Doc(temp("u8bom", ".csv", &bom));
    assert_is_grid(&render(&ok.0, &["--no-alt", "--width", "60"]));
}

/// A character device has no size and no offset, so it cannot be indexed
/// lazily; it takes the same path as piped stdin. The bug this catches is a
/// second `open` of a non-seekable path, which on a fifo never returns.
#[test]
#[cfg(unix)]
fn a_device_path_opens_as_a_stream_and_exits() {
    let dev = PathBuf::from("/dev/null");
    assert_eq!(render(&dev, &["--no-alt", "--width", "60"]), "");
    assert_eq!(render(&dev, &["--no-alt", "--width", "60", "--format", "csv"]), "");
    assert_eq!(render(&dev, &["--toc"]), "");
}
