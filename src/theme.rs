//! Colour palette, per-element styles and the H1 banner glyph font.
//!
//! Everything the renderer paints with lives here so a palette change never
//! touches layout code (SPEC.md §Testing: layout tests assert on ANSI-stripped
//! text, style tests assert on spans).
#![deny(unsafe_code)]

use crate::term::Style;

// ---------------------------------------------------------------------------
// Palette (xterm-256 indices)
// ---------------------------------------------------------------------------

/// Primary accent: H1 banners, status-bar highlights.
pub const ACCENT: u8 = 45;
/// Link blue, mandated by SPEC.md §Inline (`38;5;39`). Links that stay inside
/// the reader.
pub const LINK: u8 = 39;
/// A link that leaves the reader — `http`, `https`, `mailto` and every scheme
/// that will be refused (SPEC.md §Navigation: external links "are coloured apart
/// from links that stay inside the reader, so it is visible before pressing
/// which links leave").
///
/// Violet rather than a second blue: the point is that the eye separates the two
/// *without* reading the URL, and two shades of blue on a 256-colour terminal do
/// not survive an unusual palette. It reuses the hue the JSON tree already gives
/// booleans rather than starting a third family.
pub const LINK_EXTERNAL: u8 = 141;
pub const H1_FG: u8 = ACCENT;
pub const H2_FG: u8 = 231;
pub const H3_FG: u8 = 51;
pub const H4_FG: u8 = 44;
pub const H5_FG: u8 = 37;
pub const H6_FG: u8 = 245;
/// Horizontal rules under H1/H2 and thematic breaks.
pub const RULE_FG: u8 = 240;
pub const CODE_FG: u8 = 252;
pub const CODE_BG: u8 = 236;
pub const CODE_SPAN_FG: u8 = 216;
pub const CODE_SPAN_BG: u8 = 236;
pub const QUOTE_BAR_FG: u8 = 244;
pub const QUOTE_FG: u8 = 250;
pub const TABLE_BORDER_FG: u8 = 240;
pub const TABLE_HEAD_FG: u8 = 231;
/// Muted text: footnotes, HTML literals, language labels, image alts.
pub const MUTED_FG: u8 = 245;

// Syntax colouring (SPEC.md §Code). Four hues and no more: a reader is scanning
// for shape, and every extra colour competes with the search highlight and the
// cursor row. Comments reuse `MUTED_FG` — they are prose, not a fifth category.
pub const SYNTAX_KEYWORD: u8 = 176;
pub const SYNTAX_STRING: u8 = 114;
pub const SYNTAX_NUMBER: u8 = 216;
pub const SEARCH_BG: u8 = 226;
pub const SEARCH_FG: u8 = 16;
pub const SEARCH_CURRENT_BG: u8 = 208;
pub const SELECTION_BG: u8 = 238;
pub const STATUS_BG: u8 = 238;
pub const STATUS_FG: u8 = 252;
pub const GUTTER_FG: u8 = 244;
pub const TASK_DONE_FG: u8 = 114;
/// The `+` on a row carrying fields past the header. Amber: it is a notice,
/// not an error — the data is there, it is just not in the grid.
pub const MORE_FG: u8 = 214;
/// Document status: live, still in flight, historical.
/// Background of the CSV column the cursor is on.
pub const COLUMN_BG: u8 = 237;
pub const STATUS_LIVE: u8 = 114;
pub const STATUS_OPEN: u8 = 214;

// ---------------------------------------------------------------------------
// Element styles
// ---------------------------------------------------------------------------

pub const fn text() -> Style {
    Style::new()
}
pub const fn muted() -> Style {
    Style::new().fg(MUTED_FG)
}
pub const fn rule() -> Style {
    Style::new().fg(RULE_FG)
}
pub const fn gutter() -> Style {
    Style::new().fg(GUTTER_FG)
}
pub const fn banner_style() -> Style {
    Style::new().fg(H1_FG).bold()
}
pub const fn code() -> Style {
    Style::new().fg(CODE_FG).bg(CODE_BG)
}
pub const fn code_label() -> Style {
    Style::new().fg(MUTED_FG).bg(CODE_BG).italic()
}
pub const fn quote_bar() -> Style {
    Style::new().fg(QUOTE_BAR_FG).dim()
}
pub const fn table_border() -> Style {
    Style::new().fg(TABLE_BORDER_FG)
}
/// The status of a document, coloured by what it says.
///
/// Opinionated on purpose, and matched on the first word so a status carrying
/// a trailing explanation (`Superseded — co-location abandoned`) still reads
/// as superseded. Three states are worth distinguishing at a glance: live,
/// in flight, and historical. Anything unrecognised is left plain rather than
/// mis-coloured.
pub fn status_of(value: &str) -> Style {
    let first = value
        .split(|c: char| c.is_whitespace() || c == '|' || c == '\u{2014}')
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_ascii_lowercase();
    match first.as_str() {
        "active" | "accepted" | "implemented" | "done" => Style::new().fg(STATUS_LIVE).bold(),
        "draft" | "proposed" | "executing" | "in" | "partially" => {
            Style::new().fg(STATUS_OPEN).bold()
        }
        "superseded" | "archived" | "cancelled" | "rejected" => Style::new().fg(MUTED_FG).dim(),
        _ => Style::new(),
    }
}

/// The foreground a link is painted in, given whether it leaves the reader.
///
/// The one place the two colours are chosen between, so the inline renderer, a
/// code span inside a link and the focused-link style cannot disagree about
/// which link is which.
pub const fn link_fg(external: bool) -> u8 {
    match external {
        true => LINK_EXTERNAL,
        false => LINK,
    }
}

pub const fn more() -> Style {
    Style::new().fg(MORE_FG).bold()
}
pub const fn table_head() -> Style {
    Style::new().fg(TABLE_HEAD_FG).bold()
}
pub const fn bullet_style() -> Style {
    Style::new().fg(ACCENT)
}
pub const fn task_done() -> Style {
    Style::new().fg(TASK_DONE_FG)
}
pub const fn search() -> Style {
    Style::new().fg(SEARCH_FG).bg(SEARCH_BG)
}
pub const fn search_current() -> Style {
    Style::new().fg(SEARCH_FG).bg(SEARCH_CURRENT_BG)
}
pub const fn selection() -> Style {
    Style::new().bg(SELECTION_BG)
}
pub const fn status() -> Style {
    Style::new().fg(STATUS_FG).bg(STATUS_BG)
}

/// Style of a heading of `level` (1-6). Levels out of range clamp to 6.
pub const fn heading(level: u8) -> Style {
    match level {
        1 => Style::new().fg(H1_FG).bold(),
        2 => Style::new().fg(H2_FG).bold(),
        3 => Style::new().fg(H3_FG).bold(),
        4 => Style::new().fg(H4_FG),
        5 => Style::new().fg(H5_FG).dim(),
        _ => Style::new().fg(H6_FG).dim().italic(),
    }
}

/// Left indent (columns, inside the gutter) of a heading of `level`.
pub const fn heading_indent(level: u8) -> usize {
    match level {
        1 | 2 => 0,
        3 => 2,
        4 => 4,
        5 => 6,
        _ => 8,
    }
}

/// Collapse gutter markers (SPEC.md §Headings).
pub const MARKER_OPEN: char = '\u{25be}'; // ▾
pub const MARKER_CLOSED: char = '\u{25b8}'; // ▸

// ---------------------------------------------------------------------------
// JSON tree (SPEC.md §JSON, "The tree")
// ---------------------------------------------------------------------------
//
// "Keys, strings, numbers, booleans and null are coloured distinctly." Five
// colours that stay apart on a dark and on a light terminal, plus a dim one for
// the punctuation so the structure recedes behind the data.

pub const JSON_KEY_FG: u8 = 81;
pub const JSON_STR_FG: u8 = 150;
pub const JSON_NUM_FG: u8 = 215;
pub const JSON_BOOL_FG: u8 = 141;
pub const JSON_NULL_FG: u8 = MUTED_FG;
pub const JSON_PUNCT_FG: u8 = RULE_FG;
/// A row that could not be read: a member past the size cap, or invalid JSON.
pub const ERROR_FG: u8 = 203;

pub const fn json_key() -> Style {
    Style::new().fg(JSON_KEY_FG)
}
pub const fn json_string() -> Style {
    Style::new().fg(JSON_STR_FG)
}
pub const fn json_number() -> Style {
    Style::new().fg(JSON_NUM_FG)
}
pub const fn json_bool() -> Style {
    Style::new().fg(JSON_BOOL_FG).bold()
}
pub const fn json_null() -> Style {
    Style::new().fg(JSON_NULL_FG).italic()
}
pub const fn json_punct() -> Style {
    Style::new().fg(JSON_PUNCT_FG)
}
/// The `▾`/`▸` in front of a container row.
pub const fn json_marker() -> Style {
    Style::new().fg(GUTTER_FG)
}
pub const fn error() -> Style {
    Style::new().fg(ERROR_FG)
}

// ---------------------------------------------------------------------------
// Lenses (SPEC.md §Lenses)
// ---------------------------------------------------------------------------
//
// A trajectory read through a lens is a conversation, so the speaker is what
// the eye should find first: one colour per actor, reusing the JSON palette's
// hues rather than starting a second one, and everything that is not speech —
// the clock, the group summary — in the same muted grey the tree's punctuation
// uses.

pub const fn lens_user() -> Style {
    Style::new().fg(JSON_KEY_FG).bold()
}
pub const fn lens_assistant() -> Style {
    Style::new().fg(STATUS_LIVE).bold()
}
pub const fn lens_tool() -> Style {
    Style::new().fg(JSON_NUM_FG)
}
pub const fn lens_system() -> Style {
    Style::new().fg(MUTED_FG)
}
/// The clock on a summary row.
pub const fn lens_time() -> Style {
    Style::new().fg(GUTTER_FG)
}
/// `⟨6 steps · 4 tool calls⟩` — a count of what is folded away, not content.
pub const fn lens_group() -> Style {
    Style::new().fg(MUTED_FG).italic()
}
/// The message itself, under its summary row. Plain: this is the document, and
/// the summary above it is the headline.
pub const fn lens_body() -> Style {
    Style::new()
}
/// `⋯ +12 lines` — what a clipped body is not showing. A notice, like the
/// group count, and never mistakable for the message it stands at the end of.
pub const fn lens_more() -> Style {
    Style::new().fg(MUTED_FG).italic()
}

/// Stands in for a row's left border when the row carries more fields than the
/// header named. It replaces the bar rather than sitting beside it: a grid that
/// shifted by a column on some rows would be worse than the problem it reports.
pub const MARKER_MORE: char = '+';

/// Bullet glyph for a list nesting depth (0-based), cycling `•` `◦` `▪`.
pub const fn bullet(depth: usize) -> char {
    match depth % 3 {
        0 => '\u{2022}', // •
        1 => '\u{25e6}', // ◦
        _ => '\u{25aa}', // ▪
    }
}

/// Task-list checkbox glyph.
pub const fn checkbox(checked: bool) -> char {
    if checked {
        '\u{2611}' // ☑
    } else {
        '\u{2610}' // ☐
    }
}

// ---------------------------------------------------------------------------
// Banner glyph font
// ---------------------------------------------------------------------------

/// Rows per glyph.
pub const BANNER_ROWS: usize = 5;
/// Columns per glyph.
const GLYPH_W: usize = 3;
/// Blank columns between glyphs.
const GLYPH_GAP: usize = 1;

/// 3x5 block font. `#` is an inked cell, `.` blank. Lowercase input is folded
/// to uppercase before lookup, so only the uppercase forms are stored.
#[rustfmt::skip]
const GLYPHS: &[(char, [&str; BANNER_ROWS])] = &[
    (' ', ["...", "...", "...", "...", "..."]),
    ('A', [".#.", "#.#", "###", "#.#", "#.#"]),
    ('B', ["##.", "#.#", "##.", "#.#", "##."]),
    ('C', [".##", "#..", "#..", "#..", ".##"]),
    ('D', ["##.", "#.#", "#.#", "#.#", "##."]),
    ('E', ["###", "#..", "##.", "#..", "###"]),
    ('F', ["###", "#..", "##.", "#..", "#.."]),
    ('G', [".##", "#..", "#.#", "#.#", ".##"]),
    ('H', ["#.#", "#.#", "###", "#.#", "#.#"]),
    ('I', ["###", ".#.", ".#.", ".#.", "###"]),
    ('J', ["..#", "..#", "..#", "#.#", ".#."]),
    ('K', ["#.#", "##.", "#..", "##.", "#.#"]),
    ('L', ["#..", "#..", "#..", "#..", "###"]),
    ('M', ["#.#", "###", "###", "#.#", "#.#"]),
    ('N', ["#.#", "###", "###", "###", "#.#"]),
    ('O', ["###", "#.#", "#.#", "#.#", "###"]),
    ('P', ["##.", "#.#", "##.", "#..", "#.."]),
    ('Q', ["###", "#.#", "#.#", "##.", ".##"]),
    ('R', ["##.", "#.#", "##.", "#.#", "#.#"]),
    ('S', [".##", "#..", ".#.", "..#", "##."]),
    ('T', ["###", ".#.", ".#.", ".#.", ".#."]),
    ('U', ["#.#", "#.#", "#.#", "#.#", "###"]),
    ('V', ["#.#", "#.#", "#.#", "#.#", ".#."]),
    ('W', ["#.#", "#.#", "###", "###", "#.#"]),
    ('X', ["#.#", "#.#", ".#.", "#.#", "#.#"]),
    ('Y', ["#.#", "#.#", ".#.", ".#.", ".#."]),
    ('Z', ["###", "..#", ".#.", "#..", "###"]),
    ('0', ["###", "#.#", "#.#", "#.#", "###"]),
    ('1', [".#.", "##.", ".#.", ".#.", "###"]),
    ('2', ["##.", "..#", ".#.", "#..", "###"]),
    ('3', ["###", "..#", ".##", "..#", "###"]),
    ('4', ["#.#", "#.#", "###", "..#", "..#"]),
    ('5', ["###", "#..", "##.", "..#", "##."]),
    ('6', [".##", "#..", "###", "#.#", "###"]),
    ('7', ["###", "..#", "..#", ".#.", ".#."]),
    ('8', ["###", "#.#", "###", "#.#", "###"]),
    ('9', ["###", "#.#", "###", "..#", "##."]),
    ('-', ["...", "...", "###", "...", "..."]),
    ('.', ["...", "...", "...", "...", ".#."]),
    (',', ["...", "...", "...", ".#.", "#.."]),
    (':', ["...", ".#.", "...", ".#.", "..."]),
    ('!', [".#.", ".#.", ".#.", "...", ".#."]),
    ('?', ["##.", "..#", ".#.", "...", ".#."]),
    ('\'', [".#.", ".#.", "...", "...", "..."]),
    ('/', ["..#", "..#", ".#.", "#..", "#.."]),
    ('(', ["..#", ".#.", ".#.", ".#.", "..#"]),
    (')', ["#..", ".#.", ".#.", ".#.", "#.."]),
];

/// The inked cell character.
const INK: char = '\u{2588}'; // █

fn glyph(c: char) -> Option<&'static [&'static str; BANNER_ROWS]> {
    let up = c.to_uppercase().next().unwrap_or(c);
    GLYPHS.iter().find(|(g, _)| *g == up).map(|(_, rows)| rows)
}

/// Display width a banner for `text` would occupy, or `None` when a character
/// has no glyph.
pub fn banner_width(text: &str) -> Option<usize> {
    let n = text.chars().count();
    if n == 0 {
        return None;
    }
    for c in text.chars() {
        glyph(c)?;
    }
    Some(n * GLYPH_W + (n - 1) * GLYPH_GAP)
}

/// Render `text` as a block-glyph banner: `BANNER_ROWS` equal-width rows.
///
/// Returns `None` when the text has unmappable characters or the banner would
/// not fit in `max_width` — the caller then falls back to bold-uppercase with
/// a rule (SPEC.md §Headings).
pub fn banner(text: &str, max_width: usize) -> Option<Vec<String>> {
    let trimmed = text.trim();
    let width = banner_width(trimmed)?;
    if width > max_width {
        return None;
    }
    let glyphs: Vec<&[&str; BANNER_ROWS]> = trimmed.chars().filter_map(glyph).collect();
    let mut out = Vec::with_capacity(BANNER_ROWS);
    for row in 0..BANNER_ROWS {
        let mut line = String::with_capacity(width);
        for (i, g) in glyphs.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            for cell in g[row].chars() {
                line.push(if cell == '#' { INK } else { ' ' });
            }
        }
        out.push(line);
    }
    Some(out)
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
