//! Clipboard framing: base64, tmux/screen wrapping, fallback file, wording.
#![deny(unsafe_code)]

use super::*;
// -- clipboard framing -------------------------------------------------------

/// A minimal RFC 4648 decoder, so encoding is checked by round trip and not
/// only against fixed vectors.
fn unbase64(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut acc = 0u32;
    let mut bits = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let v = A.iter().position(|x| *x == c).expect("base64 alphabet") as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[test]
fn base64_matches_known_vectors_and_round_trips() {
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64(b"f"), "Zg==");
    for s in ["", "a", "ab", "abc", "| k | v |\n", "héllo 🦀\n```\n"] {
        assert_eq!(unbase64(&base64(s.as_bytes())), s.as_bytes(), "{s:?}");
    }
}

#[test]
fn mux_detection_prefers_tmux() {
    assert_eq!(detect_mux(true, false, Some("xterm-256color")), Mux::Tmux);
    assert_eq!(detect_mux(true, true, Some("screen")), Mux::Tmux);
    assert_eq!(detect_mux(false, true, Some("xterm")), Mux::Screen);
    assert_eq!(detect_mux(false, false, Some("screen.linux")), Mux::Screen);
    assert_eq!(detect_mux(false, false, Some("xterm-256color")), Mux::None);
    assert_eq!(detect_mux(false, false, None), Mux::None);
}

#[test]
fn plain_and_tmux_sequences_have_the_expected_shape() {
    let (plain, rep) = clipboard_sequence("hi\n", Mux::None);
    assert_eq!(plain, format!("\x1b]52;c;{}\x07", base64(b"hi\n")));
    assert!(!rep.truncated);
    let (tmux, _) = clipboard_sequence("hi\n", Mux::Tmux);
    assert!(tmux.starts_with("\x1bPtmux;\x1b\x1b]52;c;"));
    assert!(tmux.ends_with("\x07\x1b\\"));
    assert_eq!(tmux.matches('\x1b').count(), 4, "inner ESC doubled, plus DCS and ST");
}

#[test]
fn screen_sequences_are_dcs_wrapped_and_chunked() {
    let (short, _) = clipboard_sequence("hi", Mux::Screen);
    assert_eq!(short, format!("\x1bP\x1b]52;c;{}\x07\x1b\\", base64(b"hi")));
    let big = "x".repeat(4000);
    let (long, rep) = clipboard_sequence(&big, Mux::Screen);
    assert!(!rep.truncated);
    let chunks = long.matches("\x1bP").count();
    assert!(chunks > 1, "a 4 KB payload must be split for screen");
    // every chunk is closed and none exceeds the DCS budget
    assert_eq!(chunks, long.matches("\x1b\\").count());
    for part in long.split("\x1bP").skip(1) {
        assert!(part.len() <= SCREEN_CHUNK + 2, "chunk of {} bytes", part.len());
    }
    // and the payload survives reassembly
    let joined: String = long
        .split("\x1bP")
        .skip(1)
        .map(|p| p.trim_end_matches("\x1b\\"))
        .collect();
    let b64 = joined.trim_start_matches("\x1b]52;c;").trim_end_matches('\x07');
    assert_eq!(unbase64(b64), big.as_bytes());
}

#[test]
fn oversized_payloads_truncate_instead_of_breaking_the_escape() {
    let big = "y".repeat(crate::term::MAX_CLIPBOARD_BYTES + 100);
    for mux in [Mux::None, Mux::Tmux, Mux::Screen] {
        let (seq, rep) = clipboard_sequence(&big, mux);
        assert!(rep.truncated, "{mux:?}");
        assert_eq!(rep.sent, crate::term::MAX_CLIPBOARD_BYTES);
        assert!(seq.contains("\x1b]52;c;") || seq.contains("\x1b\x1b]52;c;"));
    }
}

#[test]
fn no_clipboard_path_ever_emits_mouse_tracking() {
    for mux in [Mux::None, Mux::Tmux, Mux::Screen] {
        let (seq, _) = clipboard_sequence("drag select must keep working", mux);
        for bad in ["?1000", "?1002", "?1003", "?1006", "?1015"] {
            assert!(!seq.contains(bad), "{bad} leaked into {mux:?}");
        }
    }
}

// -- fallback file and wording ----------------------------------------------

/// The per-platform rules live in `plat::dirs` and are tested there for all
/// three at once; this checks the wiring — that `fallback_path` reads the
/// environment through that table and lands on *this* platform's answer.
#[test]
fn fallback_path_uses_this_platforms_cache_location() {
    use crate::plat::dirs::Env;
    use crate::plat::Platform;
    let env = Env::of(&[
        ("HOME", "/home/u"),
        ("USERPROFILE", "C:\\Users\\u"),
        ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
    ]);
    let want = match Platform::HOST {
        Platform::Linux => "/home/u/.cache/mdr/last-yank.txt",
        Platform::Macos => "/home/u/Library/Caches/mdr/last-yank.txt",
        Platform::Windows => "C:\\Users\\u\\AppData\\Local\\mdr\\last-yank.txt",
    };
    assert_eq!(fallback_path(&env), Some(PathBuf::from(want)));
    assert_eq!(fallback_path(&Env::default()), None);
}

/// The full text is written verbatim, LF-only: `md::sanitize::clean` has
/// already folded every CRLF and lone CR, so the cache file cannot pick up a
/// stray `\r` on any platform, and nothing here adds one.
#[test]
fn the_fallback_file_content_is_lf_only() {
    let doc = crate::md::parse("a\r\n\r\nb\rc\r\n\r\n```\r\nx\r\ny\r\n```\r\n");
    let text = crate::select::source::blocks_markdown(&doc.blocks);
    assert!(!text.contains('\r'), "{text:?}");
    assert!(text.contains("x\ny"), "{text:?}");
}

#[test]
fn paths_under_home_are_shown_the_way_this_platform_writes_them() {
    use crate::plat::Platform;
    let home = PathBuf::from("/home/u");
    let p = PathBuf::from("/home/u/.cache/mdr/last-yank.txt");
    let shortened = match Platform::HOST.is_windows() {
        // No `~` on Windows: nothing there expands it (see plat::dirs).
        true => "/home/u/.cache/mdr/last-yank.txt",
        false => "~/.cache/mdr/last-yank.txt",
    };
    assert_eq!(display_path(&p, Some(&home)), shortened);
    assert_eq!(display_path(&p, None), "/home/u/.cache/mdr/last-yank.txt");
    let other = PathBuf::from("/tmp/x.txt");
    assert_eq!(display_path(&other, Some(&home)), "/tmp/x.txt");
}

#[test]
fn messages_say_what_happened() {
    let ok = ClipReport { sent: 12, truncated: false };
    assert_eq!(
        yank_message("3 lines", Some(ok), Some("~/.cache/mdr/last-yank.txt")),
        "yanked 3 lines  \u{b7}  saved to ~/.cache/mdr/last-yank.txt"
    );
    assert_eq!(yank_message("1 line", Some(ok), None), "yanked 1 line");
    let cut = ClipReport { sent: 73_000, truncated: true };
    let msg = yank_message("900 lines", Some(cut), Some("~/y.txt"));
    assert!(msg.contains("truncated to 73000 bytes") && msg.contains("~/y.txt"), "{msg}");
    let failed = yank_message("3 lines", None, Some("~/y.txt"));
    assert!(failed.contains("refused") && failed.contains("~/y.txt"), "{failed}");
    assert!(yank_message("3 lines", None, None).starts_with("could not copy"));
}

#[test]
fn line_count_is_singular_at_one() {
    assert_eq!(line_count(0), "0 lines");
    assert_eq!(line_count(1), "1 line");
    assert_eq!(line_count(7), "7 lines");
}
