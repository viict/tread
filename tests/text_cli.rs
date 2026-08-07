//! Plain text through the real binary (SPEC.md §Plain text).
//!
//! The unit tests cover the source; these cover the *wiring*: that a file whose
//! extension names no parser reaches the text path and comes back verbatim,
//! that `--format text` forces it over an extension that would have claimed a
//! parser, and that markdown is untouched by any of it.

mod harness;

use harness::{render, render_stdin, strip, temp_doc_ext};
use std::path::PathBuf;

const SCRIPT: &str = "#!/bin/sh\n# deploy the thing\nset -eu\n\nfor f in *; do\n\techo \"$f\"\ndone\n";

struct Doc(PathBuf);

impl Drop for Doc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lines(out: &str) -> Vec<String> {
    strip(out).lines().map(|l| l.trim_end().to_string()).collect()
}

/// The whole promise: a shell script comes back as a shell script.
#[test]
fn an_unknown_extension_renders_verbatim() {
    let doc = Doc(temp_doc_ext("script", "sh", SCRIPT.as_bytes()));
    let out = lines(&render(&doc.0, &["--no-alt", "--plain", "--width", "80"]));
    assert_eq!(
        out,
        [
            "#!/bin/sh",
            "# deploy the thing",
            "set -eu",
            "",
            "for f in *; do",
            // The tab is expanded to the next 8-column stop, not dotted.
            "        echo \"$f\"",
            "done",
        ],
        "{out:#?}"
    );
}

/// A name the detector does not know at all — no extension — is text too, and
/// nothing sniffs its content to decide otherwise.
#[test]
fn an_extensionless_file_is_text_and_is_not_sniffed() {
    // Content that would sniff as CSV if anything sniffed it.
    let body = "id,name\n1,alice\n2,bo\n3,carol\n";
    let doc = Doc(temp_doc_ext("plainname", "conf", body.as_bytes()));
    let out = lines(&render(&doc.0, &["--no-alt", "--plain", "--width", "80"]));
    assert_eq!(out, ["id,name", "1,alice", "2,bo", "3,carol"], "{out:#?}");
    assert!(!out[0].starts_with('\u{250c}'), "read as a grid: {out:#?}");
}

/// `--format text` forces it for a file whose extension would otherwise claim
/// a parser (SPEC.md §Plain text).
#[test]
fn the_flag_forces_text_over_an_extension() {
    let md = "# Title\n\nSome **bold** prose.\n";
    let doc = Doc(temp_doc_ext("forced", "md", md.as_bytes()));
    let out = lines(&render(&doc.0, &["--no-alt", "--plain", "--width", "80", "--format", "text"]));
    assert_eq!(out, ["# Title", "", "Some **bold** prose."], "{out:#?}");

    // …and without the flag the same file is markdown: a banner, not a `#`.
    let normal = strip(&render(&doc.0, &["--no-alt", "--plain", "--width", "80"]));
    assert!(!normal.contains("# Title"), "{normal}");
    assert!(normal.contains("bold"), "{normal}");
}

/// A `.txt` names the text reader now that there is one; `# TODO` at the top of
/// a notes file is a comment, not a heading.
#[test]
fn a_txt_file_is_text() {
    let doc = Doc(temp_doc_ext("notes", "txt", b"# TODO\n- buy milk\n"));
    let out = lines(&render(&doc.0, &["--no-alt", "--plain", "--width", "80"]));
    assert_eq!(out, ["# TODO", "- buy milk"], "{out:#?}");
}

/// Odd line endings, no trailing newline, control bytes and invalid UTF-8: all
/// render, none escape to the terminal, and the process exits 0.
#[test]
fn hostile_bytes_render_and_exit_cleanly() {
    let mut body = b"crlf\r\ncr\rlf\n\x1b[2Jescape\n".to_vec();
    body.extend_from_slice(b"\xff\xfe not utf8\nno trailing newline");
    let doc = Doc(temp_doc_ext("hostile", "log", &body));
    let raw = render(&doc.0, &["--no-alt", "--plain", "--width", "80"]);
    let out = lines(&raw);
    assert_eq!(out.len(), 6, "{out:#?}");
    assert_eq!(out[0], "crlf");
    assert_eq!(out[1], "cr");
    assert_eq!(out[2], "lf");
    assert_eq!(out[3], "\u{b7}[2Jescape");
    assert!(out[4].contains('\u{fffd}'), "{:?}", out[4]);
    assert_eq!(out[5], "no trailing newline");
}

/// `--toc` on a text file prints nothing and exits 0: it has no outline and
/// will not invent one.
#[test]
fn toc_of_a_text_file_is_empty() {
    let doc = Doc(temp_doc_ext("toc", "sh", SCRIPT.as_bytes()));
    assert_eq!(render(&doc.0, &["--toc"]), "");
}

/// A pipe still gets sniffed — it has no name to read — so piping markdown
/// keeps rendering markdown and the text path does not swallow stdin.
#[test]
fn a_pipe_is_still_sniffed() {
    let out = strip(&render_stdin(b"# Title\n\nprose\n", &["--no-alt", "--plain", "--width", "80"]));
    assert!(!out.contains("# Title"), "a piped document is markdown: {out}");
    // `--format text` works on a pipe too.
    let forced = lines(&render_stdin(
        b"# Title\n\nprose\n",
        &["--no-alt", "--plain", "--width", "80", "--format", "text"],
    ));
    assert_eq!(forced, ["# Title", "", "prose"], "{forced:#?}");
}
