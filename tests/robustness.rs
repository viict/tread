//! Adversarial-input soak, run as part of `cargo test`.
//!
//! The shell soak (`tools/soak.sh`, `tools/soak_pty.py`) covers the real corpus
//! and the interactive path; this file keeps the crash-and-corruption cases that
//! must never regress inside the ordinary test run. Everything here drives the
//! real binary, so it also proves the CLI, the decoder and the dump path agree.

mod harness;

use harness::{render, render_stdin, strip, temp_doc};

const WIDTHS: [&str; 5] = ["1", "20", "40", "80", "200"];

/// No render may emit an escape byte to a pipe, ever.
fn assert_clean(label: &str, out: &str) {
    assert!(
        !out.contains('\u{1b}'),
        "{label}: escape leaked into a piped render"
    );
    for c in out.chars() {
        assert!(
            c == '\n' || c == '\t' || !c.is_control(),
            "{label}: raw control {c:?} reached the output"
        );
    }
    assert_eq!(strip(out), out, "{label}: stripping changed a plain render");
}

/// Render `body` at every width and return the width-80 output.
fn all_widths(name: &str, body: &[u8]) -> String {
    let path = temp_doc(name, body);
    let mut at80 = String::new();
    for w in WIDTHS {
        let out = render(&path, &["--width", w]);
        assert_clean(&format!("{name}@{w}"), &out);
        if w == "80" {
            at80 = out;
        }
    }
    // --toc must survive the same inputs.
    let _ = render(&path, &["--toc"]);
    let _ = std::fs::remove_file(&path);
    at80
}

#[test]
fn empty_and_whitespace_only_documents() {
    assert_eq!(all_widths("empty", b"").trim(), "");
    assert_eq!(all_widths("ws", b"   \n\t\n \n").trim(), "");
    assert_eq!(all_widths("newlines", b"\n\n\n\n").trim(), "");
}

#[test]
fn a_bom_only_file_renders_nothing() {
    assert_eq!(all_widths("bom-only", b"\xef\xbb\xbf").trim(), "");
}

#[test]
fn a_bom_is_not_mistaken_for_paragraph_text() {
    let out = all_widths("bom-head", "\u{feff}# Title\n\nbody\n".as_bytes());
    assert!(out.contains("body"), "{out:?}");
    assert!(!out.contains('\u{feff}'), "BOM survived into the render");
}

#[test]
fn invalid_utf8_is_replaced_not_rejected() {
    let out = all_widths("bad-utf8", b"# T\n\nbefore \xff\xfe\x80 after\n");
    assert!(out.contains("before \u{fffd}\u{fffd}\u{fffd} after"), "{out:?}");
}

#[test]
fn control_characters_cannot_escape_the_document() {
    let body = b"# C\n\nx \x1b[31mred\x1b[0m \x1b]0;title\x07 \x07\x08\x7f \xc2\x9b y\n";
    let out = all_widths("controls", body);
    // The bytes are still visible as replacement characters, but inert.
    assert!(out.contains("[31mred"), "text was dropped: {out:?}");
    assert!(out.contains('\u{fffd}'), "controls were not replaced");
}

#[test]
fn nul_bytes_are_replaced() {
    let out = all_widths("nul", b"# N\n\nbefore\x00after\n");
    assert!(out.contains("before\u{fffd}after"), "{out:?}");
}

#[test]
fn crlf_documents_parse_as_if_they_were_lf() {
    let crlf = b"# H\r\n\r\n- one\r\n- two\r\n\r\n| a | b |\r\n|---|---|\r\n| 1 | 2 |\r\n";
    let lf = b"# H\n\n- one\n- two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let a = all_widths("crlf", crlf);
    let b = all_widths("lf", lf);
    assert_eq!(a, b, "CRLF and LF renders diverged");
    assert!(a.contains("one") && a.contains("\u{2502}"), "{a:?}");
}

#[test]
fn a_lone_cr_is_a_line_break_not_a_cursor_move() {
    let out = all_widths("lone-cr", b"a\rb\rc\n");
    assert!(!out.contains('\r'), "carriage return survived: {out:?}");
    assert!(out.contains("a b c") || out.contains('a'), "{out:?}");
}

#[test]
fn a_single_word_longer_than_the_width_does_not_hang_or_panic() {
    let mut body = b"# Long\n\n".to_vec();
    body.extend(std::iter::repeat(b'W').take(5000));
    body.extend(b"\n\ntail\n");
    let out = all_widths("longword", &body);
    assert!(out.contains("tail"), "content after the long word was lost");
    assert!(out.contains("WWWW"));
}

#[test]
fn a_long_word_inside_a_code_block_is_not_wrapped() {
    let mut body = b"```\n".to_vec();
    body.extend(std::iter::repeat(b'X').take(5000));
    body.extend(b"\n```\n");
    let out = all_widths("longword-code", &body);
    assert!(out.contains("XXXX"));
}

#[test]
fn deeply_nested_lists_terminate() {
    let mut body = String::from("# Deep\n\n");
    for i in 0..200 {
        body.push_str(&"  ".repeat(i));
        body.push_str(&format!("- level {i}\n"));
    }
    let out = all_widths("deep-list", body.as_bytes());
    assert!(out.contains("level 0"));
}

#[test]
fn deeply_nested_quotes_terminate() {
    let body = format!("{} deep\n", ">".repeat(500));
    let out = all_widths("deep-quote", body.as_bytes());
    assert!(out.contains("deep"), "{out:?}");
}

#[test]
fn an_unclosed_fence_runs_to_end_of_file() {
    let out = all_widths("unclosed", b"# D\n\n```rust\nfn main() {\n    let x = 1;\n");
    assert!(out.contains("let x = 1;"), "{out:?}");
    let out = all_widths("unclosed-tilde", b"~~~\nstuff\n");
    assert!(out.contains("stuff"), "{out:?}");
}

#[test]
fn malformed_tables_do_not_panic() {
    let body = b"| a | b | c |\n|:--|:-:|--:|\n| 1 |\n| 1 | 2 | 3 | 4 | 5 |\n|||\n\n|---|\n";
    let out = all_widths("bad-table", body);
    assert!(out.contains('\u{2502}'), "no table drawn: {out:?}");
}

#[test]
fn unbalanced_inline_markup_does_not_panic() {
    let body = format!(
        "{}\n\n{}\n\n{}\n\n{}x{}\n\n![i](a \"t\") [r][z] <http://x> \\*e\\* ~~s~~\n",
        "*".repeat(400),
        "_".repeat(400),
        "`".repeat(200),
        "[".repeat(200),
        "]".repeat(200),
    );
    let out = all_widths("inline-evil", body.as_bytes());
    assert!(!out.is_empty());
}

#[test]
fn wide_zero_width_and_emoji_text_never_splits_a_char() {
    let body = "# W\n\n\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8} \
        a\u{301}e\u{301} \u{1f469}\u{200d}\u{1f4bb} \u{1f1ef}\u{1f1f5} a\u{200b}b\n\n\
        | \u{5217}\u{4e00} | b |\n| --- | --- |\n| \u{65e5}\u{672c}\u{8a9e} | x |\n";
    let out = all_widths("wide", body.as_bytes());
    assert!(out.contains('\u{65e5}'));
}

#[test]
fn a_multibyte_heading_with_a_closing_hash_run() {
    // Regression: byte arithmetic over the closing `#` run used to slice
    // `\u{30c8}` in half and panic.
    for src in [
        "# \u{65e5}\u{672c}\u{8a9e}\u{30c6}\u{30b9}\u{30c8}\n",
        "# \u{30c6}\u{30b9}\u{30c8} #\n",
        "## \u{30c6}\u{30b9}\u{30c8}###\n",
        "### \u{30c6} ###\n",
        "#\u{30c6}\n",
        "####### \u{30c6}\n",
    ] {
        let out = all_widths("multibyte-heading", src.as_bytes());
        assert!(!out.is_empty() || src.starts_with("#######"));
    }
}

#[test]
fn stdin_takes_the_same_path_as_a_file() {
    let body = b"# S\n\nfrom a pipe with \xff bad bytes and \r\n CRLF\n";
    let out = render_stdin(body, &["--width", "60"]);
    assert_clean("stdin", &out);
    assert!(out.contains("from a pipe"), "{out:?}");
}

/// A deterministic shuffle of markdown fragments: cheap coverage of parser
/// state transitions that hand-written cases miss. Any panic fails the test
/// because `render` asserts on the child's exit status.
#[test]
fn pseudo_random_documents_never_crash_the_renderer() {
    const FRAGMENTS: [&str; 24] = [
        "# H1\n",
        "###### H6 ###\n",
        "text **bold _mixed_** `code`\n",
        "\n",
        "```rust\n",
        "```\n",
        "~~~\n",
        "> quote\n",
        ">> deeper\n",
        "- item\n",
        "  - nested\n",
        "1. one\n",
        "- [ ] task\n",
        "| a | b |\n",
        "| --- | ---: |\n",
        "| 1 | 2 |\n",
        "---\n",
        "[link](a.md)\n",
        "[ref]: http://x\n",
        "<div>html</div>\n",
        "    indented code\n",
        "\u{65e5}\u{672c}\u{8a9e}\n",
        "\ttab led\n",
        "a\u{301}\u{200b}\u{1f680}\n",
    ];
    // xorshift, so the corpus is identical on every machine and every run.
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for doc in 0..40 {
        let mut body = String::new();
        for _ in 0..60 {
            body.push_str(FRAGMENTS[(next() % FRAGMENTS.len() as u64) as usize]);
        }
        let path = temp_doc(&format!("fuzz{doc}"), body.as_bytes());
        for w in ["7", "40", "133"] {
            let out = render(&path, &["--width", w]);
            assert_clean(&format!("fuzz{doc}@{w}"), &out);
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// A dump is not a viewport: metadata starts folded for a reader, and a folded
/// block in a pipe is simply missing output. `tread doc.md > out.txt` must
/// contain every field.
#[test]
fn a_dump_shows_metadata_that_a_reader_would_have_to_unfold() {
    let body = b"---\nstatus: Active\nrelated:\n  - models/A.md\n---\n\n# Title\n\nbody\n";
    let path = temp_doc("metadata", body);
    let out = strip(&render(&path, &["--width", "80"]));
    assert!(out.contains("status"), "{out}");
    assert!(out.contains("models/A.md"), "the folded field is in the dump: {out}");
    // The H1 is a block-glyph banner, so assert on the prose instead.
    assert!(out.contains("body"), "{out}");
}
