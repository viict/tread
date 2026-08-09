//! A directory, read as a document (SPEC.md §Directories).
//!
//! The listing is built once at open: a directory's entries are cheap to read
//! and there is no way to lay one out without knowing them all, so unlike the
//! big-file formats there is nothing to be lazy about. What it does *not* do is
//! walk into subdirectories — that is what opening one is for.
//!
//! Every entry is a [`LinkSite`], which is the whole trick: `n`, `←`/`→` and
//! `Enter` are the corpus navigation the pager already has, so walking a tree
//! needs no new keys and no second mechanism.
#![deny(unsafe_code)]

use std::fs;
use std::path::{Path, PathBuf};

use super::detect;
use super::{Anchor, LinkSite};
use crate::render::{str_width, Line, LineKind, Span};
use crate::theme;

mod view;

#[cfg(test)]
mod tests;

/// Columns the name is padded to before the size, unless a name is longer.
const NAME_W: usize = 32;

/// One entry, as the listing needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    name: String,
    is_dir: bool,
    /// `None` for a directory, and for a file whose size could not be read.
    size: Option<u64>,
    hidden: bool,
}

impl Item {
    /// Sort key: directories first, then case-insensitive by name. A listing
    /// should read the way a person would write one.
    fn key(&self) -> (bool, String) {
        (!self.is_dir, self.name.to_lowercase())
    }

    /// What the entry is called as a link: a directory keeps its `/`, so the
    /// text and the target agree and a reader can see which is which.
    fn url(&self) -> String {
        match self.is_dir {
            true => format!("{}/", self.name),
            false => self.name.clone(),
        }
    }
}

pub struct DirSource {
    path: PathBuf,
    items: Vec<Item>,
    /// Why the directory could not be read, if it could not be.
    failed: Option<String>,
    show_hidden: bool,
    rows: Vec<Line>,
    links: Vec<LinkSite>,
    /// The live search query, and the rows it matches.
    query: String,
    matches: Vec<usize>,
    current: Option<usize>,
}

impl DirSource {
    /// List `path`. An unreadable directory is a listing that says so, not an
    /// error: the reader got there by following a link and should be told what
    /// happened without losing the document it came from.
    pub fn open(path: &Path) -> DirSource {
        let (items, failed) = match fs::read_dir(path) {
            Ok(rd) => (collect(rd), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let mut s = DirSource {
            path: path.to_path_buf(),
            items,
            failed,
            show_hidden: false,
            rows: Vec::new(),
            links: Vec::new(),
            query: String::new(),
            matches: Vec::new(),
            current: None,
        };
        s.build();
        s
    }

    /// `a`: show or hide the dotfiles, and say which state it is in.
    pub(super) fn flip_hidden(&mut self) -> String {
        self.show_hidden = !self.show_hidden;
        self.build();
        let n = self.hidden_count();
        match self.show_hidden {
            true => format!("showing {n} hidden {}", entries(n)),
            false => format!("hiding {n} hidden {}", entries(n)),
        }
    }

    fn hidden_count(&self) -> usize {
        self.items.iter().filter(|i| i.hidden).count()
    }

    /// The entries currently on screen, in order.
    fn shown(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .filter(move |i| self.show_hidden || !i.hidden)
    }

    /// Lay the listing out. Cheap enough to redo whenever the state changes,
    /// which is what keeps `show_hidden` from needing incremental bookkeeping.
    fn build(&mut self) {
        self.rows.clear();
        self.links.clear();

        let hidden = self.hidden_count();
        self.rows.push(header(&self.path, self.shown().count(), hidden));
        self.rows.push(blank());

        if let Some(why) = self.failed.clone() {
            self.rows.push(note(&format!("cannot be read: {why}")));
            return;
        }

        let items: Vec<Item> = self.shown().cloned().collect();
        for item in &items {
            let row = self.rows.len();
            self.rows.push(entry_line(item));
            // The link starts after the two-column gutter every row carries.
            self.links.push(LinkSite {
                anchor: Anchor(row),
                col: 2,
                url: item.url(),
            });
        }

        if items.is_empty() {
            self.rows.push(note("empty"));
        }
        if hidden > 0 && !self.show_hidden {
            self.rows.push(blank());
            self.rows
                .push(note(&format!("press a to show {hidden} hidden {}", entries(hidden))));
        }
        self.rematch();
    }

    /// Recompute the search matches for the current query and rows.
    fn rematch(&mut self) {
        self.matches.clear();
        self.current = None;
        if self.query.is_empty() {
            return;
        }
        let needle = self.query.to_lowercase();
        for (i, l) in self.rows.iter().enumerate() {
            if l.text().to_lowercase().contains(&needle) {
                self.matches.push(i);
            }
        }
    }
}

fn entries(n: usize) -> &'static str {
    match n == 1 {
        true => "entry",
        false => "entries",
    }
}

/// Read the entries, sorted. A name that is not valid UTF-8 is kept as its
/// lossy spelling rather than dropped: a file that exists should be listed even
/// if it cannot be named exactly.
fn collect(rd: fs::ReadDir) -> Vec<Item> {
    let mut out: Vec<Item> = rd
        .filter_map(Result::ok)
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let meta = e.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            Item {
                hidden: name.starts_with('.'),
                size: match is_dir {
                    true => None,
                    false => meta.as_ref().map(|m| m.len()),
                },
                name,
                is_dir,
            }
        })
        .collect();
    out.sort_by_key(Item::key);
    out
}

/// `▾ /some/dir · 12 entries · 2 hidden`
fn header(path: &Path, shown: usize, hidden: usize) -> Line {
    let mut spans = vec![
        Span::new(format!("{} ", theme::MARKER_OPEN), theme::gutter()),
        Span::new(path.display().to_string(), theme::heading(1)),
        Span::new(
            format!("  \u{b7}  {shown} {}", entries(shown)),
            theme::muted(),
        ),
    ];
    if hidden > 0 {
        spans.push(Span::new(
            format!("  \u{b7}  {hidden} hidden"),
            theme::muted(),
        ));
    }
    line(spans, LineKind::Paragraph)
}

/// One entry: `  name/` for a directory, `  name    size   format` for a file.
fn entry_line(item: &Item) -> Line {
    let text = item.url();
    let mut spans = vec![
        Span::new("  ", theme::text()),
        Span::new(text.clone(), link_style(item)),
    ];
    if let Some(size) = item.size {
        let pad = NAME_W.saturating_sub(str_width(&text)) + 1;
        spans.push(Span::new(" ".repeat(pad), theme::text()));
        spans.push(Span::new(format!("{:>8}", human(size)), theme::muted()));
        if let Some(f) = format_of(&item.name) {
            spans.push(Span::new(format!("   {f}"), theme::muted()));
        }
    }
    line(spans, LineKind::Paragraph)
}

/// A directory is not the link blue: it goes somewhere else in kind, and the
/// distinction is worth seeing before pressing Enter.
///
/// Every entry is a local path, so the internal-link colour is always the right
/// one — `render::inline` owns the internal/external decision and a listing has
/// no external entries to decide about.
fn link_style(item: &Item) -> crate::term::Style {
    match item.is_dir {
        true => theme::heading(3),
        false => crate::term::Style::new().fg(theme::LINK).underline(),
    }
}

/// What `tread` would read the entry as, or nothing when it has no opinion
/// worth printing (plain text is the default, and saying so on every row is
/// noise).
fn format_of(name: &str) -> Option<&'static str> {
    match detect::from_path(Path::new(name))? {
        detect::Format::Markdown => Some("markdown"),
        detect::Format::Csv => Some("csv"),
        detect::Format::Json => Some("json"),
        detect::Format::Jsonl => Some("records"),
        detect::Format::Code => Some("code"),
        detect::Format::Text => None,
    }
}

/// A size a person can read at a glance. Binary units, because a file listing
/// is closer to `ls` than to a disk vendor.
fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    match u {
        0 => format!("{bytes} B"),
        _ if v < 10.0 => format!("{v:.1} {}", UNITS[u]),
        _ => format!("{v:.0} {}", UNITS[u]),
    }
}

fn note(text: &str) -> Line {
    line(
        vec![Span::new(format!("  {text}"), theme::muted())],
        LineKind::Paragraph,
    )
}

fn blank() -> Line {
    line(Vec::new(), LineKind::Blank)
}

fn line(spans: Vec<Span>, kind: LineKind) -> Line {
    Line {
        spans,
        block: 0,
        source_line: 1,
        heading: None,
        scroll: false,
        kind,
    }
}
