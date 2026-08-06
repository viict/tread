//! Byte-level escape-sequence and UTF-8 parsing for [`super::Decoder`].
#![deny(unsafe_code)]

use super::{Key, KeyEvent, Mods, Step, ESC};

pub(crate) fn step(b: &[u8]) -> Step {
    match b[0] {
        ESC => escape(b),
        c if c < 0x20 || c == 0x7f => Step::Emit(KeyEvent::plain(control(c)), 1),
        _ => utf8(b),
    }
}

fn control(c: u8) -> Key {
    match c {
        0x00 => Key::Ctrl(' '),
        0x08 | 0x7f => Key::Backspace,
        0x09 => Key::Tab,
        0x0a | 0x0d => Key::Enter,
        0x01..=0x1a => Key::Ctrl((b'a' + c - 1) as char),
        0x1c => Key::Ctrl('\\'),
        0x1d => Key::Ctrl(']'),
        0x1e => Key::Ctrl('^'),
        0x1f => Key::Ctrl('_'),
        _ => Key::Unknown,
    }
}

/// Assemble one UTF-8 scalar. Invalid lead/continuation bytes yield
/// [`Key::Unknown`] and consume exactly one byte, so the stream always makes
/// progress and a malformed paste can never wedge the decoder.
fn utf8(b: &[u8]) -> Step {
    let len = match b[0] {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Step::Emit(KeyEvent::plain(Key::Unknown), 1),
    };
    if b.len() < len {
        // A truncated read is only plausible if what we have is all valid
        // continuation bytes; otherwise the scalar is simply malformed.
        if b[1..].iter().all(|&c| (0x80..0xc0).contains(&c)) {
            return Step::Need;
        }
        return Step::Emit(KeyEvent::plain(Key::Unknown), 1);
    }
    match std::str::from_utf8(&b[..len])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(c) => Step::Emit(KeyEvent::plain(Key::Char(c)), len),
        None => Step::Emit(KeyEvent::plain(Key::Unknown), 1),
    }
}

fn escape(b: &[u8]) -> Step {
    if b.len() < 2 {
        return Step::Need;
    }
    match b[1] {
        b'[' => csi(b),
        b'O' => ss3(b),
        b'P' | b']' | b'^' | b'_' => string_terminated(b),
        ESC => Step::Emit(KeyEvent::plain(Key::Esc), 1),
        _ => alt_prefixed(b),
    }
}

fn alt_prefixed(b: &[u8]) -> Step {
    match utf8(&b[1..]) {
        Step::Emit(ev, n) => {
            let key = match ev.key {
                Key::Char(c) => Key::Alt(c),
                other => other,
            };
            Step::Emit(KeyEvent::with(key, Mods::ALT), n + 1)
        }
        Step::Need => Step::Need,
        Step::Skip(n) => Step::Skip(n + 1),
    }
}

/// Swallow DCS/OSC/PM/APC strings (terminal query replies) up to ST or BEL.
fn string_terminated(b: &[u8]) -> Step {
    let mut i = 2;
    while i < b.len() {
        if b[i] == 0x07 {
            return Step::Skip(i + 1);
        }
        if b[i] == ESC && i + 1 < b.len() && b[i + 1] == b'\\' {
            return Step::Skip(i + 2);
        }
        i += 1;
    }
    Step::Need
}

fn ss3(b: &[u8]) -> Step {
    if b.len() < 3 {
        return Step::Need;
    }
    let key = match b[2] {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'M' => Key::Enter,
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        _ => Key::Unknown,
    };
    Step::Emit(KeyEvent::plain(key), 3)
}

/// Parsed shape of a CSI sequence.
struct Csi {
    private: Option<u8>,
    params: Vec<u32>,
    final_byte: u8,
    len: usize,
}

fn parse_csi(b: &[u8]) -> Option<Csi> {
    let mut i = 2usize;
    let mut private = None;
    if i < b.len() && matches!(b[i], b'?' | b'<' | b'>' | b'=') {
        private = Some(b[i]);
        i += 1;
    }
    let mut params: Vec<u32> = Vec::new();
    let mut cur: Option<u32> = None;
    while i < b.len() {
        let c = b[i];
        match c {
            b'0'..=b'9' => {
                let v = cur.unwrap_or(0);
                cur = Some(v.saturating_mul(10).saturating_add((c - b'0') as u32));
                i += 1;
            }
            b';' | b':' => {
                params.push(cur.take().unwrap_or(0));
                i += 1;
            }
            0x20..=0x2f => i += 1, // intermediate bytes
            0x40..=0x7e => {
                if let Some(v) = cur {
                    params.push(v);
                }
                return Some(Csi {
                    private,
                    params,
                    final_byte: c,
                    len: i + 1,
                });
            }
            _ => return None,
        }
    }
    None
}

/// True while the buffer is still a possible prefix of a CSI sequence.
fn csi_could_continue(b: &[u8]) -> bool {
    b[2..]
        .iter()
        .all(|&c| matches!(c, b'0'..=b'9' | b';' | b':' | b'?' | b'<' | b'>' | b'=' | 0x20..=0x2f))
}

fn csi(b: &[u8]) -> Step {
    let c = match parse_csi(b) {
        Some(c) => c,
        None => {
            return if csi_could_continue(b) {
                Step::Need
            } else {
                // Not a CSI at all: the ESC stood alone, reprocess the rest.
                Step::Emit(KeyEvent::plain(Key::Esc), 1)
            };
        }
    };
    if let Some(step) = mouse_report(b, &c) {
        return step;
    }
    if c.final_byte == b'~' {
        return tilde(&c);
    }
    if c.final_byte == b'u' {
        return csi_u(&c);
    }
    let mods = c
        .params
        .get(1)
        .copied()
        .map(Mods::from_param)
        .unwrap_or(Mods::NONE);
    let key = match c.final_byte {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => return Step::Emit(KeyEvent::plain(Key::BackTab), c.len),
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        _ => Key::Unknown,
    };
    Step::Emit(KeyEvent::with(key, mods), c.len)
}

/// Mouse tracking is never enabled, but a stray report left over from another
/// program must be swallowed rather than typed into the document.
fn mouse_report(b: &[u8], c: &Csi) -> Option<Step> {
    if c.private == Some(b'<') {
        return Some(Step::Skip(c.len));
    }
    // X10 mouse: `ESC [ M` followed by exactly three raw bytes.
    if c.final_byte == b'M' && c.private.is_none() && c.params.is_empty() {
        return Some(if b.len() >= c.len + 3 {
            Step::Skip(c.len + 3)
        } else {
            Step::Need
        });
    }
    None
}

fn tilde(c: &Csi) -> Step {
    let n = c.params.first().copied().unwrap_or(0);
    let mods = c
        .params
        .get(1)
        .copied()
        .map(Mods::from_param)
        .unwrap_or(Mods::NONE);
    // Bracketed-paste markers are detected and swallowed.
    if n == 200 || n == 201 {
        return Step::Skip(c.len);
    }
    let key = match n {
        1 | 7 => Key::Home,
        2 => Key::Insert,
        3 => Key::Delete,
        4 | 8 => Key::End,
        5 => Key::PageUp,
        6 => Key::PageDown,
        11..=15 => Key::F((n - 10) as u8),
        17..=21 => Key::F((n - 11) as u8),
        23..=26 => Key::F((n - 12) as u8),
        _ => Key::Unknown,
    };
    Step::Emit(KeyEvent::with(key, mods), c.len)
}

/// kitty / foot `CSI <codepoint> ; <mods> u`.
fn csi_u(c: &Csi) -> Step {
    let cp = c.params.first().copied().unwrap_or(0);
    let mods = c
        .params
        .get(1)
        .copied()
        .map(Mods::from_param)
        .unwrap_or(Mods::NONE);
    let key = match char::from_u32(cp) {
        Some('\r') | Some('\n') => Key::Enter,
        Some('\t') if mods.shift => Key::BackTab,
        Some('\t') => Key::Tab,
        Some('\x7f') | Some('\u{8}') => Key::Backspace,
        Some('\x1b') => Key::Esc,
        Some(ch) if mods.ctrl => Key::Ctrl(ch.to_ascii_lowercase()),
        Some(ch) if mods.alt => Key::Alt(ch),
        Some(ch) => Key::Char(ch),
        None => Key::Unknown,
    };
    Step::Emit(KeyEvent::with(key, mods), c.len)
}
