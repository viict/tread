//! Navigation against the *real* target corpus (`~/notes`), when this machine
//! has it.
//!
//! Split out of `tests.rs` to keep both files under the size limit, and it is a
//! natural seam: everything here is conditional on a corpus that CI does not
//! have, and skips quietly when it is absent so `cargo test` stays green
//! anywhere. Everything in `tests.rs` runs against an in-memory filesystem and
//! is deterministic.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use super::link::{self, Target};
use super::Navigator;
use crate::md;
use crate::md::ast::{Block, SlugSet};

// -- the real corpus ---------------------------------------------------------

/// The real corpus, when this machine has it. Tests that need it skip quietly
/// when it is absent so `cargo test` stays green anywhere.
fn codex() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home).join("notes");
    p.join("README.md").is_file().then_some(p)
}

#[test]
fn real_codex_index_lists_the_real_documents() {
    let root = match codex() {
        Some(p) => p,
        None => return,
    };
    let nav = Navigator::new(&root.join("README.md"), None, Path::new("/"));
    let entries = nav.entries();
    assert!(entries.len() > 80, "only {} entries", entries.len());
    let dns = entries
        .iter()
        .find(|e| e.path.ends_with("models/SAMPLE_MODEL.md"))
        .expect("SAMPLE_MODEL.md is in the index");
    assert_eq!(dns.section, "Models");
    assert!(dns.desc.contains("sample architecture"), "{}", dns.desc);
    // Every listed target really exists and is markdown.
    for e in entries {
        assert!(e.path.is_file(), "{} is missing", e.path.display());
        assert!(link::is_markdown(&e.path));
        assert!(e.path.starts_with(&root));
    }
    // Sections come from the H2s of the README.
    let sections: Vec<&str> = entries.iter().map(|e| e.section.as_str()).collect();
    for want in ["Foundations", "Models", "Plans", "Decisions", "Operational"] {
        assert!(sections.contains(&want), "no section {want}");
    }
}

#[test]
fn real_codex_anchors_match_generated_heading_ids() {
    let root = match codex() {
        Some(p) => p,
        None => return,
    };
    let text = std::fs::read_to_string(root.join("README.md")).unwrap();
    let doc = md::parse(&text);
    let ids: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert!(ids.contains(&"how-to-read-this".to_string()), "{ids:?}");
    assert!(ids.contains(&"future-directions".to_string()));
    // The anchor a link would carry is exactly the id the parser assigned.
    let anchor = match link::split_fragment("README.md#Future directions") {
        (_, Some(a)) => a,
        _ => panic!("no anchor"),
    };
    assert!(ids.contains(&anchor));
    // Dedup suffixes agree with the parser's SlugSet.
    let mut set = SlugSet::new();
    assert_eq!(set.unique("How to read this"), "how-to-read-this");
    assert_eq!(set.unique("How to read this"), "how-to-read-this-1");
}

#[test]
fn real_codex_root_is_discovered_by_walking_up() {
    let root = match codex() {
        Some(p) => p,
        None => return,
    };
    let deep = root.join("models/SAMPLE_MODEL.md");
    let nav = Navigator::new(&deep, None, Path::new("/"));
    assert_eq!(nav.root(), root.as_path());
    assert_eq!(nav.index_path(), Some(root.join("README.md").as_path()));
    assert_eq!(nav.label(&deep), "models/SAMPLE_MODEL.md");
    // A link inside that document resolves relative to models/, not to cwd.
    assert!(matches!(
        nav.resolve("../CONVENTIONS.md"),
        Target::Doc { ref path, .. } if path == &root.join("CONVENTIONS.md")
    ));
    assert!(matches!(
        nav.resolve("../../etc/passwd"),
        Target::Broken { .. }
    ));
}
