//! Navigation tests. Path resolution runs against an in-memory filesystem so
//! it is deterministic; the index parser and slug matching are additionally
//! checked against the real codex corpus when it is present on this machine.
#![deny(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::index;
use super::link::{self, Fs, Target};
use super::{absolutize, discover, Navigator};
use crate::md;
use crate::md::ast::{Block, SlugSet};

// -- in-memory filesystem ----------------------------------------------------

#[derive(Default)]
struct MemFs {
    files: HashMap<PathBuf, String>,
}

impl MemFs {
    fn with(paths: &[(&str, &str)]) -> MemFs {
        MemFs {
            files: paths
                .iter()
                .map(|(p, c)| (PathBuf::from(p), c.to_string()))
                .collect(),
        }
    }
}

impl Fs for MemFs {
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.files.keys().any(|f| f.parent() == Some(path))
            || self.files.keys().any(|f| f.starts_with(path) && f != path)
    }
    fn read(&self, path: &Path) -> Result<String, String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("{}: not found", path.display()))
    }
}

const ROOT: &str = "/corpus";

fn fs() -> MemFs {
    MemFs::with(&[
        ("/corpus/README.md", "# Index\n\n[a](models/A.md)\n"),
        ("/corpus/CONVENTIONS.md", "# Conventions\n"),
        ("/corpus/models/A.md", "# A\n\n## Deep Heading\n"),
        ("/corpus/models/B.md", "# B\n"),
        ("/corpus/models/README.md", "# Models\n"),
        ("/corpus/assets/diagram.png", "binary"),
        ("/outside/SECRET.md", "# Secret\n"),
    ])
}

fn resolve(raw: &str, from: &str) -> Target {
    link::resolve(raw, Path::new(from), Path::new(ROOT), &fs())
}

// -- path resolution ---------------------------------------------------------

#[test]
fn relative_links_resolve_against_the_document_not_the_cwd() {
    assert_eq!(
        resolve("B.md", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/B.md"),
            anchor: None
        }
    );
    // The same link text from the root resolves somewhere else entirely.
    assert!(matches!(
        resolve("B.md", "/corpus"),
        Target::Broken { .. }
    ));
    assert_eq!(
        resolve("models/A.md", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: None
        }
    );
}

#[test]
fn dot_and_dotdot_are_folded() {
    assert_eq!(
        resolve("../CONVENTIONS.md", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/CONVENTIONS.md"),
            anchor: None
        }
    );
    assert_eq!(
        resolve("./B.md", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/B.md"),
            anchor: None
        }
    );
    assert_eq!(
        resolve("../models/./../models/B.md", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/B.md"),
            anchor: None
        }
    );
}

#[test]
fn escapes_above_the_index_root_are_rejected() {
    for raw in ["../../outside/SECRET.md", "../outside/SECRET.md", "../../../../etc/passwd"] {
        match resolve(raw, "/corpus/models") {
            Target::Broken { why, .. } => assert!(
                why.contains("escapes") || why.contains("no such file"),
                "{raw}: {why}"
            ),
            other => panic!("{raw} should not resolve: {other:?}"),
        }
    }
    // …and the message names the escape, so the status bar can say so.
    let t = resolve("../../outside/SECRET.md", "/corpus/models");
    assert!(t.describe(Path::new(ROOT)).contains("escapes the index root"));
}

#[test]
fn a_directory_resolves_to_its_readme() {
    assert_eq!(
        resolve("models", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/README.md"),
            anchor: None
        }
    );
    assert_eq!(
        resolve("models/", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/README.md"),
            anchor: None
        }
    );
}

#[test]
fn an_extensionless_target_resolves_to_the_md_file() {
    assert_eq!(
        resolve("CONVENTIONS", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/CONVENTIONS.md"),
            anchor: None
        }
    );
    assert_eq!(
        resolve("../CONVENTIONS#when-in-doubt", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/CONVENTIONS.md"),
            anchor: Some("when-in-doubt".to_string())
        }
    );
}

#[test]
fn root_absolute_links_resolve_against_the_corpus_root() {
    assert_eq!(
        resolve("/models/B.md", "/corpus/models"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/B.md"),
            anchor: None
        }
    );
}

#[test]
fn link_kinds_are_classified() {
    assert!(matches!(
        resolve("https://example.com/x", "/corpus"),
        Target::External(_)
    ));
    assert!(matches!(
        resolve("mailto:a@b.c", "/corpus"),
        Target::External(_)
    ));
    assert!(matches!(resolve("#some-heading", "/corpus"), Target::Anchor(a) if a == "some-heading"));
    assert!(matches!(
        resolve("assets/diagram.png", "/corpus"),
        Target::Other(_)
    ));
    assert!(matches!(resolve("nope.md", "/corpus"), Target::Broken { .. }));
    assert!(matches!(resolve("", "/corpus"), Target::Broken { .. }));
}

#[test]
fn cross_document_anchors_carry_a_slug() {
    assert_eq!(
        resolve("models/A.md#Deep Heading", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: Some("deep-heading".to_string())
        }
    );
    assert_eq!(
        resolve("models/A.md#deep-heading", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: Some("deep-heading".to_string())
        }
    );
}

#[test]
fn percent_escapes_and_schemes() {
    assert_eq!(link::percent_decode("a%20b"), "a b");
    assert_eq!(link::percent_decode("a%zzb"), "a%zzb");
    assert_eq!(link::scheme_of("https://x"), Some("https"));
    assert_eq!(link::scheme_of("models/DNS:MODEL.md"), None);
    assert_eq!(link::scheme_of("./x"), None);
}

#[test]
fn describe_and_yank_use_the_right_text() {
    let root = Path::new(ROOT);
    let doc = resolve("models/A.md#deep-heading", "/corpus");
    assert_eq!(doc.describe(root), "models/A.md#deep-heading");
    let ext = resolve("https://example.com/x", "/corpus");
    assert_eq!(ext.describe(root), "https://example.com/x");
    assert_eq!(ext.yank_text(root), "https://example.com/x");
    assert_eq!(doc.yank_text(root), "models/A.md");
}

// -- root discovery ----------------------------------------------------------

#[test]
fn explicit_index_wins_and_accepts_a_directory() {
    let fs = fs();
    let (root, idx) = discover(&fs, Path::new("/corpus/models/A.md"), Some(Path::new("/corpus")));
    assert_eq!(root, PathBuf::from("/corpus"));
    assert_eq!(idx, Some(PathBuf::from("/corpus/README.md")));
    let (root, idx) = discover(
        &fs,
        Path::new("/corpus/models/A.md"),
        Some(Path::new("/corpus/README.md")),
    );
    assert_eq!((root, idx), (PathBuf::from("/corpus"), Some(PathBuf::from("/corpus/README.md"))));
}

#[test]
fn discovery_walks_up_to_the_readme_that_links_to_the_file() {
    let fs = fs();
    // /corpus/models/README.md exists but does not link to A.md, so the walk
    // continues to /corpus/README.md, which does.
    let (root, idx) = discover(&fs, Path::new("/corpus/models/A.md"), None);
    assert_eq!(root, PathBuf::from("/corpus"));
    assert_eq!(idx, Some(PathBuf::from("/corpus/README.md")));
}

#[test]
fn discovery_falls_back_to_the_files_own_directory() {
    let fs = MemFs::with(&[("/lone/NOTES.md", "# Notes\n")]);
    let (root, idx) = discover(&fs, Path::new("/lone/NOTES.md"), None);
    assert_eq!(root, PathBuf::from("/lone"));
    assert_eq!(idx, None);
}

#[test]
fn absolutize_normalizes_relative_paths() {
    assert_eq!(
        absolutize(Path::new("../a/./b.md"), Path::new("/x/y")),
        PathBuf::from("/x/a/b.md")
    );
    assert_eq!(
        absolutize(Path::new("/x/../y.md"), Path::new("/ignored")),
        PathBuf::from("/y.md")
    );
}

/// The nav layer is platform-agnostic because it delegates to `plat::path`,
/// whose Windows rules are proved in `plat::path_tests` from any host. What is
/// checked here is the *wiring*: that these entry points speak this platform's
/// dialect rather than a hardcoded `/` one.
#[test]
fn path_handling_speaks_this_platforms_dialect() {
    use crate::plat::{path as ppath, Platform};
    let sep = ppath::sep(Platform::HOST);
    let root = PathBuf::from(ppath::join(Platform::HOST, "/corpus", "").unwrap());
    let deep = PathBuf::from(ppath::join(Platform::HOST, "/corpus", "models/a.md").unwrap());
    // Joining speaks the native dialect...
    assert!(deep.to_string_lossy().contains(&format!("models{}a.md", ppath::sep(Platform::HOST))));
    // ...but the corpus-relative name a user sees and yanks is always `/`.
    assert_eq!(link::rel_to(&deep, &root), "models/a.md");
    assert!(link::within_root(&deep, &root));
    assert!(!link::within_root(Path::new("/elsewhere/a.md"), &root));
    // Identity survives a `.` and a redundant separator.
    let noisy = PathBuf::from(format!("{}{sep}.{sep}models{sep}a.md", root.display()));
    assert!(link::same_path(&deep, &noisy));
    assert!(!link::same_path(&deep, &root));
    // A link that walks above the root is still an escape, not a fold to `/`.
    assert_eq!(link::normalize(&root, "../../etc/passwd"), None);
}

// -- index parsing -----------------------------------------------------------

const INDEX_SRC: &str = "\
# Corpus

## Models

| Doc | Status |
|---|---|
| [models/A.md](models/A.md) — the A model | Active |
| [models/B.md](models/B.md) | Draft |

## Links we ignore

- [external](https://example.com)
- [picture](assets/diagram.png)
- [again](models/A.md)
";

fn parsed_index() -> Vec<index::Entry> {
    let doc = md::parse(INDEX_SRC);
    index::parse(&doc, Path::new(ROOT), Path::new(ROOT), &fs())
}

#[test]
fn index_groups_by_section_and_keeps_trailing_text() {
    let e = parsed_index();
    assert_eq!(e.len(), 2, "external, non-markdown and repeat links drop out");
    assert_eq!(e[0].section, "Models");
    assert_eq!(e[0].title, "models/A.md");
    assert_eq!(e[0].path, PathBuf::from("/corpus/models/A.md"));
    assert_eq!(e[0].desc, "the A model");
    // With no trailing text the row's other columns become the description.
    assert_eq!(e[1].desc, "Draft");
    assert!(e[0].row().contains("the A model"));
    assert!(e[0].haystack().contains("models/a.md"));
}

#[test]
fn navigator_walks_the_corpus_in_index_order() {
    let files = [
        ("/corpus/README.md", INDEX_SRC),
        ("/corpus/models/A.md", "# A\n"),
        ("/corpus/models/B.md", "# B\n"),
        ("/corpus/assets/diagram.png", "x"),
    ];
    let mk = |cur: &str| {
        Navigator::with_fs(
            Box::new(MemFs::with(&files)),
            Path::new(cur),
            None,
            Path::new("/corpus"),
        )
    };
    let nav = mk("/corpus/README.md");
    assert_eq!(nav.entries().len(), 2);
    assert_eq!(nav.sibling(1), Some(PathBuf::from("/corpus/models/A.md")));
    assert_eq!(nav.sibling(-1), None);
    let nav = mk("/corpus/models/A.md");
    assert_eq!(nav.sibling(1), Some(PathBuf::from("/corpus/models/B.md")));
    assert_eq!(nav.sibling(-1), Some(PathBuf::from("/corpus/README.md")));
    let nav = mk("/corpus/models/B.md");
    assert_eq!(nav.sibling(1), None);
    assert_eq!(nav.label(Path::new("/corpus/models/B.md")), "models/B.md");
}

#[test]
fn navigator_history_restores_position() {
    let files = [
        ("/corpus/README.md", INDEX_SRC),
        ("/corpus/models/A.md", "# A\n"),
        ("/corpus/models/B.md", "# B\n"),
    ];
    let mut nav = Navigator::with_fs(
        Box::new(MemFs::with(&files)),
        Path::new("/corpus/README.md"),
        None,
        Path::new("/corpus"),
    );
    let mut here = super::history::Snapshot::of(PathBuf::from("/corpus/README.md"));
    here.top = 30;
    here.cursor = 42;
    here.collapsed = vec!["models".to_string()];
    nav.push(here.clone());
    nav.set_current(PathBuf::from("/corpus/models/A.md"));
    assert_eq!(nav.depth(), 1);
    let back = nav
        .back(super::history::Snapshot::of(PathBuf::from("/corpus/models/A.md")))
        .unwrap();
    assert_eq!(back, here);
    assert!(nav.load(Path::new("/corpus/models/A.md")).is_ok());
    assert!(nav.load(Path::new("/corpus/nope.md")).is_err());
}

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

// -- a corpus that names itself ----------------------------------------------

/// `corpus/models/A.md`, written *inside* `/corpus`. The leading segment is
/// the corpus's own name, so the path only resolves from the root's parent —
/// which is how such a reference gets written in the first place.
#[test]
fn a_path_prefixed_with_the_root_name_resolves_from_the_root() {
    assert_eq!(
        resolve("corpus/models/A.md", "/corpus/decisions"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: None
        }
    );
    // And with an anchor, from the root itself.
    assert_eq!(
        resolve("corpus/models/A.md#deep-heading", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: Some("deep-heading".into())
        }
    );
}

/// The fallback must not invent a file. A path that is wrong in the ordinary
/// way stays wrong rather than being retried into something that exists.
#[test]
fn the_root_name_fallback_never_conjures_a_target() {
    assert!(matches!(
        resolve("corpus/models/NOPE.md", "/corpus/decisions"),
        Target::Broken { .. }
    ));
    assert!(matches!(
        resolve("models/NOPE.md", "/corpus"),
        Target::Broken { .. }
    ));
}

/// A real relative path wins: the fallback only runs when nothing was found,
/// so it cannot shadow a directory that genuinely sits inside the corpus.
#[test]
fn a_real_relative_path_is_preferred_to_the_fallback() {
    assert_eq!(
        resolve("models/A.md", "/corpus"),
        Target::Doc {
            path: PathBuf::from("/corpus/models/A.md"),
            anchor: None
        }
    );
}
