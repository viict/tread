//! Unit tests for the key decoder. No terminal required.
#![deny(unsafe_code)]

use super::*;

fn k(bytes: &[u8]) -> Vec<Key> {
    Decoder::new().feed_keys(bytes)
}

fn ev(bytes: &[u8]) -> Vec<KeyEvent> {
    Decoder::new().feed(bytes)
}

#[test]
fn plain_ascii() {
    assert_eq!(
        k(b"abc"),
        vec![Key::Char('a'), Key::Char('b'), Key::Char('c')]
    );
    assert_eq!(
        k(b"jkgG/"),
        vec![
            Key::Char('j'),
            Key::Char('k'),
            Key::Char('g'),
            Key::Char('G'),
            Key::Char('/'),
        ]
    );
}

#[test]
fn control_characters() {
    assert_eq!(k(b"\x03"), vec![Key::Ctrl('c')]);
    assert_eq!(k(b"\x04"), vec![Key::Ctrl('d')]);
    assert_eq!(k(b"\x0d"), vec![Key::Enter]);
    assert_eq!(k(b"\x0a"), vec![Key::Enter]);
    assert_eq!(k(b"\x09"), vec![Key::Tab]);
    assert_eq!(k(b"\x7f"), vec![Key::Backspace]);
    assert_eq!(k(b"\x08"), vec![Key::Backspace]);
    assert_eq!(k(b"\x00"), vec![Key::Ctrl(' ')]);
    assert_eq!(k(b"\x1f"), vec![Key::Ctrl('_')]);
    assert_eq!(
        k(b"\x1c\x1d\x1e"),
        vec![Key::Ctrl('\\'), Key::Ctrl(']'), Key::Ctrl('^')]
    );
}

#[test]
fn csi_arrows() {
    assert_eq!(k(b"\x1b[A"), vec![Key::Up]);
    assert_eq!(k(b"\x1b[B"), vec![Key::Down]);
    assert_eq!(k(b"\x1b[C"), vec![Key::Right]);
    assert_eq!(k(b"\x1b[D"), vec![Key::Left]);
}

#[test]
fn ss3_arrows_and_function_keys() {
    assert_eq!(k(b"\x1bOA"), vec![Key::Up]);
    assert_eq!(k(b"\x1bOB"), vec![Key::Down]);
    assert_eq!(k(b"\x1bOC"), vec![Key::Right]);
    assert_eq!(k(b"\x1bOD"), vec![Key::Left]);
    assert_eq!(k(b"\x1bOH"), vec![Key::Home]);
    assert_eq!(k(b"\x1bOF"), vec![Key::End]);
    assert_eq!(k(b"\x1bOM"), vec![Key::Enter]);
}

#[test]
fn all_twelve_function_keys() {
    let seqs: [(&[u8], u8); 12] = [
        (b"\x1bOP", 1),
        (b"\x1bOQ", 2),
        (b"\x1bOR", 3),
        (b"\x1bOS", 4),
        (b"\x1b[15~", 5),
        (b"\x1b[17~", 6),
        (b"\x1b[18~", 7),
        (b"\x1b[19~", 8),
        (b"\x1b[20~", 9),
        (b"\x1b[21~", 10),
        (b"\x1b[23~", 11),
        (b"\x1b[24~", 12),
    ];
    for (bytes, n) in seqs {
        assert_eq!(k(bytes), vec![Key::F(n)], "seq {bytes:?}");
    }
    // The alternate linux-console / CSI encoding for F1-F4.
    assert_eq!(
        k(b"\x1b[11~\x1b[12~\x1b[13~\x1b[14~"),
        vec![Key::F(1), Key::F(2), Key::F(3), Key::F(4)]
    );
    // xterm's modified-F1 form.
    assert_eq!(ev(b"\x1b[1;2P")[0].key, Key::F(1));
}

#[test]
fn navigation_tilde_keys() {
    assert_eq!(k(b"\x1b[1~"), vec![Key::Home]);
    assert_eq!(k(b"\x1b[2~"), vec![Key::Insert]);
    assert_eq!(k(b"\x1b[3~"), vec![Key::Delete]);
    assert_eq!(k(b"\x1b[4~"), vec![Key::End]);
    assert_eq!(k(b"\x1b[5~"), vec![Key::PageUp]);
    assert_eq!(k(b"\x1b[6~"), vec![Key::PageDown]);
    assert_eq!(k(b"\x1b[7~\x1b[8~"), vec![Key::Home, Key::End]);
    assert_eq!(k(b"\x1b[H\x1b[F"), vec![Key::Home, Key::End]);
}

#[test]
fn back_tab() {
    assert_eq!(k(b"\x1b[Z"), vec![Key::BackTab]);
    assert_eq!(k(b"\t\x1b[Z"), vec![Key::Tab, Key::BackTab]);
}

#[test]
fn modifier_parameters() {
    assert_eq!(
        ev(b"\x1b[1;5A"),
        vec![KeyEvent::with(
            Key::Up,
            Mods {
                ctrl: true,
                ..Mods::NONE
            }
        )]
    );
    assert_eq!(
        ev(b"\x1b[1;2C"),
        vec![KeyEvent::with(
            Key::Right,
            Mods {
                shift: true,
                ..Mods::NONE
            }
        )]
    );
    assert_eq!(
        ev(b"\x1b[1;8D")[0].mods,
        Mods {
            shift: true,
            alt: true,
            ctrl: true
        }
    );
    let e = ev(b"\x1b[3;5~");
    assert_eq!(e[0].key, Key::Delete);
    assert!(e[0].mods.ctrl);
    // Unmodified sequences report no mods.
    assert_eq!(ev(b"\x1b[A")[0].mods, Mods::NONE);
}

#[test]
fn utf8_multibyte_assembly() {
    assert_eq!(k("é".as_bytes()), vec![Key::Char('é')]);
    assert_eq!(k("中".as_bytes()), vec![Key::Char('中')]);
    assert_eq!(k("🦀".as_bytes()), vec![Key::Char('🦀')]);
    assert_eq!(
        k("aé中🦀z".as_bytes()),
        vec![
            Key::Char('a'),
            Key::Char('é'),
            Key::Char('中'),
            Key::Char('🦀'),
            Key::Char('z'),
        ]
    );
}

#[test]
fn four_byte_emoji_split_across_four_feeds() {
    let bytes = "🦀".as_bytes().to_vec();
    let mut d = Decoder::new();
    assert!(d.feed_keys(&bytes[0..1]).is_empty());
    assert!(d.feed_keys(&bytes[1..2]).is_empty());
    assert!(d.feed_keys(&bytes[2..3]).is_empty());
    assert_eq!(d.feed_keys(&bytes[3..4]), vec![Key::Char('🦀')]);
    assert!(d.pending().is_empty());
}

#[test]
fn three_byte_char_split_after_two_bytes() {
    let bytes = "中".as_bytes().to_vec();
    let mut d = Decoder::new();
    assert!(d.feed_keys(&bytes[0..2]).is_empty());
    assert_eq!(d.feed_keys(&bytes[2..]), vec![Key::Char('中')]);
}

#[test]
fn lone_escape_is_emitted_immediately() {
    assert_eq!(k(b"\x1b"), vec![Key::Esc]);
    assert_eq!(k(b"\x1b\x1b"), vec![Key::Esc, Key::Esc]);
    assert_eq!(k(b"a\x1b"), vec![Key::Char('a'), Key::Esc]);
}

#[test]
fn partial_csi_is_held_then_completed() {
    let mut d = Decoder::new();
    assert!(d.feed_keys(b"\x1b[").is_empty());
    assert_eq!(d.pending(), b"\x1b[");
    assert_eq!(d.feed_keys(b"A"), vec![Key::Up]);
    assert!(d.pending().is_empty());
}

#[test]
fn partial_csi_with_params_split_three_ways() {
    let mut d = Decoder::new();
    assert!(d.feed_keys(b"\x1b[1").is_empty());
    assert!(d.feed_keys(b";5").is_empty());
    assert_eq!(d.feed_keys(b"A"), vec![Key::Up]);
}

#[test]
fn partial_ss3_split_across_feeds() {
    let mut d = Decoder::new();
    assert!(d.feed_keys(b"\x1bO").is_empty());
    assert_eq!(d.feed_keys(b"P"), vec![Key::F(1)]);
}

#[test]
fn decode_retains_bare_escape_as_leftover() {
    let (evs, rest) = decode(b"a\x1b");
    assert_eq!(evs, vec![KeyEvent::plain(Key::Char('a'))]);
    assert_eq!(rest, b"\x1b");
    let (keys, rest) = decode_keys(b"\x1b[1;");
    assert!(keys.is_empty());
    assert_eq!(rest, b"\x1b[1;");
}

#[test]
fn alt_prefixed_characters() {
    assert_eq!(ev(b"\x1bx"), vec![KeyEvent::with(Key::Alt('x'), Mods::ALT)]);
    assert_eq!(ev("\x1bé".as_bytes())[0].key, Key::Alt('é'));
}

#[test]
fn bracketed_paste_markers_are_swallowed() {
    assert_eq!(
        k(b"\x1b[200~hi\x1b[201~"),
        vec![Key::Char('h'), Key::Char('i')]
    );
    assert_eq!(k(b"\x1b[200~\x1b[201~"), vec![]);
    assert_eq!(
        k(b"a\x1b[200~b\x1b[201~c"),
        vec![Key::Char('a'), Key::Char('b'), Key::Char('c')]
    );
}

#[test]
fn bracketed_paste_marker_split_across_feeds() {
    let mut d = Decoder::new();
    assert!(d.feed_keys(b"\x1b[200").is_empty());
    assert_eq!(d.feed_keys(b"~x"), vec![Key::Char('x')]);
}

#[test]
fn stray_mouse_reports_are_swallowed() {
    // We never enable mouse tracking, but a leftover report from another
    // program must not be interpreted as typed text.
    assert_eq!(k(b"\x1b[<0;12;24Ma"), vec![Key::Char('a')]);
    assert_eq!(k(b"\x1b[<0;12;24ma"), vec![Key::Char('a')]);
    // X10 form: ESC [ M plus three raw bytes.
    assert_eq!(k(b"\x1b[M @Ba"), vec![Key::Char('a')]);
}

#[test]
fn terminal_query_replies_are_swallowed() {
    assert_eq!(k(b"\x1b]11;rgb:0000/0000/0000\x07j"), vec![Key::Char('j')]);
    assert_eq!(k(b"\x1bP>|foo\x1b\\k"), vec![Key::Char('k')]);
}

#[test]
fn csi_u_encoding() {
    assert_eq!(ev(b"\x1b[99;5u")[0].key, Key::Ctrl('c'));
    assert_eq!(ev(b"\x1b[97;1u")[0].key, Key::Char('a'));
    assert_eq!(ev(b"\x1b[9;2u")[0].key, Key::BackTab);
    assert_eq!(ev(b"\x1b[13;1u")[0].key, Key::Enter);
    assert_eq!(ev(b"\x1b[127;1u")[0].key, Key::Backspace);
}

#[test]
fn invalid_utf8_makes_progress_without_panicking() {
    assert_eq!(k(&[0xff, b'a']), vec![Key::Unknown, Key::Char('a')]);
    assert_eq!(k(&[0xc3, 0x28]), vec![Key::Unknown, Key::Char('(')]);
    assert_eq!(k(&[0x80, b'z']), vec![Key::Unknown, Key::Char('z')]);
}

#[test]
fn a_pager_burst_decodes_in_order() {
    let mut d = Decoder::new();
    assert_eq!(
        d.feed_keys(b"j\x1b[Bk\x1b[6~G\x1b[Zq"),
        vec![
            Key::Char('j'),
            Key::Down,
            Key::Char('k'),
            Key::PageDown,
            Key::Char('G'),
            Key::BackTab,
            Key::Char('q'),
        ]
    );
}

#[test]
fn unrecognised_but_well_formed_csi_is_consumed_whole() {
    assert_eq!(k(b"\x1b[999Xz"), vec![Key::Unknown, Key::Char('z')]);
}

#[test]
fn a_stuck_partial_is_held_until_it_completes() {
    let mut d = Decoder::new();
    d.feed(b"\x1b[1;");
    assert!(!d.pending().is_empty());
    assert_eq!(d.feed(b"5A"), vec![KeyEvent::with(Key::Up, Mods { ctrl: true, ..Mods::NONE })]);
    assert!(d.pending().is_empty());
}

#[test]
fn mods_from_param_decodes_the_xterm_bitmask() {
    assert_eq!(Mods::from_param(1), Mods::NONE);
    assert_eq!(
        Mods::from_param(2),
        Mods {
            shift: true,
            ..Mods::NONE
        }
    );
    assert_eq!(Mods::from_param(3), Mods::ALT);
    assert_eq!(
        Mods::from_param(5),
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    );
    assert_eq!(
        Mods::from_param(7),
        Mods {
            shift: false,
            alt: true,
            ctrl: true
        }
    );
    assert_eq!(Mods::from_param(0), Mods::NONE);
}

#[test]
fn every_spec_keybinding_byte_form_decodes() {
    // Sanity sweep over the bindings listed in SPEC.md §Keybindings.
    for (bytes, expect) in [
        (&b"j"[..], Key::Char('j')),
        (&b"k"[..], Key::Char('k')),
        (&b"d"[..], Key::Char('d')),
        (&b"u"[..], Key::Char('u')),
        (&b" "[..], Key::Char(' ')),
        (&b"f"[..], Key::Char('f')),
        (&b"b"[..], Key::Char('b')),
        (&b"-"[..], Key::Char('-')),
        (&b"\x1b[A"[..], Key::Up),
        (&b"\x1b[D"[..], Key::Left),
        (&b"\r"[..], Key::Enter),
        (&b"\x7f"[..], Key::Backspace),
        (&b"\t"[..], Key::Tab),
        (&b"\x1b[Z"[..], Key::BackTab),
        (&b"\x1bOP"[..], Key::F(1)),
    ] {
        assert_eq!(k(bytes), vec![expect], "bytes {bytes:?}");
    }
}
