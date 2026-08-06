//! Tests for the crate-root wiring in `main.rs`: the NO_COLOR rule, the
//! outline used by `--toc`, index resolution and the yank delivery path.
//! Split out of `main.rs` to keep that file under the size limit.
#![deny(unsafe_code)]

use super::*;

#[test]
fn plain_mode_rules() {
    assert!(!plain_mode(false, None, true));
    assert!(plain_mode(true, None, true));
    assert!(plain_mode(false, Some("1".into()), true));
    assert!(!plain_mode(false, Some(String::new()), true));
    assert!(plain_mode(false, None, false));
}

/// Regression: `Term::new` used to apply `var_os("NO_COLOR").is_some()`
/// while the dump path applied `!v.is_empty()`, so `NO_COLOR=` produced a
/// monochrome pager and a coloured pipe. Both paths now call this one
/// function, so the same inputs must give the same answer.
#[test]
fn the_interactive_and_dump_paths_share_one_no_color_rule() {
    for flag in [false, true] {
        for no_color in [None, Some(String::new()), Some("1".into())] {
            let interactive = plain_mode(flag, no_color.clone(), true);
            let piped = plain_mode(flag, no_color.clone(), false);
            // The only legitimate difference is "stdout is not a terminal".
            assert!(piped, "a pipe is always plain");
            assert_eq!(
                interactive,
                flag || no_color.as_deref().map(|v| !v.is_empty()).unwrap_or(false)
            );
        }
    }
}

fn yank_bytes(text: &str, mux: select::clip::Mux, plain: bool) -> String {
    let (frame, _) = clipboard_frame(term::Frame::new(plain), text, mux);
    frame.as_str().to_string()
}

#[test]
fn the_yank_frame_carries_an_osc52_payload() {
    let out = yank_bytes("hi\n", select::clip::Mux::None, false);
    assert!(out.starts_with("\x1b]52;c;"), "{out:?}");
    assert!(out.ends_with('\x07'));
    assert!(out.contains(&term::base64(b"hi\n")));
}

/// Plain mode strips *styling*, not control sequences: the clipboard write
/// must survive `--plain` and `NO_COLOR`, or `y` silently stops working.
#[test]
fn plain_mode_does_not_swallow_the_clipboard_sequence() {
    assert_eq!(
        yank_bytes("hi", select::clip::Mux::None, true),
        yank_bytes("hi", select::clip::Mux::None, false)
    );
}

#[test]
fn a_multiplexer_wraps_the_same_payload() {
    let bare = yank_bytes("hi", select::clip::Mux::None, false);
    let tmux = yank_bytes("hi", select::clip::Mux::Tmux, false);
    assert_ne!(bare, tmux);
    assert!(tmux.starts_with("\x1bP") && tmux.ends_with("\x1b\\"));
    assert!(tmux.contains(&term::base64(b"hi")));
}

#[test]
fn the_report_matches_what_was_actually_framed() {
    let big = "z".repeat(term::MAX_CLIPBOARD_BYTES + 500);
    let (_, report) = clipboard_frame(
        term::Frame::new(false),
        &big,
        select::clip::Mux::None,
    );
    assert!(report.truncated);
    assert_eq!(report.sent, term::MAX_CLIPBOARD_BYTES);
    // ... and the status line says so rather than claiming success.
    let msg = select::clip::yank_message("3 lines", Some(report), Some("~/x.txt"));
    assert!(msg.contains("truncated"), "{msg}");
}

#[test]
fn a_failed_write_is_reported_as_a_refusal() {
    let msg = select::clip::yank_message("1 line", None, Some("~/.cache/tread/last-yank.txt"));
    assert!(msg.contains("refused") && msg.contains("last-yank.txt"), "{msg}");
}

fn outline_of(src: &str) -> Vec<(u8, String)> {
    outline(&md::parse(src))
}

#[test]
fn outline_reads_atx_headings() {
    let src = "# Top\n\ntext\n\n## Two\n### Three ###\n";
    assert_eq!(
        outline_of(src),
        vec![
            (1, "Top".to_string()),
            (2, "Two".to_string()),
            (3, "Three".to_string())
        ]
    );
}

#[test]
fn outline_skips_fenced_code() {
    let src = "# A\n\n```sh\n# not a heading\n```\n\n~~~\n## nope\n~~~\n\n## B\n";
    assert_eq!(outline_of(src), vec![(1, "A".into()), (2, "B".into())]);
}

#[test]
fn outline_rejects_non_headings() {
    assert!(outline_of("#hashtag\n").is_empty());
    assert!(outline_of("####### seven\n").is_empty());
    assert!(outline_of("plain\n").is_empty());
}

#[test]
fn outline_render_indents_by_level() {
    let o = vec![(1, "A".to_string()), (3, "C".to_string())];
    assert_eq!(render_outline(&o), "A\n    C\n");
}

#[test]
fn index_path_errors_are_usage_errors() {
    let missing = Path::new("/nonexistent-corpus-xyz");
    let err = index_path(Some(missing)).unwrap_err();
    assert_eq!(err.code, EXIT_USAGE);
    assert!(err.msg.contains("index not found"));
}

#[test]
fn a_terminal_that_cannot_go_raw_dumps_instead_of_failing() {
    // `sys::set_raw` returning None is documented in sys/mod.rs as "main treats
    // it as no tty and dumps instead" — the path a pre-1703 Windows conhost
    // takes, where the console cannot do VT output. Only a write error is fatal.
    assert!(is_non_interactive(&term::TermError::NoTty));
    assert!(is_non_interactive(&term::TermError::RawMode));
    assert!(!is_non_interactive(&term::TermError::Io(5)));
}
