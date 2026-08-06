//! Lexical native-path arithmetic, parameterized by [`Platform`].
//!
//! `std::path` only ever speaks the *host's* dialect: on a Linux builder
//! `Path::new(r"C:\corpus\a.md")` is a single component named `C:\corpus\a.md`,
//! so Windows path handling written against `std::path` cannot be tested here
//! at all. Since this project has no Windows machine in the loop, the rules are
//! written out as pure functions over `&str` instead and tested for both
//! dialects on whatever host runs `cargo test`. `std::path` is still used
//! everywhere the *host's* own dialect is what matters (`parent`, `extension`,
//! `file_name`, `is_file`).
//!
//! Everything is lexical: `.` and `..` are folded textually and a `..` that
//! walks off the front is an error, never a trip to the filesystem. That is
//! what makes the corpus-root containment check meaningful — a symlink cannot
//! launder a path out of the corpus if the path never reaches `canonicalize`.
//!
//! Dialects:
//!
//! * unix (Linux, macOS) — `/` separates, no volume prefixes, case-sensitive.
//! * Windows — `/` and `\` both separate, paths may carry a volume prefix
//!   (`C:`, `\\server\share`, `\\?\C:`), and comparison is ASCII
//!   case-insensitive. Non-ASCII case folding is deliberately not attempted:
//!   Windows folds by the volume's collation table, which is not knowable from
//!   here, and over-folding would let two genuinely different files compare
//!   equal. ASCII covers the case the check exists for (`c:\` vs `C:\`).
#![deny(unsafe_code)]

use super::Platform;

/// The separator to *write* on this platform. Both are read on Windows.
pub const fn sep(p: Platform) -> char {
    if p.is_windows() {
        '\\'
    } else {
        '/'
    }
}

/// Does `c` separate path components on `p`?
pub fn is_sep(p: Platform, c: char) -> bool {
    c == '/' || (p.is_windows() && c == '\\')
}

fn is_sep_b(p: Platform, b: u8) -> bool {
    b == b'/' || (p.is_windows() && b == b'\\')
}

/// A native path taken apart: volume prefix, whether it is rooted on that
/// volume, and its `.`/`..`-folded components.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    /// `""`, or `C:` / `\\server\share` / `\\?\C:` on Windows. Never ends in a
    /// separator.
    pub prefix: String,
    /// A separator followed the prefix: `C:\x` and `/x` are rooted, `C:x` and
    /// `x` are not.
    pub rooted: bool,
    pub comps: Vec<String>,
}

/// Length in bytes of the volume prefix of `s` (always 0 off Windows).
///
/// Recognises `C:`, `\\server\share`, and the `\\?\` / `\\.\` device
/// namespaces (whose prefix runs through the device component, e.g.
/// `\\?\C:`). Separators are ASCII, so every index here is a char boundary.
pub fn prefix_len(p: Platform, s: &str) -> usize {
    if !p.is_windows() {
        return 0;
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return 2;
    }
    if b.len() >= 2 && is_sep_b(p, b[0]) && is_sep_b(p, b[1]) {
        let verbatim = b.len() > 3 && (b[2] == b'?' || b[2] == b'.') && is_sep_b(p, b[3]);
        let first = comp_end(p, b, if verbatim { 4 } else { 2 });
        if verbatim || first >= b.len() {
            return first;
        }
        return comp_end(p, b, first + 1);
    }
    0
}

/// Index of the first separator at or after `i` (or the end).
fn comp_end(p: Platform, b: &[u8], mut i: usize) -> usize {
    while i < b.len() && !is_sep_b(p, b[i]) {
        i += 1;
    }
    i
}

/// Split `s` into its volume prefix and the rest.
pub fn split_prefix(p: Platform, s: &str) -> (&str, &str) {
    s.split_at(prefix_len(p, s))
}

/// True when `s` names a location that needs no working directory: `/x`,
/// `C:\x`, `\\server\share\x`. Windows' `\x` (rooted, volume-relative) and
/// `C:x` (volume, directory-relative) are *not* absolute — see [`absolutize`].
pub fn is_absolute(p: Platform, s: &str) -> bool {
    let (prefix, rest) = split_prefix(p, s);
    let rooted = rest.starts_with(|c| is_sep(p, c));
    if p.is_windows() {
        !prefix.is_empty() && rooted
    } else {
        rooted
    }
}

/// Fold `s`'s components onto `out`. `None` when a `..` walks off the front,
/// which is how a link that escapes the corpus root is detected.
fn fold_into(p: Platform, s: &str, out: &mut Vec<String>) -> Option<()> {
    for part in s.split(|c| is_sep(p, c)) {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            other => out.push(other.to_string()),
        }
    }
    Some(())
}

/// Take a native path apart, folding `.` and `..`.
pub fn parse(p: Platform, s: &str) -> Option<Parts> {
    let (prefix, rest) = split_prefix(p, s);
    let mut comps = Vec::new();
    fold_into(p, rest, &mut comps)?;
    Some(Parts {
        prefix: prefix.to_string(),
        rooted: rest.starts_with(|c| is_sep(p, c)),
        comps,
    })
}

/// Put a [`Parts`] back together with this platform's separator.
pub fn render(p: Platform, parts: &Parts) -> String {
    let sep = sep(p);
    let mut out = String::with_capacity(parts.prefix.len() + parts.comps.len() * 8);
    out.push_str(&parts.prefix);
    if parts.rooted {
        out.push(sep);
    }
    for (i, c) in parts.comps.iter().enumerate() {
        if i > 0 {
            out.push(sep);
        }
        out.push_str(c);
    }
    out
}

/// Join a relative path onto `base` and fold the result. `None` on escape.
///
/// `rel` is split on every separator the platform recognises. That is
/// deliberate for markdown links too: a link is *written* with `/`, but on
/// Windows no filename may contain `\`, so treating `..\..\etc` as one
/// component there would hand the OS an escape the containment check never
/// saw. On unix `\` stays an ordinary filename byte.
pub fn join(p: Platform, base: &str, rel: &str) -> Option<String> {
    let mut parts = parse(p, base)?;
    fold_into(p, rel, &mut parts.comps)?;
    Some(render(p, &parts))
}

/// [`join`], falling back to `given` (the path as the user wrote it) when the
/// join walks off the front. A path that cannot be made absolute is still
/// usable — the filesystem will have the last word on it — so this never
/// invents a location.
fn join_or(p: Platform, base: &str, rel: &str, given: &str) -> String {
    join(p, base, rel).unwrap_or_else(|| given.to_string())
}

/// Fold a path in place, falling back to the input when it escapes.
fn fold_or(p: Platform, s: &str) -> String {
    parse(p, s).map(|q| render(p, &q)).unwrap_or_else(|| s.to_string())
}

/// A markdown link destination that starts at the corpus root: `/models/a.md`
/// (and `\models\a.md` on Windows, which the OS would read the same way).
pub fn markdown_is_rooted(p: Platform, s: &str) -> bool {
    s.starts_with(|c| is_sep(p, c))
}

/// Strip the leading separators [`markdown_is_rooted`] matched.
pub fn markdown_trim_root(p: Platform, s: &str) -> &str {
    s.trim_start_matches(|c| is_sep(p, c))
}

/// Make `path` absolute against `cwd`, folding `.` and `..`.
///
/// Windows has two shapes that are neither absolute nor plainly relative:
/// `\dir` is rooted on the *current* volume, and `C:dir` is relative to the
/// process's working directory *on drive C*. The second is unknowable without
/// asking the OS, so it is resolved against `cwd` when the volumes match and
/// against `C:\` otherwise — the conservative reading, and one that can never
/// silently graft a path from one volume onto another.
pub fn absolutize(p: Platform, cwd: &str, path: &str) -> String {
    let (prefix, rest) = split_prefix(p, path);
    let rooted = rest.starts_with(|c| is_sep(p, c));
    if is_absolute(p, path) {
        return fold_or(p, path);
    }
    if rooted {
        // Windows `\dir`: keep the working directory's volume.
        let (cwd_prefix, _) = split_prefix(p, cwd);
        return fold_or(p, &format!("{cwd_prefix}{path}"));
    }
    if !prefix.is_empty() {
        let (cwd_prefix, _) = split_prefix(p, cwd);
        if eq(p, cwd_prefix, prefix) {
            return join_or(p, cwd, rest, path);
        }
        return fold_or(p, &format!("{prefix}{}{rest}", sep(p)));
    }
    join_or(p, cwd, path, path)
}

/// Compare two path *pieces* the way the platform does.
fn eq(p: Platform, a: &str, b: &str) -> bool {
    if p.is_windows() {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Are `a` and `b` the same location? Folds `.`/`..`, ignores separator
/// flavour and (on Windows) ASCII case, so `c:/corpus\A.MD` and
/// `C:\corpus\a.md` are one document.
pub fn same(p: Platform, a: &str, b: &str) -> bool {
    match (parse(p, a), parse(p, b)) {
        (Some(x), Some(y)) => {
            x.rooted == y.rooted
                && eq(p, &x.prefix, &y.prefix)
                && x.comps.len() == y.comps.len()
                && x.comps.iter().zip(&y.comps).all(|(u, v)| eq(p, u, v))
        }
        _ => eq(p, a, b),
    }
}

/// Is `child` inside `root` (or the same path)? The corpus-escape check.
pub fn contains(p: Platform, root: &str, child: &str) -> bool {
    strip(p, root, child).is_some()
}

/// `child`'s components below `root`, or `None` when it is not inside.
fn strip(p: Platform, root: &str, child: &str) -> Option<Vec<String>> {
    let (r, c) = (parse(p, root)?, parse(p, child)?);
    if r.rooted != c.rooted || !eq(p, &r.prefix, &c.prefix) || c.comps.len() < r.comps.len() {
        return None;
    }
    if !r.comps.iter().zip(&c.comps).all(|(u, v)| eq(p, u, v)) {
        return None;
    }
    Some(c.comps[r.comps.len()..].to_vec())
}

/// `child` written relative to `root` for display, with native separators.
/// Falls back to `child` unchanged when it is not under `root`.
pub fn rel_to(p: Platform, root: &str, child: &str) -> String {
    match strip(p, root, child) {
        Some(rest) => rest.join(&sep(p).to_string()),
        None => child.to_string(),
    }
}
