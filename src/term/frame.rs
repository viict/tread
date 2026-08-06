//! The per-frame output buffer. One `Frame` is built entirely in memory and
//! handed to `Term::flush`, which issues exactly one `write(2)`.
//!
//! No mouse-tracking sequence (`?1000h` / `?1002h` / `?1006h`) is emitted here
//! or anywhere else: terminal-native drag-select must keep working.
#![deny(unsafe_code)]

use super::style::{write_transition, Style};

pub struct Frame {
    buf: String,
    cur: Style,
    plain: bool,
}

impl Frame {
    pub fn new(plain: bool) -> Self {
        Frame {
            buf: String::with_capacity(8 * 1024),
            cur: Style::new(),
            plain,
        }
    }
    /// Drop all buffered content, keeping the allocation.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.cur = Style::new();
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.buf.as_bytes()
    }
    pub fn as_str(&self) -> &str {
        &self.buf
    }
    /// Append text with no styling applied.
    pub fn raw(&mut self, s: &str) {
        self.buf.push_str(s);
    }
    /// Switch the active style, emitting only the difference.
    pub fn set_style(&mut self, s: Style) {
        if self.plain {
            return;
        }
        write_transition(&mut self.buf, self.cur, s);
        self.cur = s;
    }
    /// Return to the default style.
    pub fn reset_style(&mut self) {
        if self.plain || self.cur.is_default() {
            self.cur = Style::new();
            return;
        }
        self.buf.push_str("\x1b[0m");
        self.cur = Style::new();
    }
    /// Write `text` in `style`.
    pub fn span(&mut self, style: Style, text: &str) {
        self.set_style(style);
        self.buf.push_str(text);
    }
    /// Always reset before the newline so styles never bleed across lines.
    pub fn end_line(&mut self) {
        self.reset_style();
        self.buf.push_str("\r\n");
    }
    /// 1-based cursor positioning.
    pub fn move_to(&mut self, row: u16, col: u16) {
        self.buf
            .push_str(&format!("\x1b[{};{}H", row.max(1), col.max(1)));
    }
    pub fn clear_to_eol(&mut self) {
        self.buf.push_str("\x1b[K");
    }
    /// Begin an OSC 8 hyperlink. Terminals without support ignore the OSC, so
    /// this degrades to plain text. Suppressed in plain mode and for URLs that
    /// contain control characters (which would break out of the sequence).
    pub fn link_open(&mut self, url: &str) {
        if self.plain || url.is_empty() || url.len() > 2048 || !is_osc_safe(url) {
            return;
        }
        self.buf.push_str("\x1b]8;;");
        self.buf.push_str(url);
        self.buf.push_str("\x1b\\");
    }
    pub fn link_close(&mut self) {
        if self.plain {
            return;
        }
        self.buf.push_str("\x1b]8;;\x1b\\");
    }
}

/// OSC payloads must not carry control characters that would terminate them.
pub fn is_osc_safe(s: &str) -> bool {
    !s.chars().any(|c| (c as u32) < 0x20 || c == '\x7f')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(spans: &[(Style, &str)], plain: bool) -> String {
        let mut f = Frame::new(plain);
        for (s, t) in spans {
            f.span(*s, t);
        }
        f.end_line();
        f.as_str().to_string()
    }

    #[test]
    fn plain_mode_emits_no_escapes() {
        let out = render(
            &[(Style::new().fg(39).bold(), "hi"), (Style::new(), "!")],
            true,
        );
        assert_eq!(out, "hi!\r\n");
    }

    #[test]
    fn identical_consecutive_styles_emit_one_sequence() {
        let s = Style::new().fg(39);
        assert_eq!(
            render(&[(s, "a"), (s, "b")], false),
            "\x1b[38;5;39mab\x1b[0m\r\n"
        );
    }

    #[test]
    fn adding_an_attribute_is_incremental() {
        let a = Style::new().fg(4);
        let b = Style::new().fg(4).bold();
        assert_eq!(
            render(&[(a, "x"), (b, "y")], false),
            "\x1b[38;5;4mx\x1b[1my\x1b[0m\r\n"
        );
    }

    #[test]
    fn removing_an_attribute_forces_a_reset() {
        let a = Style::new().bold().underline();
        let b = Style::new().bold();
        assert_eq!(
            render(&[(a, "x"), (b, "y")], false),
            "\x1b[1;4mx\x1b[0;1my\x1b[0m\r\n"
        );
    }

    #[test]
    fn background_and_foreground_both_encode() {
        assert_eq!(
            render(&[(Style::new().fg(7).bg(236), "x")], false),
            "\x1b[38;5;7;48;5;236mx\x1b[0m\r\n"
        );
    }

    #[test]
    fn every_line_ends_reset() {
        let mut f = Frame::new(false);
        f.span(Style::new().bold(), "a");
        f.end_line();
        f.span(Style::new().bold(), "b");
        f.end_line();
        assert_eq!(f.as_str(), "\x1b[1ma\x1b[0m\r\n\x1b[1mb\x1b[0m\r\n");
    }

    #[test]
    fn frame_never_enables_mouse_tracking() {
        let mut f = Frame::new(false);
        f.move_to(3, 4);
        f.span(Style::new().fg(1), "text");
        f.clear_to_eol();
        f.link_open("https://example.com/x");
        f.raw("link");
        f.link_close();
        f.end_line();
        for bad in ["?1000", "?1002", "?1003", "?1005", "?1006", "?1015"] {
            assert!(!f.as_str().contains(bad), "mouse sequence {bad} leaked");
        }
    }

    #[test]
    fn hyperlink_helpers_wrap_the_text() {
        let mut f = Frame::new(false);
        f.link_open("https://a.b/c");
        f.raw("t");
        f.link_close();
        assert_eq!(f.as_str(), "\x1b]8;;https://a.b/c\x1b\\t\x1b]8;;\x1b\\");
    }

    #[test]
    fn hyperlinks_are_suppressed_in_plain_mode_and_for_unsafe_urls() {
        let mut f = Frame::new(true);
        f.link_open("https://a.b");
        f.raw("t");
        f.link_close();
        assert_eq!(f.as_str(), "t");
        let mut g = Frame::new(false);
        g.link_open("http://a\x07b");
        assert!(g.as_str().is_empty());
    }

    #[test]
    fn move_to_is_one_based_and_clamped() {
        let mut f = Frame::new(false);
        f.move_to(0, 0);
        assert_eq!(f.as_str(), "\x1b[1;1H");
    }

    #[test]
    fn reset_clears_buffer_and_style() {
        let mut f = Frame::new(false);
        f.span(Style::new().bold(), "x");
        f.reset();
        assert!(f.as_str().is_empty());
        f.span(Style::new().bold(), "y");
        assert_eq!(f.as_str(), "\x1b[1my");
    }

    #[test]
    fn osc_safety_check() {
        assert!(is_osc_safe("https://x.y/z?a=b#c"));
        assert!(!is_osc_safe("a\nb"));
        assert!(!is_osc_safe("a\x1bb"));
    }
}
