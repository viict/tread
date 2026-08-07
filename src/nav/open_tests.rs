//! Following a link *out* of markdown (SPEC.md §Plain text, §Navigation).
//!
//! Its own file, and against a real temp directory rather than the in-memory
//! filesystem the rest of `tests.rs` uses: the lazily indexed formats need a
//! real path — that is what makes a 2GB log open instantly — so what
//! `load_source` hands back can only be checked on a disk.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use super::link::Target;
use super::Navigator;

/// A temp directory holding a small corpus, removed when the test ends.
struct TempCorpus(PathBuf);

impl TempCorpus {
    fn new(name: &str, files: &[(&str, &str)]) -> TempCorpus {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(format!("tread-nav-{}-{nanos}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp corpus");
        for (name, body) in files {
            std::fs::write(dir.join(name), body).expect("write");
        }
        TempCorpus(dir)
    }
}

impl Drop for TempCorpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A corpus link to a script opens, instead of being refused as "not markdown"
/// (SPEC.md §Plain text, §Navigation).
///
/// Two halves, and both matter: the link resolves to a followable target, and
/// what comes back through the seam is the *file's own lines* — `# comment` is
/// a comment, not a banner heading. A markdown neighbour still loads as
/// markdown, which is the thing this change must not break.
#[test]
fn a_link_to_a_script_opens_as_plain_text() {
    let corpus = TempCorpus::new(
        "script",
        &[
            ("README.md", "# Index\n\n[deploy](deploy.sh)\n\n[doc](other.md)\n"),
            ("deploy.sh", "#!/bin/sh\n# not a heading\nset -eu\n"),
            ("other.md", "# Other\n\ntext\n"),
        ],
    );
    let nav = Navigator::new(&corpus.0.join("README.md"), None, Path::new("/"));

    let script = match nav.resolve("deploy.sh") {
        Target::Doc { path, .. } => path,
        other => panic!("a script must be followable, got {other:?}"),
    };
    let mut src = nav.load_source(&script).expect("load the script");
    src.set_width(80);
    let rows: Vec<String> = src.lines(0..src.len()).iter().map(|l| l.text()).collect();
    assert_eq!(rows, ["#!/bin/sh", "# not a heading", "set -eu"]);
    assert!(src.outline().is_empty(), "a script has no headings");

    let doc = match nav.resolve("other.md") {
        Target::Doc { path, .. } => path,
        other => panic!("{other:?}"),
    };
    let mut md = nav.load_source(&doc).expect("load the document");
    md.set_width(80);
    assert!(!md.outline().is_empty(), "markdown keeps loading as markdown");

    // The index listing stays a listing of *documents*: the script resolves
    // and opens, but it is not an entry in the corpus's table of contents.
    let listed: Vec<String> = nav
        .entries()
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    assert!(listed.iter().any(|p| p.ends_with("other.md")), "{listed:?}");
    assert!(!listed.iter().any(|p| p.ends_with("deploy.sh")), "{listed:?}");
}

/// Every extension the detector calls markdown is listed in the index and loads
/// as markdown — including the two the index listing used to miss.
///
/// The listing filters on [`super::link::is_markdown`] while the loader asks
/// [`crate::source::detect`]; when those were two separate lists a `.mkd`
/// document opened correctly and was silently absent from `i`, `]` and the
/// outline overlay. One table now answers both, and this is the assertion that
/// keeps it one.
#[test]
fn every_markdown_extension_is_listed_and_loads_as_markdown() {
    let corpus = TempCorpus::new(
        "exts",
        &[
            (
                "README.md",
                "# Index\n\n- [a](a.md)\n- [b](b.markdown)\n- [c](c.mdown)\n- [d](d.mkd)\n",
            ),
            ("a.md", "# A\n"),
            ("b.markdown", "# B\n"),
            ("c.mdown", "# C\n"),
            ("d.mkd", "# D\n"),
        ],
    );
    let nav = Navigator::new(&corpus.0.join("README.md"), None, Path::new("/"));

    let listed: Vec<String> = nav
        .entries()
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    for name in ["a.md", "b.markdown", "c.mdown", "d.mkd"] {
        assert!(listed.iter().any(|p| p.ends_with(name)), "{name} not in {listed:?}");
        let mut src = nav.load_source(&corpus.0.join(name)).expect(name);
        src.set_width(80);
        assert!(!src.outline().is_empty(), "{name} must load as markdown");
    }
}
