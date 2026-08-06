//! Home and cache resolution for all three platforms, from a Linux host.
use std::path::PathBuf;

use super::dirs::*;
use super::Platform::{self, Linux, Macos, Windows};

fn want(s: &str) -> Option<PathBuf> {
    Some(PathBuf::from(s))
}

#[test]
fn home_is_home_on_unix_and_userprofile_on_windows() {
    let e = Env::of(&[("HOME", "/home/u"), ("USERPROFILE", "C:\\Users\\u")]);
    assert_eq!(home(Linux, &e).as_deref(), Some("/home/u"));
    assert_eq!(home(Macos, &e).as_deref(), Some("/home/u"));
    assert_eq!(home(Windows, &e).as_deref(), Some("C:\\Users\\u"));
    // $HOME is what MSYS / Git-Bash sets, so it still counts on Windows.
    let msys = Env::of(&[("HOME", "C:\\msys\\home\\u")]);
    assert_eq!(home(Windows, &msys).as_deref(), Some("C:\\msys\\home\\u"));
    // Nothing set at all.
    assert_eq!(home(Windows, &Env::default()), None);
    assert_eq!(home(Linux, &Env::of(&[("USERPROFILE", "C:\\u")])), None);
}

#[test]
fn an_exported_but_empty_variable_counts_as_unset() {
    let e = Env::of(&[("HOME", ""), ("XDG_CACHE_HOME", "")]);
    assert_eq!(home(Linux, &e), None);
    assert_eq!(yank_fallback(Linux, &e), None);
}

#[test]
fn linux_uses_xdg_then_dot_cache() {
    let xdg = Env::of(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "/x/cache")]);
    assert_eq!(yank_fallback(Linux, &xdg), want("/x/cache/tread/last-yank.txt"));
    let plain = Env::of(&[("HOME", "/home/u")]);
    assert_eq!(
        yank_fallback(Linux, &plain),
        want("/home/u/.cache/tread/last-yank.txt")
    );
    assert_eq!(yank_fallback(Linux, &Env::default()), None);
}

/// `~/Library/Caches` is the documented macOS location and the one the OS's own
/// cache eviction knows about; an explicit `XDG_CACHE_HOME` still wins.
#[test]
fn macos_uses_library_caches_unless_xdg_is_set() {
    let plain = Env::of(&[("HOME", "/Users/u")]);
    assert_eq!(
        yank_fallback(Macos, &plain),
        want("/Users/u/Library/Caches/tread/last-yank.txt")
    );
    let xdg = Env::of(&[("HOME", "/Users/u"), ("XDG_CACHE_HOME", "/x")]);
    assert_eq!(yank_fallback(Macos, &xdg), want("/x/tread/last-yank.txt"));
    assert_eq!(yank_fallback(Macos, &Env::default()), None);
}

#[test]
fn windows_uses_localappdata_then_temp_then_userprofile() {
    let full = Env::of(&[
        ("USERPROFILE", "C:\\Users\\u"),
        ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
        ("TEMP", "C:\\Temp"),
    ]);
    assert_eq!(
        yank_fallback(Windows, &full),
        want("C:\\Users\\u\\AppData\\Local\\tread\\last-yank.txt")
    );
    let temp_only = Env::of(&[("TEMP", "C:\\Temp")]);
    assert_eq!(
        yank_fallback(Windows, &temp_only),
        want("C:\\Temp\\tread\\last-yank.txt")
    );
    let profile_only = Env::of(&[("USERPROFILE", "C:\\Users\\u")]);
    assert_eq!(
        yank_fallback(Windows, &profile_only),
        want("C:\\Users\\u\\AppData\\Local\\tread\\last-yank.txt")
    );
    assert_eq!(yank_fallback(Windows, &Env::default()), None);
}

/// `$HOME` is never consulted for a Windows cache path — that is the bug this
/// module exists to prevent.
#[test]
fn windows_never_falls_back_to_a_unix_home() {
    let unixish = Env::of(&[("HOME", "/home/u")]);
    assert_eq!(yank_fallback(Windows, &unixish), None);
}

#[test]
fn every_fallback_path_is_absolute_and_ends_in_the_yank_file() {
    let envs = [
        (Linux, Env::of(&[("HOME", "/home/u")])),
        (Macos, Env::of(&[("HOME", "/Users/u")])),
        (
            Windows,
            Env::of(&[("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local")]),
        ),
    ];
    for (p, env) in envs {
        let path = yank_fallback(p, &env).expect("a path");
        let s = path.to_string_lossy().into_owned();
        assert!(super::path::is_absolute(p, &s), "{p:?}: {s}");
        assert!(s.ends_with("last-yank.txt"), "{p:?}: {s}");
        assert!(s.contains("tread"), "{p:?}: {s}");
        // Never the other platform's separator.
        if p.is_windows() {
            assert!(!s.contains('/'), "{s}");
        } else {
            assert!(!s.contains('\\'), "{s}");
        }
    }
}

#[test]
fn the_relative_part_is_one_definition_shared_by_every_platform() {
    assert_eq!(YANK_RELATIVE, "tread/last-yank.txt");
    for p in Platform::ALL {
        let base = if p.is_windows() { "C:\\c" } else { "/c" };
        let joined = super::path::join(p, base, YANK_RELATIVE).unwrap();
        assert!(joined.ends_with("last-yank.txt"));
    }
}

// -- display ----------------------------------------------------------------

#[test]
fn unix_paths_under_home_are_shown_with_a_tilde() {
    for p in [Linux, Macos] {
        let h = Some("/home/u");
        assert_eq!(
            display_path(p, "/home/u/.cache/tread/last-yank.txt", h),
            "~/.cache/tread/last-yank.txt"
        );
        assert_eq!(display_path(p, "/tmp/x.txt", h), "/tmp/x.txt");
        assert_eq!(display_path(p, "/home/u/x", None), "/home/u/x");
        // Home itself, and a sibling that merely shares the prefix text.
        assert_eq!(display_path(p, "/home/u", h), "/home/u");
        assert_eq!(display_path(p, "/home/user2/x", h), "/home/user2/x");
    }
}

/// `~\…` is meaningless to cmd, PowerShell and Explorer, so Windows shows the
/// path the user could actually paste back.
#[test]
fn windows_paths_are_shown_verbatim() {
    let h = Some("C:\\Users\\u");
    let p = "C:\\Users\\u\\AppData\\Local\\tread\\last-yank.txt";
    assert_eq!(display_path(Windows, p, h), p);
    assert_eq!(display_path(Windows, p, None), p);
    assert!(!display_path(Windows, p, h).contains('~'));
}
