//! Code behind the format seam (SPEC.md §Code).
//!
//! A source file opens as its comments and declarations with every body folded
//! shut. `zR` opens them all — which *is* the raw file, because every line is
//! rendered and only the fold state differs — and `r` toggles between the two
//! in one key.
//!
//! Almost nothing here is new machinery. The symbols come from `crate::code`,
//! the folding from `source::collapse`, and the outline that `o` shows is
//! derived from the same headings the folds are. What this module does is join
//! them and hold the state.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use super::collapse::{self, HeadingRef};
use super::fold::{self, Region};
use super::{Anchor, Entry, FoldState, LinkSite};
use crate::code::{java_decl, py_decl, rust_decl, ts_decl, Kind, Symbol};
use crate::render::Line;

mod jump;
mod paint;
mod render;
mod tsconfig;
mod workspace;
mod view;

#[cfg(test)]
mod tests;

/// Which languages are understood, and by what.
///
/// Adding one is a module in `crate::code` plus a line here — the same shape
/// `docs/lenses.md` sets for lenses. Extensions are matched lowercased.
type Parser = fn(&str) -> Option<Vec<Symbol>>;

const LANGS: &[(&str, &[&str], Parser)] = &[
    ("rust", &["rs"], rust_decl::symbols),
    (
        "typescript",
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        ts_decl::symbols,
    ),
    ("python", &["py", "pyi"], py_decl::symbols),
    ("java", &["java"], java_decl::symbols),
];

/// Every region of the file that can fold: one per symbol body, and one per
/// import run.
///
/// This is where code parts company with prose. A symbol already knows exactly
/// where its body ends — the walker computed it — so the region is stated
/// rather than inferred, and a fold can never reach past the closing brace.
fn regions_of(
    symbols: &[Symbol],
    groups: &[render::Group],
    blocks: &[crate::code::decl::Block],
    len: usize,
) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::new();
    // Blocks first, so a declaration's own region wins the `id` race when both
    // start on the same line — a function's body is the function's, not a
    // nameless block's.
    for (head, body, end) in blocks {
        out.push(Region {
            // Keyed by source line: stable while the file is open, which is all
            // a block fold needs — unlike a symbol, it has no name to survive
            // a re-read under.
            id: format!("block@{head}"),
            // Deeper than any declaration, so folding a function still folds
            // the branches inside it.
            level: 100,
            head: *head,
            body: (*body).min(len),
            end: (*end).min(len),
        });
    }
    for (n, s) in symbols.iter().enumerate() {
        // A symbol swallowed by an import run folds with the run, not alone.
        if groups.iter().any(|g| n > g.first && n < g.first + g.count) {
            continue;
        }
        let group = groups.iter().find(|g| g.first == n);
        let (id, end) = match group {
            Some(g) => {
                let last = &symbols[(g.first + g.count - 1).min(symbols.len() - 1)];
                (g.id.clone(), last.span().1.max(last.sig.1))
            }
            None => (s.path.clone(), s.body.1),
        };
        out.push(Region {
            id,
            level: s.depth + 1,
            head: s.span().0,
            body: s.sig.1.min(len),
            end: end.min(len),
        });
    }
    out
}

/// The blocks of a file, over its blanked source so a brace in a string or a
/// comment cannot open one.
fn block_ranges(lang: &str, src: &str) -> Vec<crate::code::decl::Block> {
    use crate::code::{java, py, py_decl, rust, scan, ts};
    let toks = match lang {
        "rust" => rust::lex(src),
        "typescript" => ts::lex(src),
        "java" => java::lex(src),
        "python" => py::lex(src),
        _ => return Vec::new(),
    };
    let blanked = scan::blank(src, &toks);
    let lines: Vec<&str> = blanked.lines().collect();
    // Three lines is the floor: folding a two-line block replaces it with a
    // marker no shorter than what it hid.
    match lang {
        // Python delimits a suite by indentation; there are no braces to count.
        "python" => py_decl::blocks(&lines, 3),
        _ => crate::code::decl::blocks(&lines, 3),
    }
}

/// The real filesystem, for the workspace reader.
struct RealFiles;

impl workspace::Files for RealFiles {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn list(&self, dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Is this symbol one of the file's imports? Rust's `mod foo;` counts: it names
/// another file exactly as an import does, and reads as one at the top of a
/// module.
fn is_import(s: &Symbol, lang: &str) -> bool {
    s.kind == Kind::Import || (lang == "rust" && s.kind == Kind::Mod)
}

/// Runs of consecutive imports, each folding as one.
///
/// A run rather than "all imports in the file": Rust interleaves `mod` with
/// items, and folding across a function in between would hide the function.
fn import_groups(symbols: &[Symbol], ranges: &[(usize, usize, usize, String)]) -> Vec<render::Group> {
    let mut out: Vec<render::Group> = Vec::new();
    let mut i = 0usize;
    while i < symbols.len() {
        if !matches!(symbols[i].kind, Kind::Import | Kind::Mod) {
            i += 1;
            continue;
        }
        let start = i;
        while i < symbols.len() && matches!(symbols[i].kind, Kind::Import | Kind::Mod) {
            i += 1;
        }
        let count = i - start;
        // One line is not a wall; leave it alone.
        if count < 2 {
            continue;
        }
        let lines: (usize, usize) = (
            symbols[start].span().0,
            symbols[i - 1].span().1.max(symbols[i - 1].sig.1),
        );
        // How many names the run brings in, which is what a reader is really
        // asking: not how many lines, but how much came from where.
        let names = ranges
            .iter()
            .filter(|(l, ..)| *l >= lines.0 && *l < lines.1)
            .count()
            .max(count);
        out.push(render::Group {
            first: start,
            count,
            id: format!("imports-{start}"),
            label: format!("{count} imports"),
            note: format!("{names} symbols from {count} modules"),
        });
    }
    out
}

/// Where each import points, as byte ranges on the lines that spell it.
///
/// One link *per imported name*, not one per statement: `import { A, B }` gives
/// two, so `n` steps between them and `Enter` lands on that declaration in the
/// target instead of at the top of the file. The url carries the name as an
/// anchor, which the target resolves through `goto_id` — its fold ids are
/// symbol paths, so this needs nothing new on the other side.
fn import_ranges(
    path: &Path,
    lang: &str,
    src: &str,
    symbols: &[Symbol],
    exists: &dyn Fn(&Path) -> bool,
    aliases: &tsconfig::Aliases,
    workspace: &workspace::Workspace,
) -> Vec<(usize, usize, usize, String)> {
    let root = jump::crate_root(path);
    let cx = jump::Cx {
        root: root.as_deref(),
        exists,
        aliases,
        workspace,
        files: &RealFiles,
    };
    let raw: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for s in symbols {
        // An import statement is re-measured: the walker stops its signature at
        // the line that opens a brace, which for `import {` is the first of
        // several, leaving the module and the bindings on lines it never saw.
        let (from, to) = match s.kind {
            Kind::Import => jump::statement_span(&raw, s.sig.0, lang),
            _ => (s.sig.0, s.sig.1),
        };
        let stmt: Vec<&str> = raw.get(from..to.min(raw.len())).unwrap_or(&[]).to_vec();
        // A Rust `mod foo;` names a file exactly as an import does.
        let module = match s.kind {
            Kind::Import => match jump::module_of(lang, &stmt) {
                Some(m) => m,
                None => continue,
            },
            Kind::Mod if lang == "rust" => format!("self::{}", s.name),
            _ => continue,
        };
        let Some(target) = jump::resolve(lang, path, &module, &cx) else {
            continue;
        };
        let url = relative(path, &target);
        let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let named = match s.kind {
            Kind::Import => jump::imported_names(lang, &stmt),
            _ => Vec::new(),
        };
        // In Rust a name equal to the file's own stem named the *module*, not
        // something inside it: `use crate::theme;` lands at the top of
        // `theme.rs`, where an anchor could only fail to match.
        //
        // The same rule is wrong for TypeScript, where naming a file after the
        // thing it exports is the convention — `Widget.tsx` exports `Widget` —
        // so applying it there strips the anchor from the commonest import in
        // the language.
        let useful: Vec<_> = named
            .into_iter()
            .filter(|(_, _, _, n)| lang != "rust" || n != stem)
            .collect();
        if useful.is_empty() {
            let len = raw.get(from).map(|l| l.len()).unwrap_or(0);
            out.push((from, 0, len, url));
            continue;
        }
        for (off, a, b, name) in useful {
            out.push((from + off, a, b, format!("{url}#{name}")));
        }
    }
    out
}

/// `target` as a path relative to the directory holding `from`, which is what
/// `nav` resolves against.
///
/// Always relative, with `../` where needed. An absolute path is *not* an
/// escape hatch here: `nav` reads a leading `/` as relative to the corpus root,
/// so `/home/you/project/x.ts` would be looked for under
/// `<root>/home/you/project/x.ts` and reported missing — which is exactly how
/// every cross-directory import came to say "no such file".
fn relative(from: &Path, target: &Path) -> String {
    let dir = from.parent().unwrap_or(Path::new("."));
    let mut a = dir.components().peekable();
    let mut b = target.components().peekable();
    // Drop the shared prefix.
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        if x != y {
            break;
        }
        a.next();
        b.next();
    }
    let ups = a.count();
    let mut out = String::new();
    for _ in 0..ups {
        out.push_str("../");
    }
    if ups == 0 {
        out.push_str("./");
    }
    // Joined with `/` whatever the platform separator is: this is a link, and
    // it reads and resolves the same on Windows as it does anywhere else.
    let rest: Vec<String> = b.map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    out.push_str(&rest.join("/"));
    out
}

/// The language for a path, if this build understands it.
pub fn language_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    LANGS
        .iter()
        .find(|(_, exts, _)| exts.contains(&ext.as_str()))
        .map(|(name, _, _)| *name)
}

fn parser_for(lang: &str) -> Option<Parser> {
    LANGS
        .iter()
        .find(|(name, _, _)| *name == lang)
        .map(|(_, _, f)| *f)
}

pub struct CodeSource {
    lang: &'static str,
    /// Where an import resolves to, as a link on the import's own row. This is
    /// the only jump the reader offers, and the only one it can answer without
    /// guessing (SPEC.md §Code).
    links: Vec<LinkSite>,
    /// Every line of the file, in order. Folds hide, they do not remove.
    lines: Vec<Line>,
    /// Indices into `lines` a fold does not hide. Rows index this.
    visible: Vec<usize>,
    /// Every foldable region of the file, computed once from the symbols.
    ///
    /// Explicit rather than inferred: a code block ends where it closes, and
    /// "until the next heading" would make a folded `if` swallow the statements
    /// after its closing brace (`source::fold`).
    regions: Vec<Region>,
    /// `(signature line, lines it hides)` for each closed fold.
    ///
    /// Keyed by the *last* row the heading owns rather than the first, because
    /// that is the signature — `struct Item {  (7 lines)` reads, while the same
    /// count stranded on the first line of a three-line doc comment does not.
    counts: Vec<(usize, usize)>,
    collapsed: FoldState,
    /// What a folded group says instead of a line count, by fold id.
    notes: Vec<(String, String)>,
    /// Ids worth folding: a `use` or a one-line `const` has no body, and
    /// folding it hides nothing but the blank line after it — which reads as
    /// `(1 line)` against every import in the file.
    foldable: Vec<String>,
    outline: Vec<Entry>,
    /// Set when the file did not lex: there are no symbols, so it is shown as
    /// plain source and the status bar says why.
    unparsed: bool,
    /// The live search query and the rows matching it.
    query: String,
    matches: Vec<usize>,
    current: Option<usize>,
}

impl CodeSource {
    /// Read `path` as code. The whole file is laid out at open, like markdown
    /// and unlike CSV — a source file is small, and every view of it needs the
    /// line list anyway (SPEC.md §Code).
    pub fn open(path: &Path) -> std::io::Result<CodeSource> {
        // Lossily, like every other reader here: an invalid byte becomes
        // `U+FFFD` and the file still opens (SPEC.md §Plain text). Reading it
        // strictly would refuse a source file for one stray byte — a
        // `# coding: iso-8859-5` header is a real thing in a Python tree — and
        // refusing to show a file is worse than showing one character wrong.
        let src = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
        let lang = language_of(path).unwrap_or("text");
        Ok(CodeSource::new(path.to_path_buf(), lang, &src))
    }

    /// `path` names the language and nothing else today; the file's own name
    /// is the pager's to show, and resolving a path to another file is the
    /// jump phase's job (SPEC.md §Code).
    fn new(path: PathBuf, lang: &'static str, src: &str) -> CodeSource {
        // One config lookup per file opened, not per import.
        let aliases = match lang {
            "typescript" => tsconfig::Aliases::load(&path, &|p| std::fs::read_to_string(p).ok()),
            _ => tsconfig::Aliases::none(),
        };
        let ws = match lang {
            "typescript" => workspace::Workspace::load(&path, &RealFiles),
            _ => workspace::Workspace::none(),
        };
        CodeSource::with_fs(path, lang, src, &|p| p.is_file(), &aliases, &ws)
    }

    /// The filesystem and the alias table are parameters, so import resolution
    /// is testable without laying down a tree.
    fn with_fs(
        path: PathBuf,
        lang: &'static str,
        src: &str,
        exists: &dyn Fn(&Path) -> bool,
        aliases: &tsconfig::Aliases,
        workspace: &workspace::Workspace,
    ) -> CodeSource {
        let symbols = parser_for(lang).and_then(|f| f(src));
        let unparsed = symbols.is_none();
        let symbols = symbols.unwrap_or_default();
        let ranges = import_ranges(&path, lang, src, &symbols, exists, aliases, workspace);
        let groups = import_groups(&symbols, &ranges);
        let lines = render::rows(lang, src, &symbols, &ranges, &groups);
        // Blocks are foldable but never folded on open: a reader opens a
        // function *to* read it, and finding its branches shut would be one
        // more thing to undo.
        let blocks = block_ranges(lang, src);
        let regions = regions_of(&symbols, &groups, &blocks, lines.len());
        // Columns come from the spans themselves, so a tab before a link does
        // not move it.
        let links: Vec<LinkSite> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, l)| {
                l.links()
                    .into_iter()
                    .map(move |(col, url)| LinkSite {
                        anchor: Anchor(i),
                        col,
                        url: url.to_string(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        // Imports fold as *runs*, never one at a time: folding a single import
        // hides nothing but the blank line after it, while the block of them is
        // a wall a reader scrolls past rather than reads.
        let mut foldable: Vec<String> = symbols
            .iter()
            .filter(|s| s.hidden() > 0 && !is_import(s, lang))
            .map(|s| s.path.clone())
            .collect();
        foldable.extend(groups.iter().map(|g| g.id.clone()));
        let notes: Vec<(String, String)> =
            groups.iter().map(|g| (g.id.clone(), g.note.clone())).collect();
        let mut s = CodeSource {
            lang,
            links,
            regions,
            // A file with no symbols has nothing to fold, so it opens flat —
            // which is the honest thing to show for a file we could not read.
            collapsed: match unparsed {
                true => Vec::new(),
                false => foldable.clone(),
            },
            foldable,
            notes,
            lines,
            visible: Vec::new(),
            counts: Vec::new(),
            outline: Vec::new(),
            unparsed,
            query: String::new(),
            matches: Vec::new(),
            current: None,
        };
        s.refresh();
        s
    }

    /// Recompute the visible list, the fold counts and the outline.
    fn refresh(&mut self) {
        self.visible = fold::visible(self.lines.len(), &self.regions, &self.collapsed);
        self.counts = fold::counts(&self.regions, &self.collapsed, fold::Note::LastOwn);
        self.outline = collapse::headings(&self.lines)
            .into_iter()
            .map(|h: HeadingRef| Entry {
                level: h.level,
                folded: self.collapsed.contains(&h.id),
                id: h.id,
                text: h.text,
                anchor: Anchor(h.index),
            })
            .collect();
        self.rematch();
    }

    /// True when everything that can be folded is folded.
    fn all_folded(&self) -> bool {
        !self.foldable.is_empty() && self.foldable.iter().all(|id| self.collapsed.contains(id))
    }

    /// The line index behind a row, if the row exists.
    fn at(&self, row: usize) -> Option<usize> {
        self.visible.get(row).copied()
    }

    /// `r`: the summary, or the source. Folding everything open *is* the raw
    /// file, so this is one toggle rather than a second view to keep in step.
    fn flip_source(&mut self) -> String {
        match self.all_folded() {
            true => {
                self.collapsed.clear();
                self.refresh();
                "showing source".into()
            }
            false => {
                self.collapsed = self.foldable.clone();
                self.refresh();
                "showing symbols".into()
            }
        }
    }

    /// Rows a fold does not hide, clamped to the row count.
    fn line_rows(&self, rows: std::ops::Range<usize>) -> Vec<usize> {
        let end = rows.end.min(self.visible.len());
        let start = rows.start.min(end);
        self.visible[start..end].to_vec()
    }

    fn rematch(&mut self) {
        self.matches.clear();
        self.current = None;
        if self.query.is_empty() {
            return;
        }
        let needle = self.query.to_lowercase();
        for (row, &i) in self.visible.iter().enumerate() {
            if self.lines[i].text().to_lowercase().contains(&needle) {
                self.matches.push(row);
            }
        }
    }
}
