//! OSC 52 clipboard writes, with a hand-rolled base64 (zero dependencies) and
//! tmux passthrough wrapping.
#![deny(unsafe_code)]

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 (RFC 4648) with `=` padding.
pub fn base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Most terminals refuse OSC 52 payloads beyond roughly 100 KB of base64 and
/// silently drop or bisect the sequence. Stay comfortably under that and report
/// truncation instead of emitting something the terminal will mangle.
/// 73000 raw bytes -> 97336 base64 characters.
pub const MAX_CLIPBOARD_BYTES: usize = 73_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClipReport {
    /// Raw bytes actually placed on the clipboard.
    pub sent: usize,
    /// True when the payload had to be cut to fit terminal limits.
    pub truncated: bool,
}

/// Build the OSC 52 sequence for `text`, wrapping it for tmux when `in_tmux`.
///
/// The cut point is always moved back to a `char` boundary so the clipboard
/// never receives a half-encoded scalar.
pub fn osc52_sequence(text: &str, in_tmux: bool) -> (String, ClipReport) {
    let mut cut = text.len().min(MAX_CLIPBOARD_BYTES);
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let report = ClipReport {
        sent: cut,
        truncated: cut < text.len(),
    };
    let inner = format!("\x1b]52;c;{}\x07", base64(&text.as_bytes()[..cut]));
    let seq = if in_tmux {
        // tmux passthrough: DCS `tmux;` <inner, every ESC doubled> ST
        format!("\x1bPtmux;{}\x1b\\", inner.replace('\x1b', "\x1b\x1b"))
    } else {
        inner
    };
    (seq, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_high_bytes_and_utf8() {
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
        assert_eq!(base64("é".as_bytes()), "w6k=");
        assert_eq!(base64("🦀".as_bytes()), "8J+mgA==");
    }

    #[test]
    fn base64_length_is_always_a_multiple_of_four() {
        for n in 0..40usize {
            let v = vec![b'x'; n];
            assert_eq!(base64(&v).len() % 4, 0);
        }
    }

    #[test]
    fn plain_form() {
        let (seq, rep) = osc52_sequence("foo", false);
        assert_eq!(seq, "\x1b]52;c;Zm9v\x07");
        assert_eq!(
            rep,
            ClipReport {
                sent: 3,
                truncated: false
            }
        );
    }

    #[test]
    fn tmux_form_doubles_escapes() {
        let (seq, _) = osc52_sequence("foo", true);
        assert_eq!(seq, "\x1bPtmux;\x1b\x1b]52;c;Zm9v\x07\x1b\\");
    }

    #[test]
    fn large_payloads_are_truncated_and_reported() {
        let big = "a".repeat(MAX_CLIPBOARD_BYTES + 500);
        let (seq, rep) = osc52_sequence(&big, false);
        assert!(rep.truncated);
        assert_eq!(rep.sent, MAX_CLIPBOARD_BYTES);
        assert!(seq.starts_with("\x1b]52;c;") && seq.ends_with('\x07'));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        // 2-byte chars: an odd cut would split one in half.
        let s = "é".repeat(MAX_CLIPBOARD_BYTES);
        let (_, rep) = osc52_sequence(&s, false);
        assert!(rep.truncated);
        assert_eq!(rep.sent % 2, 0);
        // 4-byte chars against an odd limit.
        let e = "🦀".repeat(MAX_CLIPBOARD_BYTES);
        let (_, rep) = osc52_sequence(&e, false);
        assert_eq!(rep.sent % 4, 0);
        assert!(rep.sent <= MAX_CLIPBOARD_BYTES);
    }

    #[test]
    fn empty_selection_is_a_valid_clear() {
        let (seq, rep) = osc52_sequence("", false);
        assert_eq!(seq, "\x1b]52;c;\x07");
        assert_eq!(
            rep,
            ClipReport {
                sent: 0,
                truncated: false
            }
        );
    }

    #[test]
    fn no_mouse_sequences_ever() {
        let (seq, _) = osc52_sequence("x", true);
        assert!(!seq.contains("?100") && !seq.contains("?101"));
    }
}
