//! Display width from Unicode ranges — no crate, no tables beyond these.
//!
//! Every wrap / pad / truncate / table-sizing computation in the renderer goes
//! through [`char_width`] and [`str_width`]; `.len()` and `.chars().count()`
//! are never used for layout (SPEC.md §Width & unicode).
#![deny(unsafe_code)]

/// Combining marks, format controls and variation selectors: width 0.
#[rustfmt::skip]
const ZERO: &[(u32, u32)] = &[
    (0x0300, 0x036F), (0x0483, 0x0489), (0x0591, 0x05BD), (0x05BF, 0x05BF),
    (0x05C1, 0x05C2), (0x0610, 0x061A), (0x064B, 0x065F), (0x0670, 0x0670),
    (0x06D6, 0x06DC), (0x06DF, 0x06E4), (0x0711, 0x0711), (0x0730, 0x074A),
    (0x07A6, 0x07B0), (0x07EB, 0x07F3), (0x0816, 0x082D), (0x0859, 0x085B),
    (0x08E3, 0x0902), (0x093A, 0x093A), (0x093C, 0x093C), (0x0941, 0x0948),
    (0x094D, 0x094D), (0x0951, 0x0957), (0x0E31, 0x0E31), (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E), (0x0EB1, 0x0EB1), (0x0EB4, 0x0EBC), (0x0F71, 0x0F84),
    (0x135D, 0x135F), (0x1AB0, 0x1AFF), (0x1DC0, 0x1DFF), (0x200B, 0x200F),
    (0x202A, 0x202E), (0x2060, 0x2064), (0x206A, 0x206F), (0x20D0, 0x20F0),
    (0x3099, 0x309A), (0xFE00, 0xFE0E), (0xFE20, 0xFE2F), (0xFEFF, 0xFEFF),
    (0xE0100, 0xE01EF),
];

/// East-Asian Wide / Fullwidth plus the emoji blocks: width 2.
#[rustfmt::skip]
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F), (0x231A, 0x231B), (0x2329, 0x232A), (0x23E9, 0x23EC),
    (0x23F0, 0x23F0), (0x23F3, 0x23F3), (0x25FD, 0x25FE), (0x2614, 0x2615),
    (0x2648, 0x2653), (0x267F, 0x267F), (0x2693, 0x2693), (0x26AA, 0x26AB),
    (0x26BD, 0x26BE), (0x2705, 0x2705), (0x270A, 0x270B), (0x2728, 0x2728),
    (0x274C, 0x274C), (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55),
    (0x2E80, 0x303E), (0x3041, 0x33FF), (0x3400, 0x4DBF), (0x4E00, 0x9FFF),
    (0xA000, 0xA4CF), (0xA960, 0xA97F), (0xAC00, 0xD7A3), (0xF900, 0xFAFF),
    (0xFE10, 0xFE19), (0xFE30, 0xFE6F), (0xFF00, 0xFF60), (0xFFE0, 0xFFE6),
    (0x16FE0, 0x16FE4), (0x17000, 0x18AFF), (0x1B000, 0x1B2FF),
    (0x1F004, 0x1F004), (0x1F0CF, 0x1F0CF), (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A), (0x1F300, 0x1FAFF), (0x20000, 0x3FFFD),
];

fn in_ranges(cp: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(lo, hi)| cp >= lo && cp <= hi)
}

/// Terminal cells occupied by `c`: 0 for combining/zero-width/control, 2 for
/// wide CJK and emoji, 1 otherwise.
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    if cp < 0x300 {
        // ASCII and Latin-1: everything printable is one cell, controls zero.
        return if cp < 0x20 || cp == 0x7f { 0 } else { 1 };
    }
    if in_ranges(cp, ZERO) {
        return 0;
    }
    // U+FE0F asks for emoji presentation, which terminals draw two cells wide.
    // Counting the selector itself as one cell makes `base + FE0F` measure 2
    // without needing lookahead in every caller.
    if cp == 0xFE0F {
        return 1;
    }
    if in_ranges(cp, WIDE) {
        return 2;
    }
    1
}

/// Display width of a string.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Longest prefix of `s` that fits in `max` columns, plus its width.
///
/// Zero-width characters always join the prefix, so a combining mark is never
/// separated from the base character it decorates.
pub fn take_width(s: &str, max: usize) -> (&str, usize) {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        let w = char_width(c);
        if w > 0 && used + w > max {
            return (&s[..i], used);
        }
        used += w;
    }
    (s, used)
}

/// `s` truncated to `max` columns.
pub fn truncate_width(s: &str, max: usize) -> &str {
    take_width(s, max).0
}

/// `s` padded on the right with spaces to `max` columns (never truncated).
pub fn pad_right(s: &str, max: usize) -> String {
    let mut out = s.to_string();
    for _ in str_width(s)..max {
        out.push(' ');
    }
    out
}

/// Repeat `c` `n` times.
pub fn repeat(c: char, n: usize) -> String {
    std::iter::repeat(c).take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_cell() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width(' '), 1);
        assert_eq!(str_width("hello"), 5);
    }

    #[test]
    fn controls_are_zero() {
        assert_eq!(char_width('\n'), 0);
        assert_eq!(char_width('\x7f'), 0);
        assert_eq!(char_width('\u{1b}'), 0);
    }

    #[test]
    fn combining_marks_are_zero() {
        assert_eq!(char_width('\u{301}'), 0);
        // "e" + combining acute is one cell wide.
        assert_eq!(str_width("e\u{301}"), 1);
        assert_eq!(str_width("cafe\u{301}"), 4);
        assert_eq!(char_width('\u{200b}'), 0);
        assert_eq!(char_width('\u{200d}'), 0);
        assert_eq!(char_width('\u{fe0e}'), 0);
    }

    #[test]
    fn emoji_presentation_measures_two_cells() {
        // U+26A0 is narrow on its own; with the emoji selector a terminal
        // draws it two cells wide.
        assert_eq!(str_width("\u{26a0}"), 1);
        assert_eq!(str_width("\u{26a0}\u{fe0f}"), 2);
        assert_eq!(str_width("\u{2705}"), 2);
    }

    #[test]
    fn cjk_kana_hangul_are_two_cells() {
        assert_eq!(char_width('\u{4e2d}'), 2); // 中
        assert_eq!(char_width('\u{3042}'), 2); // あ
        assert_eq!(char_width('\u{d55c}'), 2); // 한
        assert_eq!(char_width('\u{ff21}'), 2); // Ａ fullwidth
        assert_eq!(str_width("\u{4e2d}\u{6587}ab"), 6);
    }

    #[test]
    fn emoji_are_two_cells_and_sequences_do_not_panic() {
        assert_eq!(char_width('\u{1f600}'), 2);
        assert_eq!(char_width('\u{1f680}'), 2);
        // family ZWJ sequence: three wide glyphs joined by zero-width joiners
        let zwj = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f466}";
        assert_eq!(str_width(zwj), 6);
    }

    #[test]
    fn box_drawing_stays_narrow() {
        for c in "\u{2500}\u{2502}\u{250c}\u{2510}\u{2514}\u{2518}\u{251c}\u{2524}".chars() {
            assert_eq!(char_width(c), 1, "{c:?}");
        }
        for c in "\u{2022}\u{25e6}\u{25aa}\u{258f}\u{2588}\u{25be}\u{25b8}".chars() {
            assert_eq!(char_width(c), 1, "{c:?}");
        }
    }

    #[test]
    fn take_width_respects_wide_chars() {
        let s = "\u{4e2d}\u{6587}x";
        assert_eq!(take_width(s, 3), ("\u{4e2d}", 2));
        assert_eq!(take_width(s, 4), ("\u{4e2d}\u{6587}", 4));
        assert_eq!(take_width(s, 5), (s, 5));
        assert_eq!(take_width(s, 0), ("", 0));
    }

    #[test]
    fn take_width_keeps_combining_marks_attached() {
        let (p, w) = take_width("ae\u{301}b", 2);
        assert_eq!(p, "ae\u{301}");
        assert_eq!(w, 2);
    }

    #[test]
    fn padding_uses_display_width() {
        assert_eq!(pad_right("\u{4e2d}", 4), "\u{4e2d}  ");
        assert_eq!(pad_right("abc", 2), "abc");
        assert_eq!(repeat('-', 3), "---");
    }
}

/// Text safe to paint: every control character replaced by a visible dot.
///
/// Data can legitimately hold control bytes — a quoted CSV field may contain a
/// newline, and a JSON string may contain a tab — but none of them can be sent
/// to a terminal, where they would move the cursor and tear the frame. The
/// substitution lives here, once, so a format decides *what* it shows and never
/// how to make it safe. Yanked text keeps the original bytes: this is a display
/// transform, not a change to the data.
pub fn visible(raw: &str) -> String {
    match raw.chars().any(char::is_control) {
        false => raw.to_string(),
        true => raw
            .chars()
            .map(|c| if c.is_control() { CONTROL } else { c })
            .collect(),
    }
}

/// Stands in for a control character on screen.
pub const CONTROL: char = '\u{b7}';

#[cfg(test)]
mod visible_tests {
    use super::*;

    #[test]
    fn control_characters_become_dots_and_the_rest_is_untouched() {
        assert_eq!(visible("plain"), "plain");
        assert_eq!(visible("two\nlines"), "two\u{b7}lines");
        assert_eq!(visible("a\tb\rc\0d"), "a\u{b7}b\u{b7}c\u{b7}d");
        assert_eq!(visible("\u{4e2d}\u{6587}"), "\u{4e2d}\u{6587}");
    }
}
