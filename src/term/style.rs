//! ANSI style value type and the SGR diffing writer.
#![deny(unsafe_code)]

pub const BOLD: u8 = 1 << 0;
pub const DIM: u8 = 1 << 1;
pub const ITALIC: u8 = 1 << 2;
pub const UNDERLINE: u8 = 1 << 3;
pub const STRIKE: u8 = 1 << 4;
pub const REVERSE: u8 = 1 << 5;

/// A value-type text style: 256-colour fg/bg plus attribute bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Style {
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    pub attrs: u8,
}

impl Style {
    pub const fn new() -> Self {
        Style {
            fg: None,
            bg: None,
            attrs: 0,
        }
    }
    pub const fn fg(mut self, c: u8) -> Self {
        self.fg = Some(c);
        self
    }
    pub const fn bg(mut self, c: u8) -> Self {
        self.bg = Some(c);
        self
    }
    /// Turn on one or more attribute bits.
    pub const fn with(mut self, bits: u8) -> Self {
        self.attrs |= bits;
        self
    }
    pub const fn bold(self) -> Self {
        self.with(BOLD)
    }
    pub const fn dim(self) -> Self {
        self.with(DIM)
    }
    pub const fn italic(self) -> Self {
        self.with(ITALIC)
    }
    pub const fn underline(self) -> Self {
        self.with(UNDERLINE)
    }
    pub const fn strike(self) -> Self {
        self.with(STRIKE)
    }
    pub const fn reverse(self) -> Self {
        self.with(REVERSE)
    }
    pub const fn is_default(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && self.attrs == 0
    }
    pub const fn has(&self, bit: u8) -> bool {
        self.attrs & bit != 0
    }
}

fn attr_on_code(bit: u8) -> &'static str {
    match bit {
        BOLD => "1",
        DIM => "2",
        ITALIC => "3",
        UNDERLINE => "4",
        REVERSE => "7",
        STRIKE => "9",
        _ => "",
    }
}

const ATTR_BITS: [u8; 6] = [BOLD, DIM, ITALIC, UNDERLINE, STRIKE, REVERSE];

/// Append the SGR sequence that moves the terminal from `from` to `to`.
///
/// Emits nothing when the styles are equal. Attributes can only be *added*
/// incrementally in SGR, so any attribute removal (or a colour going back to
/// the default) forces a `0` reset followed by the full target style.
pub fn write_transition(out: &mut String, from: Style, to: Style) {
    if from == to {
        return;
    }
    let removes = from.attrs & !to.attrs;
    let full_reset = removes != 0
        || (from.fg.is_some() && to.fg.is_none())
        || (from.bg.is_some() && to.bg.is_none());
    let mut parts: Vec<String> = Vec::new();
    let base = if full_reset {
        parts.push("0".to_string());
        Style::new()
    } else {
        from
    };
    for bit in ATTR_BITS {
        if to.has(bit) && !base.has(bit) {
            parts.push(attr_on_code(bit).to_string());
        }
    }
    if to.fg != base.fg {
        match to.fg {
            Some(c) => parts.push(format!("38;5;{}", c)),
            None => parts.push("39".to_string()),
        }
    }
    if to.bg != base.bg {
        match to.bg {
            Some(c) => parts.push(format!("48;5;{}", c)),
            None => parts.push("49".to_string()),
        }
    }
    if parts.is_empty() {
        return;
    }
    out.push_str("\x1b[");
    out.push_str(&parts.join(";"));
    out.push('m');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(from: Style, to: Style) -> String {
        let mut s = String::new();
        write_transition(&mut s, from, to);
        s
    }

    #[test]
    fn equal_styles_emit_nothing() {
        assert_eq!(t(Style::new(), Style::new()), "");
        let s = Style::new().fg(9).bold();
        assert_eq!(t(s, s), "");
    }

    #[test]
    fn adding_attributes_is_incremental() {
        assert_eq!(t(Style::new().fg(4), Style::new().fg(4).bold()), "\x1b[1m");
        assert_eq!(
            t(Style::new().bold(), Style::new().bold().underline()),
            "\x1b[4m"
        );
    }

    #[test]
    fn removing_attributes_resets_then_reapplies() {
        assert_eq!(
            t(Style::new().bold().underline(), Style::new().bold()),
            "\x1b[0;1m"
        );
    }

    #[test]
    fn colour_changes_only_emit_the_colour() {
        assert_eq!(t(Style::new().fg(1), Style::new().fg(2)), "\x1b[38;5;2m");
        assert_eq!(
            t(Style::new().fg(1), Style::new().fg(1).bg(8)),
            "\x1b[48;5;8m"
        );
    }

    #[test]
    fn dropping_a_colour_resets() {
        assert_eq!(t(Style::new().fg(2), Style::new()), "\x1b[0m");
        assert_eq!(t(Style::new().bg(2), Style::new().fg(3)), "\x1b[0;38;5;3m");
    }

    #[test]
    fn full_style_from_default() {
        let s = Style::new().fg(7).bg(236).bold().italic().strike();
        assert_eq!(t(Style::new(), s), "\x1b[1;3;9;38;5;7;48;5;236m");
    }

}
