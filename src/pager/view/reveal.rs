//! Hidden characters: code points a terminal draws as nothing, or as a plain
//! space, so the reader cannot see them in the text (`I`, SPEC.md
//! §"Hidden characters").
//!
//! This module only *classifies*. [`super::cells::Cells::reveal`] swaps each
//! one for its glyph at paint time, after the search highlights and the link
//! focus are already placed by column, so revealing shifts nothing they mark.
#![deny(unsafe_code)]

/// A hidden character: the one-cell glyph that stands for it, and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hidden {
    pub glyph: char,
    pub name: &'static str,
}

/// Width 0 or invisible: joiners, marks, the BOM, tag characters.
const ZERO: char = '\u{b7}';
/// Looks like a space but is not U+0020.
const SPACE: char = '\u{2423}';
/// Changes the direction of the text after it.
const BIDI: char = '\u{2194}';
/// Breaks a line where the terminal does not.
const BREAK: char = '\u{b6}';

const fn h(glyph: char, name: &'static str) -> Option<Hidden> {
    Some(Hidden { glyph, name })
}

/// The glyph and name of `c` when it is hidden, else `None`.
///
/// Variation selectors (U+FE00..FE0F) are left out on purpose: every emoji
/// with a text and an emoji form carries one, and revealing them would break
/// ordinary text apart rather than expose anything.
pub fn classify(c: char) -> Option<Hidden> {
    match c {
        '\u{a0}' => h(SPACE, "NO-BREAK SPACE"),
        '\u{ad}' => h('\u{ac}', "SOFT HYPHEN"),
        '\u{34f}' => h(ZERO, "COMBINING GRAPHEME JOINER"),
        '\u{61c}' => h(BIDI, "ARABIC LETTER MARK"),
        '\u{180e}' => h(ZERO, "MONGOLIAN VOWEL SEPARATOR"),
        '\u{2000}'..='\u{200a}' => h(SPACE, "TYPOGRAPHIC SPACE"),
        '\u{200b}' => h(ZERO, "ZERO WIDTH SPACE"),
        '\u{200c}' => h(ZERO, "ZERO WIDTH NON-JOINER"),
        '\u{200d}' => h(ZERO, "ZERO WIDTH JOINER"),
        '\u{200e}' => h(BIDI, "LEFT-TO-RIGHT MARK"),
        '\u{200f}' => h(BIDI, "RIGHT-TO-LEFT MARK"),
        '\u{2028}' => h(BREAK, "LINE SEPARATOR"),
        '\u{2029}' => h(BREAK, "PARAGRAPH SEPARATOR"),
        '\u{202a}'..='\u{202e}' => h(BIDI, "BIDI EMBEDDING OR OVERRIDE"),
        '\u{202f}' => h(SPACE, "NARROW NO-BREAK SPACE"),
        '\u{205f}' => h(SPACE, "MEDIUM MATHEMATICAL SPACE"),
        '\u{2060}' => h(ZERO, "WORD JOINER"),
        '\u{2061}'..='\u{2064}' => h(ZERO, "INVISIBLE OPERATOR"),
        '\u{2066}'..='\u{2069}' => h(BIDI, "BIDI ISOLATE"),
        '\u{206a}'..='\u{206f}' => h(ZERO, "DEPRECATED FORMAT CHARACTER"),
        '\u{feff}' => h(ZERO, "ZERO WIDTH NO-BREAK SPACE"),
        '\u{e0001}' | '\u{e0020}'..='\u{e007f}' => h(ZERO, "TAG CHARACTER"),
        _ => None,
    }
}

/// `U+200B ZERO WIDTH SPACE, U+00A0 NO-BREAK SPACE` for the hidden characters
/// in `text`, each named once in the order it first appears. `None` when
/// there are none.
pub fn describe(text: &str) -> Option<String> {
    let mut seen: Vec<char> = Vec::new();
    for c in text.chars() {
        if classify(c).is_some() && !seen.contains(&c) {
            seen.push(c);
        }
    }
    let names: Vec<String> = seen
        .iter()
        .filter_map(|&c| classify(c).map(|k| format!("U+{:04X} {}", c as u32, k.name)))
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_common_offenders_are_hidden() {
        for c in ['\u{a0}', '\u{200b}', '\u{200d}', '\u{202e}', '\u{2066}', '\u{feff}', '\u{e0041}'] {
            assert!(classify(c).is_some(), "U+{:04X}", c as u32);
        }
        assert_eq!(classify('\u{200b}').map(|k| k.glyph), Some(ZERO));
        assert_eq!(classify('\u{a0}').map(|k| k.glyph), Some(SPACE));
        assert_eq!(classify('\u{202e}').map(|k| k.glyph), Some(BIDI));
    }

    #[test]
    fn visible_text_and_emoji_selectors_are_not() {
        for c in ['a', ' ', '\t', '\u{b7}', '\u{4e2d}', '\u{fe0f}', '\u{301}'] {
            assert_eq!(classify(c), None, "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn every_glyph_is_one_visible_cell() {
        for c in ['\u{a0}', '\u{ad}', '\u{200b}', '\u{200e}', '\u{2028}'] {
            let g = classify(c).map(|k| k.glyph).unwrap_or(' ');
            assert_eq!(crate::render::char_width(g), 1);
            assert_eq!(classify(g), None, "a glyph must not itself be hidden");
        }
    }

    #[test]
    fn describe_names_each_character_once_in_order() {
        let text = "a\u{200b}b\u{a0}c\u{200b}";
        assert_eq!(
            describe(text).as_deref(),
            Some("U+200B ZERO WIDTH SPACE, U+00A0 NO-BREAK SPACE"),
        );
        assert_eq!(describe("plain"), None);
    }
}
