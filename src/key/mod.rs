//! Terminal input byte-stream -> [`Key`] decoder. Pure, zero `unsafe`, no I/O:
//! everything is driven through [`decode`] / [`Decoder::feed`], so the whole
//! module is unit-testable without a terminal.
#![deny(unsafe_code)]

mod parse;
#[cfg(test)]
mod tests;

pub(crate) const ESC: u8 = 0x1b;

/// Modifier bits reported by CSI parameters (`CSI 1;5A` == ctrl+Up).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
    };
    pub const ALT: Mods = Mods {
        shift: false,
        alt: true,
        ctrl: false,
    };

    /// xterm encodes modifiers as `1 + bitmask`.
    pub fn from_param(p: u32) -> Mods {
        let b = p.saturating_sub(1);
        Mods {
            shift: b & 1 != 0,
            alt: b & 2 != 0,
            ctrl: b & 4 != 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Key {
    Char(char),
    /// Control letter, normalised to lowercase (`Ctrl('c')` for 0x03).
    Ctrl(char),
    /// ESC-prefixed printable (Meta/Alt).
    Alt(char),
    Enter,
    Tab,
    BackTab,
    Backspace,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    /// F1..F12.
    F(u8),
    /// A well-formed but unrecognised sequence.
    Unknown,
}

/// A decoded key plus its modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Mods,
}

impl KeyEvent {
    pub const fn plain(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Mods::NONE,
        }
    }
    pub const fn with(key: Key, mods: Mods) -> KeyEvent {
        KeyEvent { key, mods }
    }
}

/// One decoding step over the front of a byte slice.
pub(crate) enum Step {
    /// Consumed `n` bytes, produced an event.
    Emit(KeyEvent, usize),
    /// Consumed `n` bytes, produced nothing (paste marker, query reply, ...).
    Skip(usize),
    /// Incomplete: need more bytes.
    Need,
}

/// Decode as much of `bytes` as forms complete input.
///
/// Returns the events plus the *leftover* tail: a prefix of an escape sequence
/// (or of a multi-byte UTF-8 scalar) that cannot yet be resolved. A trailing
/// bare `ESC` is always leftover here — deciding whether it is a lone Esc is
/// the caller's (or [`Decoder`]'s) job, which keeps this function timeout-free.
pub fn decode(bytes: &[u8]) -> (Vec<KeyEvent>, &[u8]) {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match parse::step(&bytes[i..]) {
            Step::Need => return (out, &bytes[i..]),
            Step::Skip(n) => i += n,
            Step::Emit(ev, n) => {
                out.push(ev);
                i += n;
            }
        }
    }
    (out, &bytes[bytes.len()..])
}

/// Convenience wrapper over [`decode`] that discards modifier information.
/// Test-only: the pager needs the modifiers.
#[cfg(test)]
pub fn decode_keys(bytes: &[u8]) -> (Vec<Key>, &[u8]) {
    let (evs, rest) = decode(bytes);
    (evs.into_iter().map(|e| e.key).collect(), rest)
}

/// Buffers partial sequences between reads.
///
/// [`Decoder::feed`] treats each call as one complete `read(2)` burst: a
/// trailing *bare* `ESC` is reported as [`Key::Esc`] immediately, because a real
/// escape sequence always arrives in the same burst as its introducer. Any
/// longer partial tail (`ESC [`, `ESC [ 1 ;`, a truncated UTF-8 scalar, ...) is
/// held for the next call. No timers are involved anywhere.
#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder {
            pending: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<KeyEvent> {
        self.pending.extend_from_slice(bytes);
        let (mut evs, rest) = decode(&self.pending);
        let mut leftover = rest.to_vec();
        if leftover.len() == 1 && leftover[0] == ESC {
            evs.push(KeyEvent::plain(Key::Esc));
            leftover.clear();
        }
        self.pending = leftover;
        evs
    }

    /// Same as [`Decoder::feed`] but discards modifiers. Test-only.
    #[cfg(test)]
    pub fn feed_keys(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.feed(bytes).into_iter().map(|e| e.key).collect()
    }

    /// Bytes currently held as an incomplete sequence. Test-only.
    #[cfg(test)]
    pub fn pending(&self) -> &[u8] {
        &self.pending
    }
}
