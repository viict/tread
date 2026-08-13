//! Windows and unix path rules, both exercised on whichever host runs the
//! suite. Every test names its platform explicitly: `#[cfg(windows)]` tests
//! would never run in this project, which has no Windows machine in the loop.
use super::path::*;
use super::Platform::{self, Linux, Macos, Windows};

// -- prefixes ---------------------------------------------------------------

#[test]
fn unix_paths_have_no_volume_prefix() {
    for p in [Linux, Macos] {
        assert_eq!(split_prefix(p, "/a/b"), ("", "/a/b"));
        // A drive letter is an ordinary directory name on unix.
        assert_eq!(split_prefix(p, "C:/a"), ("", "C:/a"));
        assert_eq!(prefix_len(p, "\\\\srv\\share\\x"), 0);
    }
}

#[test]
fn windows_prefixes_cover_drives_unc_and_devices() {
    let cases = [
        ("C:\\a\\b", "C:"),
        ("c:/a", "c:"),
        ("C:a", "C:"),
        ("\\\\srv\\share\\a\\b", "\\\\srv\\share"),
        ("//srv/share/a", "//srv/share"),
        ("\\\\srv", "\\\\srv"),
        ("\\\\?\\C:\\a", "\\\\?\\C:"),
        ("\\\\.\\pipe\\x", "\\\\.\\pipe"),
        ("\\a\\b", ""),
        ("a\\b", ""),
    ];
    for (input, want) in cases {
        assert_eq!(split_prefix(Windows, input).0, want, "{input}");
    }
}

#[test]
fn absoluteness_distinguishes_windows_half_qualified_paths() {
    assert!(is_absolute(Linux, "/a"));
    assert!(!is_absolute(Linux, "a/b"));
    assert!(!is_absolute(Linux, "C:\\a"));
    assert!(is_absolute(Windows, "C:\\a"));
    assert!(is_absolute(Windows, "c:/a"));
    assert!(is_absolute(Windows, "\\\\srv\\share\\a"));
    // Rooted on the current volume, and volume-relative: neither is absolute.
    assert!(!is_absolute(Windows, "\\a"));
    assert!(!is_absolute(Windows, "C:a"));
    assert!(!is_absolute(Windows, "a\\b"));
}

// -- parse / render ---------------------------------------------------------

#[test]
fn parsing_folds_dot_and_dotdot() {
    let q = parse(Linux, "/a/./b/../c//d").unwrap();
    assert_eq!(q.prefix, "");
    assert!(q.rooted);
    assert_eq!(q.comps, ["a", "c", "d"]);
    let w = parse(Windows, "C:\\a\\.\\b\\..\\c").unwrap();
    assert_eq!(w.prefix, "C:");
    assert!(w.rooted);
    assert_eq!(w.comps, ["a", "c"]);
}

#[test]
fn walking_off_the_front_refuses_a_join_and_survives_a_parse() {
    // Walking off the front of the base is refused, and that is what `join`'s
    // `Option` is for. A `..` that merely pops a real component is not off the
    // front: it produces a path, and `contains` is what then refuses it.
    assert_eq!(join(Linux, "/corpus", "../etc/passwd").as_deref(), Some("/etc/passwd"));
    assert!(!contains(Linux, "/corpus", "/etc/passwd"));
    assert_eq!(join(Linux, "/corpus/a", "../../.."), None);
    assert_eq!(join(Windows, "C:\\corpus", "..\\..\\x"), None);
    // Nor may one `..` cancel another and creep back in.
    assert_eq!(join(Linux, "../notes", "../../etc/passwd"), None);

    // Rooted: nothing is above the root, so it clamps there. `/a/../..` is `/`.
    assert_eq!(render(Linux, &parse(Linux, "/a/../..").unwrap()), "/");
    assert_eq!(render(Windows, &parse(Windows, "C:\\..\\x").unwrap()), "C:\\x");

    // Relative: the `..` is part of what the path names and must survive, or
    // every path written that way becomes unusable — which is what made a
    // listing opened as `tread ../notes/` refuse its own entries.
    assert_eq!(render(Linux, &parse(Linux, "../x").unwrap()), "../x");
    assert_eq!(render(Linux, &parse(Linux, "../../a/b/..").unwrap()), "../../a");
    assert_eq!(render(Windows, &parse(Windows, "..\\x").unwrap()), "..\\x");
}

/// The bug this file's `..` rules were changed for: a directory reached by a
/// relative path holds its own entries, so the containment check must say yes.
#[test]
fn a_relative_parent_path_contains_what_is_under_it() {
    for p in [Linux, Macos] {
        assert!(contains(p, "../notes", "../notes/a.md"));
        assert!(contains(p, "..", "../notes/a.md"));
        assert!(contains(p, "../notes", "../notes/deep/b.md"));
        // And still says no to what is genuinely outside.
        assert!(!contains(p, "../notes", "../other/a.md"));
        assert!(!contains(p, "../notes", "/etc/passwd"));
    }
    assert!(contains(Windows, "..\\notes", "..\\notes\\a.md"));
    // A relative root never contains an absolute path, and the reverse.
    assert!(!contains(Linux, "/notes", "../notes/a.md"));
}

#[test]
fn rendering_uses_the_platforms_own_separator() {
    assert_eq!(join(Linux, "/root", "a/b").unwrap(), "/root/a/b");
    assert_eq!(join(Macos, "/root", "a/b").unwrap(), "/root/a/b");
    assert_eq!(join(Windows, "C:/root", "a/b").unwrap(), "C:\\root\\a\\b");
    // Rooted with nothing under it round-trips as the root itself.
    assert_eq!(join(Linux, "/", "").unwrap(), "/");
    assert_eq!(join(Windows, "C:\\", "").unwrap(), "C:\\");
    // Volume-relative keeps its shape: no separator is invented after `C:`.
    assert_eq!(join(Windows, "C:a", "b").unwrap(), "C:a\\b");
    assert_eq!(join(Linux, "", "a/b").unwrap(), "a/b");
}

// -- markdown link joins ----------------------------------------------------

#[test]
fn markdown_links_are_slash_written_and_join_on_every_platform() {
    for p in Platform::ALL {
        let base = if p.is_windows() { "C:\\corpus\\docs" } else { "/corpus/docs" };
        let want = if p.is_windows() {
            "C:\\corpus\\models\\SAMPLE_MODEL.md"
        } else {
            "/corpus/models/SAMPLE_MODEL.md"
        };
        assert_eq!(join(p, base, "../models/SAMPLE_MODEL.md").unwrap(), want, "{p:?}");
    }
}

/// A markdown link cannot contain a real `\` on Windows — no filename may — so
/// treating `..\..\x` as one component there would hand the OS a path that
/// leaves the corpus while the containment check saw one harmless component.
/// Split there, the escape is visible and rejected. On unix `\` stays a
/// filename byte, so the same link is a (silly) name inside the corpus.
#[test]
fn a_backslash_in_a_link_separates_on_windows_and_does_not_on_unix() {
    let win = join(Windows, "C:\\corpus\\docs", "..\\..\\etc\\x").unwrap();
    assert_eq!(win, "C:\\etc\\x");
    assert!(!contains(Windows, "C:\\corpus", &win));
    // One more `..` and the fold itself fails, which is also an escape.
    assert_eq!(join(Windows, "C:\\corpus\\docs", "..\\..\\..\\x"), None);

    let unix = join(Linux, "/corpus/docs", "..\\..\\etc\\x").unwrap();
    assert_eq!(unix, "/corpus/docs/..\\..\\etc\\x");
    assert!(contains(Linux, "/corpus", &unix));
}

#[test]
fn rooted_markdown_links_are_recognised_per_platform() {
    assert!(markdown_is_rooted(Linux, "/models/a.md"));
    assert!(!markdown_is_rooted(Linux, "\\models\\a.md"));
    assert!(markdown_is_rooted(Windows, "\\models\\a.md"));
    assert_eq!(markdown_trim_root(Windows, "\\\\models/a.md"), "models/a.md");
    assert_eq!(markdown_trim_root(Linux, "//models/a.md"), "models/a.md");
    assert_eq!(markdown_trim_root(Linux, "models/a.md"), "models/a.md");
}

// -- absolutize -------------------------------------------------------------

#[test]
fn relative_paths_are_joined_onto_the_working_directory() {
    assert_eq!(absolutize(Linux, "/w/d", "a/b.md"), "/w/d/a/b.md");
    assert_eq!(absolutize(Linux, "/w/d", "../a.md"), "/w/a.md");
    assert_eq!(absolutize(Windows, "C:\\w\\d", "a/b.md"), "C:\\w\\d\\a\\b.md");
    assert_eq!(absolutize(Windows, "C:\\w\\d", "..\\a.md"), "C:\\w\\a.md");
}

#[test]
fn absolute_paths_keep_their_volume() {
    assert_eq!(absolutize(Linux, "/w", "/x/./y"), "/x/y");
    assert_eq!(absolutize(Windows, "C:\\w", "D:\\x\\y"), "D:\\x\\y");
    assert_eq!(absolutize(Windows, "C:\\w", "d:/x/y"), "d:\\x\\y");
    assert_eq!(
        absolutize(Windows, "C:\\w", "\\\\srv\\share\\x"),
        "\\\\srv\\share\\x"
    );
    assert_eq!(absolutize(Windows, "C:\\w", "\\\\?\\D:\\x"), "\\\\?\\D:\\x");
}

/// The two Windows shapes that are neither absolute nor plainly relative.
#[test]
fn windows_half_qualified_paths_resolve_conservatively() {
    // `\dir` keeps the working directory's volume, not its directory.
    assert_eq!(absolutize(Windows, "D:\\w\\d", "\\x\\y.md"), "D:\\x\\y.md");
    assert_eq!(
        absolutize(Windows, "\\\\srv\\share\\w", "\\x"),
        "\\\\srv\\share\\x"
    );
    // `C:dir` against a cwd on the same volume uses the cwd, case-insensitively.
    assert_eq!(absolutize(Windows, "C:\\w\\d", "C:a.md"), "C:\\w\\d\\a.md");
    assert_eq!(absolutize(Windows, "c:\\w\\d", "C:a.md"), "c:\\w\\d\\a.md");
    // Against a different volume it cannot be known, so it means that volume's
    // root rather than a path grafted from the wrong drive.
    assert_eq!(absolutize(Windows, "C:\\w\\d", "E:a.md"), "E:\\a.md");
}

#[test]
fn an_unfoldable_path_falls_back_to_itself_instead_of_panicking() {
    assert_eq!(absolutize(Linux, "/w", "../../../../x"), "../../../../x");
    assert_eq!(absolutize(Windows, "C:\\", "..\\..\\x"), "..\\..\\x");
}

// -- comparison and containment ---------------------------------------------

#[test]
fn unix_comparison_is_case_sensitive_and_windows_is_not() {
    assert!(same(Linux, "/a/b.md", "/a/./b.md"));
    assert!(!same(Linux, "/a/B.md", "/a/b.md"));
    assert!(same(Windows, "C:\\a\\B.MD", "c:/a/b.md"));
    assert!(!same(Windows, "C:\\a\\b.md", "D:\\a\\b.md"));
    // Rootedness is part of identity: `a/b` is not `/a/b`.
    assert!(!same(Linux, "a/b", "/a/b"));
    assert!(!same(Windows, "C:a", "C:\\a"));
}

#[test]
fn containment_is_component_wise_not_textual() {
    assert!(contains(Linux, "/corpus", "/corpus/a/b.md"));
    assert!(contains(Linux, "/corpus", "/corpus"));
    // The classic prefix-string bug: /corpus-evil is not inside /corpus.
    assert!(!contains(Linux, "/corpus", "/corpus-evil/x.md"));
    assert!(!contains(Linux, "/corpus", "/other/x.md"));
    assert!(!contains(Windows, "C:\\corpus", "C:\\corpus-evil\\x.md"));
    assert!(contains(Windows, "C:\\Corpus", "c:/corpus/A/x.md"));
    assert!(!contains(Windows, "C:\\corpus", "D:\\corpus\\x.md"));
    assert!(!contains(
        Windows,
        "\\\\srv\\share\\c",
        "\\\\other\\share\\c\\x"
    ));
}

#[test]
fn the_status_bar_path_is_relative_with_native_separators() {
    assert_eq!(rel_to(Linux, "/corpus", "/corpus/models/a.md"), "models/a.md");
    assert_eq!(
        rel_to(Windows, "C:\\corpus", "C:\\corpus\\models\\a.md"),
        "models\\a.md"
    );
    assert_eq!(rel_to(Windows, "c:\\corpus", "C:/corpus/a.md"), "a.md");
    // The document that *is* the root renders as the empty relative path, and
    // anything outside falls back to its own spelling.
    assert_eq!(rel_to(Linux, "/corpus", "/corpus"), "");
    assert_eq!(rel_to(Linux, "/corpus", "/elsewhere/a.md"), "/elsewhere/a.md");
}
