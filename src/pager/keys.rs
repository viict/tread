//! The single source of truth for keybindings.
//!
//! [`BINDINGS`] is used by *both* the dispatcher ([`lookup`]) and the help
//! overlay ([`help_rows`]), so the help text can never drift from what the keys
//! actually do (SPEC.md §Keybindings).
#![deny(unsafe_code)]

use crate::key::{Key, KeyEvent};

/// Everything the pager can be asked to do by a key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Ctrl-C: leave immediately, without stepping back through the nav stack.
    ForceQuit,
    LineDown,
    LineUp,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    ScrollLeft,
    ScrollRight,
    /// `←`: scroll left on a row that scrolls, otherwise move the link focus
    /// back along the current row (SPEC.md §"Selecting links on a line").
    ArrowLeft,
    /// `→`: the same rule, forwards.
    ArrowRight,
    /// `w`: widen the column under the cursor, where the format has columns.
    Widen,
    /// Show or hide what the format hides by default (a listing's dotfiles).
    ToggleHidden,
    ToggleCollapse,
    /// `zt`: open the raw record under the cursor, where a row stands for one.
    OpenTree,
    OpenSection,
    CloseSection,
    CollapseAll,
    ExpandAll,
    NextHeading,
    PrevHeading,
    Outline,
    Help,
    /// Enter: follow the focused link, else toggle the section.
    Follow,
    Back,
    Forward,
    OpenIndex,
    NextDoc,
    PrevDoc,
    SearchForward,
    SearchBackward,
    NextMatch,
    PrevMatch,
    /// Enter/leave visual line-select mode.
    Visual,
    /// Yank the visual selection.
    Yank,
    /// Yank the whole section under the cursor.
    YankSection,
    /// Yank the code block under (or nearest below) the cursor, verbatim.
    YankCode,
}

/// One key (optionally behind a prefix such as `z`) that fires an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub prefix: Option<char>,
    pub key: Key,
}

impl Trigger {
    const fn k(key: Key) -> Trigger {
        Trigger { prefix: None, key }
    }
    const fn c(ch: char) -> Trigger {
        Trigger {
            prefix: None,
            key: Key::Char(ch),
        }
    }
    const fn z(ch: char) -> Trigger {
        Trigger {
            prefix: Some('z'),
            key: Key::Char(ch),
        }
    }
}

/// A row of the keymap: what to press, what it does, and the action fired.
pub struct Binding {
    /// Human-readable key list, shown in the help overlay.
    pub keys: &'static str,
    pub desc: &'static str,
    pub action: Action,
    pub triggers: &'static [Trigger],
}

/// Keys that introduce a chord. Only `z` today.
pub const PREFIXES: &[char] = &['z'];

pub fn is_prefix(ev: KeyEvent) -> bool {
    match ev.key {
        Key::Char(c) => !ev.mods.ctrl && !ev.mods.alt && PREFIXES.contains(&c),
        _ => false,
    }
}

use Action as A;

pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: "j / \u{2193}",
        desc: "line down",
        action: A::LineDown,
        triggers: &[Trigger::c('j'), Trigger::k(Key::Down)],
    },
    Binding {
        keys: "k / \u{2191}",
        desc: "line up",
        action: A::LineUp,
        triggers: &[Trigger::c('k'), Trigger::k(Key::Up)],
    },
    Binding {
        keys: "d",
        desc: "half page down",
        action: A::HalfDown,
        triggers: &[Trigger::c('d')],
    },
    Binding {
        keys: "u",
        desc: "half page up",
        action: A::HalfUp,
        triggers: &[Trigger::c('u')],
    },
    Binding {
        keys: "space / f",
        desc: "page down",
        action: A::PageDown,
        triggers: &[Trigger::c(' '), Trigger::c('f'), Trigger::k(Key::PageDown)],
    },
    Binding {
        keys: "b",
        desc: "page up",
        action: A::PageUp,
        triggers: &[Trigger::c('b'), Trigger::k(Key::PageUp)],
    },
    Binding {
        keys: "g",
        desc: "top of document",
        action: A::Top,
        triggers: &[Trigger::c('g'), Trigger::k(Key::Home)],
    },
    Binding {
        keys: "G",
        desc: "bottom of document",
        action: A::Bottom,
        triggers: &[Trigger::c('G'), Trigger::k(Key::End)],
    },
    Binding {
        keys: "h",
        desc: "scroll left \u{2014} code, wide tables, one column",
        action: A::ScrollLeft,
        triggers: &[Trigger::c('h')],
    },
    Binding {
        keys: "l",
        desc: "scroll right \u{2014} code, wide tables, one column",
        action: A::ScrollRight,
        triggers: &[Trigger::c('l')],
    },
    // `h`/`l` scroll everywhere; the arrows scroll only where scrolling means
    // something and otherwise walk the links on the row (SPEC.md §"Selecting
    // links on a line").
    Binding {
        keys: "\u{2190}",
        desc: "previous link on this row (scrolls left on a scrollable row)",
        action: A::ArrowLeft,
        triggers: &[Trigger::k(Key::Left)],
    },
    Binding {
        keys: "\u{2192}",
        desc: "next link on this row (scrolls right on a scrollable row)",
        action: A::ArrowRight,
        triggers: &[Trigger::k(Key::Right)],
    },
    Binding {
        keys: "w",
        desc: "widen the column under the cursor to fit the screen",
        action: A::Widen,
        triggers: &[Trigger::c('w')],
    },
    Binding {
        keys: "a",
        desc: "show or hide what this view hides (dotfiles, code bodies)",
        action: A::ToggleHidden,
        triggers: &[Trigger::c('a')],
    },
    Binding {
        keys: "za",
        desc: "toggle the section at the cursor",
        action: A::ToggleCollapse,
        triggers: &[Trigger::z('a')],
    },
    Binding {
        keys: "Enter",
        desc: "follow the focused link, open the row, else fold",
        action: A::Follow,
        triggers: &[Trigger::k(Key::Enter)],
    },
    Binding {
        keys: "zt",
        desc: "open the raw record under the cursor",
        action: A::OpenTree,
        triggers: &[Trigger::z('t')],
    },
    Binding {
        keys: "zo",
        desc: "open the section at the cursor",
        action: A::OpenSection,
        triggers: &[Trigger::z('o')],
    },
    Binding {
        keys: "zc",
        desc: "close the section at the cursor",
        action: A::CloseSection,
        triggers: &[Trigger::z('c')],
    },
    Binding {
        keys: "zM",
        desc: "collapse every section",
        action: A::CollapseAll,
        triggers: &[Trigger::z('M')],
    },
    Binding {
        keys: "zR",
        desc: "expand every section",
        action: A::ExpandAll,
        triggers: &[Trigger::z('R')],
    },
    Binding {
        keys: "Tab",
        desc: "next heading",
        action: A::NextHeading,
        triggers: &[Trigger::k(Key::Tab)],
    },
    Binding {
        keys: "S-Tab",
        desc: "previous heading",
        action: A::PrevHeading,
        triggers: &[Trigger::k(Key::BackTab)],
    },
    Binding {
        keys: "o",
        desc: "outline overlay",
        action: A::Outline,
        triggers: &[Trigger::c('o')],
    },
    Binding {
        keys: "/",
        desc: "search forward",
        action: A::SearchForward,
        triggers: &[Trigger::c('/')],
    },
    Binding {
        keys: "?",
        desc: "search backward",
        action: A::SearchBackward,
        triggers: &[Trigger::c('?')],
    },
    // `n`/`N` are search motions while a search is active and link motions
    // otherwise, which is what SPEC.md §Keybindings asks of them.
    Binding {
        keys: "n",
        desc: "next link (next search match while searching)",
        action: A::NextMatch,
        triggers: &[Trigger::c('n')],
    },
    Binding {
        keys: "N",
        desc: "previous link (previous match while searching)",
        action: A::PrevMatch,
        triggers: &[Trigger::c('N')],
    },
    Binding {
        keys: "Backspace / -",
        desc: "back in document history",
        action: A::Back,
        triggers: &[Trigger::k(Key::Backspace), Trigger::c('-')],
    },
    Binding {
        keys: "+",
        desc: "forward in document history",
        action: A::Forward,
        triggers: &[Trigger::c('+')],
    },
    Binding {
        keys: "i",
        desc: "corpus index (j/k move, Enter open, / filter, Esc close)",
        action: A::OpenIndex,
        triggers: &[Trigger::c('i')],
    },
    Binding {
        keys: "]",
        desc: "next document in index order",
        action: A::NextDoc,
        triggers: &[Trigger::c(']')],
    },
    Binding {
        keys: "[",
        desc: "previous document in index order",
        action: A::PrevDoc,
        triggers: &[Trigger::c('[')],
    },
    Binding {
        keys: "v",
        desc: "visual line select (j/k/d/u/g/G extend, Esc cancels)",
        action: A::Visual,
        triggers: &[Trigger::c('v')],
    },
    Binding {
        keys: "y",
        desc: "yank the selection, or the focused link's target",
        action: A::Yank,
        triggers: &[Trigger::c('y')],
    },
    Binding {
        keys: "Y",
        desc: "yank the section under the cursor",
        action: A::YankSection,
        triggers: &[Trigger::c('Y')],
    },
    Binding {
        keys: "c",
        desc: "yank the code block under the cursor, verbatim",
        action: A::YankCode,
        triggers: &[Trigger::c('c')],
    },
    Binding {
        keys: "F1 / H",
        desc: "this help",
        action: A::Help,
        triggers: &[Trigger::k(Key::F(1)), Trigger::c('H')],
    },
    Binding {
        keys: "q",
        desc: "quit (steps back first when the history is deep)",
        action: A::Quit,
        triggers: &[Trigger::c('q')],
    },
    // Raw mode clears ISIG, so Ctrl-C never becomes SIGINT: it arrives as a
    // key and has to be bound here or it does nothing at all.
    Binding {
        keys: "Ctrl-C",
        desc: "quit immediately, whatever the history depth",
        action: A::ForceQuit,
        triggers: &[Trigger::k(Key::Ctrl('c'))],
    },
];

/// Resolve a key press (with any pending chord prefix) to an action.
pub fn lookup(prefix: Option<char>, ev: KeyEvent) -> Option<Action> {
    if ev.mods.ctrl || ev.mods.alt {
        return None;
    }
    BINDINGS
        .iter()
        .find(|b| {
            b.triggers
                .iter()
                .any(|t| t.prefix == prefix && t.key == ev.key)
        })
        .map(|b| b.action)
}

/// `(keys, description)` rows for the help overlay, in table order.
pub fn help_rows() -> Vec<(&'static str, &'static str)> {
    BINDINGS.iter().map(|b| (b.keys, b.desc)).collect()
}

/// Widest key column, for aligning the help overlay.
pub fn help_key_width() -> usize {
    BINDINGS
        .iter()
        .map(|b| crate::render::str_width(b.keys))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mods;

    fn ev(c: char) -> KeyEvent {
        KeyEvent::plain(Key::Char(c))
    }

    #[test]
    fn plain_keys_resolve() {
        assert_eq!(lookup(None, ev('j')), Some(A::LineDown));
        assert_eq!(lookup(None, KeyEvent::plain(Key::Down)), Some(A::LineDown));
        assert_eq!(lookup(None, ev('q')), Some(A::Quit));
        assert_eq!(lookup(None, ev('G')), Some(A::Bottom));
    }

    #[test]
    fn chords_need_their_prefix() {
        assert_eq!(lookup(Some('z'), ev('a')), Some(A::ToggleCollapse));
        // `a` is its own binding now, so the point is that it is *not* the
        // chord: pressing it without `z` must never fold a section.
        assert_eq!(lookup(None, ev('a')), Some(A::ToggleHidden));
        assert_ne!(lookup(None, ev('a')), Some(A::ToggleCollapse));
        assert_eq!(lookup(Some('z'), ev('j')), None);
        assert_eq!(lookup(Some('z'), ev('M')), Some(A::CollapseAll));
    }

    #[test]
    fn z_is_the_only_prefix() {
        assert!(is_prefix(ev('z')));
        assert!(!is_prefix(ev('j')));
        assert!(!is_prefix(KeyEvent::with(Key::Char('z'), Mods { ctrl: true, ..Mods::NONE })));
    }

    #[test]
    fn modified_keys_do_not_fire_bindings() {
        let ctrl_j = KeyEvent::with(Key::Char('j'), Mods { ctrl: true, ..Mods::NONE });
        assert_eq!(lookup(None, ctrl_j), None);
    }

    #[test]
    fn help_covers_every_binding_and_never_drifts() {
        assert_eq!(help_rows().len(), BINDINGS.len());
        for b in BINDINGS {
            assert!(!b.keys.is_empty() && !b.desc.is_empty());
            assert!(!b.triggers.is_empty(), "{} has no trigger", b.keys);
            // Every documented binding must be reachable by the dispatcher.
            for t in b.triggers {
                assert_eq!(lookup(t.prefix, KeyEvent::plain(t.key)), Some(b.action));
            }
        }
    }

    #[test]
    fn no_trigger_is_claimed_twice() {
        let mut seen: Vec<Trigger> = Vec::new();
        for b in BINDINGS {
            for t in b.triggers {
                assert!(!seen.contains(t), "duplicate trigger {t:?}");
                seen.push(*t);
            }
        }
    }
}

