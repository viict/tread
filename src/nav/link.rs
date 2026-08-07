//! Link classification and path resolution.
//!
//! Every markdown link in a corpus document is one of four things: another
//! markdown document, an anchor inside the current one, an external URL, or a
//! file we will not render. Resolution is *lexical* — `..`/`.` are folded by
//! hand rather than by `canonicalize`, so a link that walks above the index
//! root is rejected as an escape instead of silently following a symlink out
//! of the corpus.
//!
//! A link destination is a *URL-ish* path — `models/SAMPLE_MODEL.md`, always
//! spelled with `/` — while the thing it resolves to is a *native* path, which
//! on Windows carries a volume prefix, `\` separators and case-insensitive
//! comparison. The join, the containment check and the relative form for the
//! status bar therefore all go through [`crate::plat::path`], which knows both
//! dialects and is tested for both from any host.
//!
//! Filesystem access goes through the [`Fs`] trait so resolution is unit
//! testable without touching a disk.
#![deny(unsafe_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::md::ast::slugify;
use crate::plat::{path as ppath, Platform};

/// The filesystem seam. `RealFs` is the only implementation outside tests.
pub trait Fs {
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn read(&self, path: &Path) -> Result<String, String>;
}

/// `std::fs`-backed filesystem.
pub struct RealFs;

impl Fs for RealFs {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn read(&self, path: &Path) -> Result<String, String> {
        // Lossy + BOM-stripped, exactly like the document opened from argv;
        // following a link must never fail because of a stray byte.
        crate::md::sanitize::read_file(path).map_err(|e| format!("{}: {}", path.display(), e))
    }
}

/// What a link points at, once resolved against the current document.
///
/// There used to be a fourth kind — `Other`, "a file inside the corpus that is
/// not markdown" — and following one was refused. It is gone: since SPEC.md
/// §Plain text every file the corpus links to is one the reader can show, so a
/// link to a `.sh` or a `.conf` is a [`Target::Doc`] like any other and opens
/// (SPEC.md §Navigation, "links to files the reader can show ... open in the
/// reader"). Which *format* the file is read as is [`super::Navigator::load_source`]'s
/// decision, not this module's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A file inside the corpus, with an optional heading anchor.
    Doc {
        path: PathBuf,
        anchor: Option<String>,
    },
    /// `#heading` — a heading in the document we are already reading.
    Anchor(String),
    /// `http(s)://`, `mailto:` and friends — anything with a scheme, so it
    /// covers the ones that will be refused as well. Handed to the system opener
    /// when the scheme is on [`super::external::OPENABLE`] and `--no-browser`
    /// was not given; shown and yankable either way (SPEC.md §"Opening a link
    /// outside the reader").
    External(String),
    /// Unusable: escapes the root, missing, or not a link at all.
    Broken { raw: String, why: String },
}

impl Target {
    /// What the status bar shows: the resolved path for internal links, the
    /// raw URL for external ones (SPEC.md §Navigation).
    pub fn describe(&self, root: &Path) -> String {
        match self {
            Target::Doc { path, anchor } => {
                let base = rel_to(path, root);
                match anchor {
                    Some(a) => format!("{base}#{a}"),
                    None => base,
                }
            }
            Target::Anchor(a) => format!("#{a}"),
            Target::External(url) => url.clone(),
            Target::Broken { raw, why } => format!("{raw}: {why}"),
        }
    }

    /// The text `y` would put on the clipboard for this link.
    pub fn yank_text(&self, root: &Path) -> String {
        match self {
            Target::External(url) => url.clone(),
            Target::Doc { path, .. } => rel_to(path, root),
            Target::Anchor(a) => format!("#{a}"),
            Target::Broken { raw, .. } => raw.clone(),
        }
    }
}

/// `path` shown relative to `root`, falling back to the full path.
///
/// Always `/`, on every platform. What comes back is relative to the *corpus
/// root*, not to the working directory, so it is not a path you could hand to
/// the shell on any OS — it is the corpus's own address for a document, and the
/// corpus addresses its documents the way its links do. Every consumer wants it
/// that way: the status bar names the link the document wrote, and `y` puts
/// this string on the clipboard, where `models\A.md` would be a broken markdown
/// link the moment it was pasted.
pub fn rel_to(path: &Path, root: &Path) -> String {
    let rel = ppath::rel_to(
        Platform::HOST,
        &root.to_string_lossy(),
        &path.to_string_lossy(),
    );
    match ppath::sep(Platform::HOST) {
        '/' => rel,
        native => rel.replace(native, "/"),
    }
}

/// Is `path` inside the corpus `root` (or the root itself)?
///
/// Component-wise, and ASCII case-insensitive on Windows: `c:\corpus\..` and
/// `C:\Corpus\..` are the same directory there, and a check that missed that
/// would call half the corpus an escape.
pub fn within_root(path: &Path, root: &Path) -> bool {
    ppath::contains(
        Platform::HOST,
        &root.to_string_lossy(),
        &path.to_string_lossy(),
    )
}

/// Do these two paths name the same document? Used wherever the corpus index,
/// the history stack and the open document are compared.
pub fn same_path(a: &Path, b: &Path) -> bool {
    ppath::same(Platform::HOST, &a.to_string_lossy(), &b.to_string_lossy())
}

fn broken(raw: &str, why: &str) -> Target {
    Target::Broken {
        raw: raw.to_string(),
        why: why.to_string(),
    }
}

/// Resolve one raw link destination.
///
/// `doc_dir` is the directory of the document the link appears in (**not** the
/// process working directory); `root` is the corpus root, normalized and
/// absolute. Both must already be lexically normalized.
pub fn resolve(raw: &str, doc_dir: &Path, root: &Path, fs: &dyn Fs) -> Target {
    let raw = raw.trim();
    if raw.is_empty() {
        return broken(raw, "empty link");
    }
    if let Some(frag) = raw.strip_prefix('#') {
        return Target::Anchor(slugify(frag));
    }
    if scheme_of(raw).is_some() {
        return Target::External(raw.to_string());
    }
    let (path_part, anchor) = split_fragment(raw);
    if path_part.is_empty() {
        return match anchor {
            Some(a) => Target::Anchor(a),
            None => broken(raw, "empty link"),
        };
    }
    let decoded = percent_decode(path_part);
    // A leading `/` in a link means the corpus root, not the filesystem root.
    let absolute = ppath::markdown_is_rooted(Platform::HOST, &decoded);
    let base = if absolute { root } else { doc_dir };
    let rel = ppath::markdown_trim_root(Platform::HOST, &decoded);
    let joined = match normalize(base, rel) {
        Some(p) => p,
        None => return broken(raw, "link escapes the index root"),
    };
    if !within_root(&joined, root) {
        return broken(raw, "link escapes the index root");
    }
    let joined = match fs.is_file(&joined) || fs.is_dir(&joined) {
        true => joined,
        false => root_named(rel, root, fs).unwrap_or(joined),
    };
    classify_path(raw, joined, anchor, fs)
}

/// A corpus that names itself: `codex/foundations/X.md`, written *inside* the
/// codex.
///
/// Such a path is relative to the corpus's parent — how a document refers to
/// itself from the outside, which is what you get when the path was copied
/// from somewhere else in the tree. Read from inside, the leading segment is
/// redundant and the link resolves to nothing. When it matches the root's own
/// folder name and dropping it finds a real file, that is what was meant.
///
/// Only ever a fallback, and only when the target exists: it must not shadow a
/// genuine `codex/` directory that happens to sit inside the corpus.
fn root_named(rel: &str, root: &Path, fs: &dyn Fs) -> Option<PathBuf> {
    let name = root.file_name()?.to_str()?;
    let rest = rel.strip_prefix(name)?.strip_prefix('/')?;
    let path = normalize(root, rest)?;
    (fs.is_file(&path) || fs.is_dir(&path)).then_some(path)
}

/// Decide what an existing (or almost-existing) path is.
fn classify_path(raw: &str, path: PathBuf, anchor: Option<String>, fs: &dyn Fs) -> Target {
    if fs.is_dir(&path) {
        let readme = path.join("README.md");
        if fs.is_file(&readme) {
            return Target::Doc {
                path: readme,
                anchor,
            };
        }
        return broken(raw, "directory has no README.md");
    }
    // Any file that exists is followable. The reader has a format for every
    // one of them — markdown, CSV, JSON, records, or plain text for everything
    // else — so refusing a `.sh` here would be refusing to show something it
    // can show (SPEC.md §Plain text).
    if fs.is_file(&path) {
        return Target::Doc { path, anchor };
    }
    if path.extension().is_none() {
        let mut with_md = OsString::from(path.as_os_str());
        with_md.push(".md");
        let candidate = PathBuf::from(with_md);
        if fs.is_file(&candidate) {
            return Target::Doc {
                path: candidate,
                anchor,
            };
        }
    }
    broken(raw, "no such file")
}

/// Is this path a markdown document?
///
/// Asked of [`crate::source::detect`] rather than answered here, because that
/// is the one place extensions are mapped to formats (SPEC.md §Multi-format
/// reading: "adding a format is one module plus one arm in the detector"). This
/// used to carry its own list — `md` and `markdown` — which was two extensions
/// short of the detector's: `notes.mkd` opened as markdown but was dropped from
/// the index listing, because the listing asks *this* and the loader asks the
/// detector. One table, so the two answers cannot disagree.
pub fn is_markdown(path: &Path) -> bool {
    crate::source::detect::from_path(path) == Some(crate::source::detect::Format::Markdown)
}

/// Split `path#fragment`. The fragment is slugified so it compares directly
/// against the heading ids the block parser assigns.
pub fn split_fragment(raw: &str) -> (&str, Option<String>) {
    match raw.split_once('#') {
        Some((p, f)) if !f.is_empty() => (p, Some(slugify(f))),
        Some((p, _)) => (p, None),
        None => (raw, None),
    }
}

/// The URL scheme of `raw`, if it has one. `a/b:c` is not a scheme — a colon
/// only counts before the first slash.
///
/// Re-exported from [`crate::url`], the leaf module that owns URL syntax, so
/// `link::scheme_of` keeps naming it here where resolution reads it: the
/// renderer needs the same predicate to colour a link and must not import the
/// navigator to get it.
pub use crate::url::scheme_of;

/// Lexically join the link destination `rel` onto the native directory `base`,
/// folding `.` and `..`. Returns `None` when `..` walks past the start, which
/// the caller reports as an escape from the corpus root.
///
/// `base` keeps whatever volume prefix it has (`C:`, `\\server\share`), which
/// the old `Component`-based fold silently dropped: every Windows link would
/// otherwise have resolved to a path rooted on the current drive.
pub fn normalize(base: &Path, rel: &str) -> Option<PathBuf> {
    ppath::join(Platform::HOST, &base.to_string_lossy(), rel).map(PathBuf::from)
}

/// Decode `%xx` escapes; anything malformed is left verbatim.
///
/// Works entirely on bytes: the two characters after a `%` may be the leading
/// bytes of a multi-byte scalar (`%a\u{20ac}`), so slicing the `&str` at
/// `i+1..i+3` would land inside a character and panic.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = hex_byte(bytes[i + 1], bytes[i + 2]) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// `(b'4', b'1') -> Some(0x41)`. `None` when either byte is not a hex digit,
/// which includes every non-ASCII byte.
fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some(hex_digit(hi)? << 4 | hex_digit(lo)?)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    #[test]
    fn plain_strings_pass_through() {
        assert_eq!(percent_decode("models/SAMPLE_MODEL.md"), "models/SAMPLE_MODEL.md");
        assert_eq!(percent_decode(""), "");
    }

    #[test]
    fn well_formed_escapes_decode() {
        assert_eq!(percent_decode("a%20b.md"), "a b.md");
        assert_eq!(percent_decode("%41%42"), "AB");
        assert_eq!(percent_decode("%2Fx"), "/x");
        // Multi-byte scalars round-trip through their UTF-8 escapes.
        assert_eq!(percent_decode("%E2%82%AC.md"), "\u{20ac}.md");
    }

    #[test]
    fn malformed_escapes_are_left_verbatim() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%a"), "%a");
        assert_eq!(percent_decode("%"), "%");
    }

    /// Regression: `&s[i + 1..i + 3]` used to slice inside a multi-byte char.
    #[test]
    fn a_percent_next_to_a_multibyte_char_does_not_panic() {
        assert_eq!(percent_decode("%a\u{20ac}"), "%a\u{20ac}");
        assert_eq!(percent_decode("%\u{20ac}"), "%\u{20ac}");
        assert_eq!(percent_decode("x%\u{1f600}y"), "x%\u{1f600}y");
        assert_eq!(percent_decode("%4\u{20ac}"), "%4\u{20ac}");
        // A byte sequence that decodes to invalid UTF-8 falls back to the input.
        assert_eq!(percent_decode("%ff"), "%ff");
    }

    /// The whole resolve path, which is what the pager calls on every paint.
    #[test]
    fn resolving_a_percent_encoded_link_is_panic_free() {
        struct Nothing;
        impl Fs for Nothing {
            fn is_file(&self, _p: &Path) -> bool {
                false
            }
            fn is_dir(&self, _p: &Path) -> bool {
                false
            }
            fn read(&self, _p: &Path) -> Result<String, String> {
                Err("no".into())
            }
        }
        let root = Path::new("/corpus");
        for raw in ["%a\u{20ac}", "%\u{20ac}#frag", "a%2", "%%%", "d%c3%a9j%c3%a0.md"] {
            let t = resolve(raw, root, root, &Nothing);
            assert!(matches!(t, Target::Broken { .. }), "{raw} -> {t:?}");
        }
    }
}
