//! Golden renders driven through the real `mdr` binary.
//!
//! Integration tests cannot reach into a binary crate, so these drive the
//! built executable (`CARGO_BIN_EXE_mdr`) over a temporary fixture. Per
//! SPEC.md §Testing every assertion is either on the ANSI-stripped text or on
//! a specific style span, never on a whole escaped line, so a palette tweak
//! cannot break the layout assertions.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const DOC: &str = "\
# Hi

Some **bold** text with a [link](models/SAMPLE_MODEL.md) and `code`.

- one
  - two
- [x] done

| left | right |
| :--- | ----: |
| a | 1 |
| bb | 22 |

```sh
echo hi
```
";

fn fixture(name: &str, body: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("rmarktui-golden-{}-{}.md", std::process::id(), name));
    let mut f = std::fs::File::create(&p).expect("create fixture");
    f.write_all(body.as_bytes()).expect("write fixture");
    p
}

fn run(path: &PathBuf, extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_mdr"))
        .args(extra)
        .arg(path)
        .output()
        .expect("run mdr");
    assert!(out.status.success(), "mdr exited {:?}", out.status);
    String::from_utf8(out.stdout).expect("utf-8 output")
}

/// Remove CSI and OSC sequences, leaving the visible text.
fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match cs.next() {
            Some('[') => {
                for c in cs.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = cs.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        cs.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn layout_is_stable() {
    let p = fixture("layout", DOC);
    // stdout is a pipe here, so colour is off by the NO_COLOR/tty rule.
    let text = run(&p, &["--width", "40"]);
    let lines: Vec<&str> = text.lines().collect();
    assert!(!text.contains('\x1b'), "escapes leaked into a piped render");

    // H1 banner: five glyph rows, the first carrying the collapse marker.
    assert!(lines[0].starts_with("\u{25be} "), "no gutter marker: {:?}", lines[0]);
    assert!(lines[..5].iter().all(|l| l.contains('\u{2588}')), "banner rows");
    assert_eq!(lines[5], "");

    // Paragraph: bold and code lose their markup, the link keeps its text.
    let para = lines[6];
    assert!(para.contains("Some bold text with a link and code"), "{para:?}");

    // Lists: bullets by depth, hanging indent, task marker.
    let bullets: Vec<&&str> = lines.iter().filter(|l| l.contains('\u{2022}') || l.contains('\u{25e6}')).collect();
    assert_eq!(*bullets[0], "  \u{2022} one");
    assert_eq!(*bullets[1], "    \u{25e6} two");
    assert!(lines.iter().any(|l| l.contains("\u{2611} done")), "task marker");

    // Table: box drawing, alignment honoured, every row the same width.
    let table: Vec<&&str> = lines.iter().filter(|l| l.contains('\u{2502}') || l.contains('\u{2500}')).collect();
    assert!(table[0].contains('\u{250c}') && table[0].contains('\u{252c}'));
    assert_eq!(*table[3], "  \u{2502} a    \u{2502}     1 \u{2502}");
    assert_eq!(*table[4], "  \u{2502} bb   \u{2502}    22 \u{2502}");
    let w = table[0].chars().count();
    assert!(table.iter().all(|r| r.chars().count() == w), "ragged table");

    // Code block: language label plus the verbatim body.
    assert!(lines.iter().any(|l| l.trim_end().ends_with("sh")), "language label");
    assert!(lines.iter().any(|l| l.contains("echo hi")), "code body");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn a_piped_render_is_free_of_control_sequences() {
    let p = fixture("styles", DOC);
    // A pipe is not a terminal, so colour, OSC 8 hyperlinks and the viewport
    // clip are all off. Style spans themselves are asserted at the unit level
    // (`render::tests`), which is what keeps palette changes local.
    let out = run(&p, &["--width", "40"]);
    assert_eq!(strip(&out), out, "stripping changed a plain render");
    for bad in ["\x1b", "?1000", "?1002", "?1003", "?1006", "?1015"] {
        assert!(!out.contains(bad), "{bad:?} leaked into a piped render");
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn toc_prints_the_outline_and_exits() {
    let p = fixture("toc", "# A\n\ntext\n\n## B\n\n### C\n");
    let out = run(&p, &["--toc"]);
    assert_eq!(out, "A\n  B\n    C\n");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn unknown_flags_are_usage_errors() {
    let out = Command::new(env!("CARGO_BIN_EXE_mdr"))
        .arg("--definitely-not-a-flag")
        .output()
        .expect("run mdr");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("mdr: unknown option"), "{err:?}");
}
