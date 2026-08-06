//! Corpus navigation: link resolution, the document history stack, the index
//! listing and sequential document order.
//!
//! [`Navigator`] is the pager's whole view of the world outside the current
//! document. It owns the corpus root, the parsed index, the history stack and
//! the filesystem seam; the pager asks it to resolve a link or hand over the
//! next document, and never touches a path itself.
#![deny(unsafe_code)]

pub mod history;
pub mod index;
pub mod link;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::md::{self, Document};
use history::{History, Snapshot};
use index::Entry;
use link::{Fs, RealFs, Target};

/// How far up the tree root discovery is willing to walk.
const MAX_ASCENT: usize = 16;

pub struct Navigator {
    root: PathBuf,
    index_path: Option<PathBuf>,
    current: PathBuf,
    entries: Vec<Entry>,
    history: History,
    fs: Box<dyn Fs>,
}

impl Navigator {
    /// Open `file` inside the corpus implied by `explicit_index` (`--index`) or
    /// discovered around it. Paths are made absolute against `cwd`.
    pub fn new(file: &Path, explicit_index: Option<&Path>, cwd: &Path) -> Navigator {
        Navigator::with_fs(Box::new(RealFs), file, explicit_index, cwd)
    }

    pub fn with_fs(
        fs: Box<dyn Fs>,
        file: &Path,
        explicit_index: Option<&Path>,
        cwd: &Path,
    ) -> Navigator {
        let current = absolutize(file, cwd);
        let explicit = explicit_index.map(|p| absolutize(p, cwd));
        let (root, index_path) = discover(fs.as_ref(), &current, explicit.as_deref());
        let mut nav = Navigator {
            root,
            index_path,
            current,
            entries: Vec::new(),
            history: History::new(),
            fs,
        };
        nav.reload_index();
        nav
    }

    fn reload_index(&mut self) {
        let path = match &self.index_path {
            Some(p) => p.clone(),
            None => return,
        };
        let dir = parent_of(&path);
        if let Ok(text) = self.fs.read(&path) {
            let doc = md::parse(&text);
            self.entries = index::parse(&doc, &dir, &self.root, self.fs.as_ref());
        }
    }

    // -- queries -------------------------------------------------------------

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn current(&self) -> &Path {
        &self.current
    }
    pub fn index_path(&self) -> Option<&Path> {
        self.index_path.as_deref()
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn depth(&self) -> usize {
        self.history.depth()
    }
    /// Status-bar label: the document path relative to the index root.
    pub fn label(&self, path: &Path) -> String {
        link::rel_to(path, &self.root)
    }

    // -- links ---------------------------------------------------------------

    /// Resolve a raw link destination against the *current* document.
    pub fn resolve(&self, raw: &str) -> Target {
        let dir = parent_of(&self.current);
        link::resolve(raw, &dir, &self.root, self.fs.as_ref())
    }

    /// Read and parse a document.
    pub fn load(&self, path: &Path) -> Result<Document, String> {
        self.fs.read(path).map(|t| md::parse(&t))
    }

    pub fn set_current(&mut self, path: PathBuf) {
        self.current = path;
    }

    // -- history -------------------------------------------------------------

    pub fn push(&mut self, current: Snapshot) {
        self.history.push(current);
    }
    pub fn back(&mut self, current: Snapshot) -> Option<Snapshot> {
        self.history.back(current)
    }
    pub fn forward(&mut self, current: Snapshot) -> Option<Snapshot> {
        self.history.forward(current)
    }

    // -- sequential order ----------------------------------------------------

    /// The document `delta` steps away in index order. The index itself sits
    /// just before the first entry, so `[` from the first document lands back
    /// on the index and `]` from the index opens the first document.
    pub fn sibling(&self, delta: isize) -> Option<PathBuf> {
        if self.entries.is_empty() {
            return None;
        }
        let at = match self.entries.iter().position(|e| e.path == self.current) {
            Some(i) => i as isize,
            None if Some(self.current.as_path()) == self.index_path() => -1,
            None => return None,
        };
        let next = at + delta;
        if next < 0 {
            return match at {
                0 => self.index_path.clone(),
                _ => None,
            };
        }
        self.entries.get(next as usize).map(|e| e.path.clone())
    }
}

/// Make `path` absolute and lexically normal.
pub fn absolutize(path: &Path, cwd: &Path) -> PathBuf {
    let joined = match path.is_absolute() {
        true => link::normalize(Path::new("/"), &path.to_string_lossy()),
        false => link::normalize(cwd, &path.to_string_lossy()),
    };
    joined.unwrap_or_else(|| path.to_path_buf())
}

fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Find the corpus root and its index document.
///
/// The heuristic, in order:
///
/// 1. `--index <PATH>` wins outright. A directory means `<PATH>/README.md`; a
///    file is used as-is. The root is the directory holding it.
/// 2. Otherwise walk up from the opened file, at most `MAX_ASCENT` levels,
///    looking for a `README.md` that actually *links to* the opened file by a
///    relative path. The nearest such README is the index — "links to me" is
///    what makes a README an index rather than just a neighbouring file, and it
///    is cheap to check because we have a markdown parser already.
/// 3. Failing that, fall back to the opened file's own directory as the root,
///    with its `README.md` as the index if one exists (and none at all if not).
///    A lone file therefore still gets working relative links, just no corpus.
pub fn discover(fs: &dyn Fs, file: &Path, explicit: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    if let Some(p) = explicit {
        let index = match fs.is_dir(p) {
            true => p.join("README.md"),
            false => p.to_path_buf(),
        };
        return (parent_of(&index), Some(index));
    }
    let start = parent_of(file);
    let mut dir = start.clone();
    for _ in 0..MAX_ASCENT {
        let readme = dir.join("README.md");
        if readme != file && fs.is_file(&readme) && links_to(fs, &readme, file, &dir) {
            return (dir.clone(), Some(readme));
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => break,
        }
    }
    let own = start.join("README.md");
    let index = match fs.is_file(&own) {
        true => Some(own),
        false => None,
    };
    (start, index)
}

/// Does `readme` contain a relative link that resolves to `file`?
fn links_to(fs: &dyn Fs, readme: &Path, file: &Path, dir: &Path) -> bool {
    let text = match fs.read(readme) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let doc = md::parse(&text);
    index::raw_links(&doc).iter().any(|raw| {
        matches!(
            link::resolve(raw, dir, dir, fs),
            Target::Doc { ref path, .. } if path == file
        )
    })
}
