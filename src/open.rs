//! Resolving what to open, and building the [`Source`] behind it.
//!
//! Everything between "here is an argv" and "here is a `Box<dyn Source>`:
//! where the bytes come from (a file, a pipe, the corpus index), which format
//! they are in (SPEC.md §Multi-format reading) and which module reads them.
//! Split out of `main.rs` to keep both under the size limit; this is the only
//! place in the crate that knows the list of formats.
//!
//! A file on disk is *not* read here. Only its first bytes are, and only when
//! the extension says nothing: a multi-GB CSV must reach [`CsvSource`] as a
//! path so it can be indexed lazily (SPEC.md §CSV).
#![deny(unsafe_code)]

pub(crate) mod input;
mod lens;

use std::path::{Path, PathBuf};

use crate::csv::read::Reader;
use crate::source::detect::Format;
use crate::source::json::{self as json, JsonSource};
use crate::source::text::TextSource;
use crate::source::{csv::CsvSource, jsonl::JsonlSource, markdown::MarkdownSource, Source};
use crate::plat::{path as ppath, Platform};
use crate::{cli, md, sys, url};

pub use input::resolve_input;

/// A fatal error plus the process exit code it maps to.
pub struct Fail {
    pub msg: String,
    pub code: i32,
}

impl Fail {
    pub fn runtime(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: crate::EXIT_ERROR,
        }
    }
    pub fn usage(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: crate::EXIT_USAGE,
        }
    }
}

/// The document to display, plus where keyboard input comes from.
pub struct Input {
    pub label: String,
    /// The bytes, when the input arrived on a pipe and cannot be re-read.
    /// `None` for a file on disk, which every format reads for itself — a
    /// multi-GB CSV must never be slurped (SPEC.md §CSV).
    pub bytes: Option<Vec<u8>>,
    /// The file on disk, when there is one (`None` for stdin).
    pub path: Option<PathBuf>,
    pub format: Format,
    /// Descriptor the pager reads keys from, and whether we own it. `None`
    /// means "use stdin" (it is already a terminal).
    pub tty: Option<(sys::Fd, bool)>,
}

impl Drop for Input {
    fn drop(&mut self) {
        if let Some((fd, owned)) = self.tty {
            if owned {
                sys::close_fd(fd);
            }
        }
    }
}


/// Whether this input can navigate: does it produce links that go somewhere?
///
/// Markdown has always had a corpus. A **code file** has one too — its imports
/// are links — and so does a **directory listing**, whose every entry is one.
/// Getting this wrong is silent and total: with no navigator attached, `Enter`
/// on a link falls through to showing the target in the status bar and going
/// nowhere, which reads as "the key does nothing".
///
/// CSV and JSON stay out: they emit no links, so a corpus would only give them
/// a history and an index they never use (SPEC.md §Navigation).
pub fn navigable(input: &Input) -> bool {
    let Some(path) = input.path.as_deref() else {
        // Piped input has no location to resolve anything against.
        return false;
    };
    matches!(input.format, Format::Markdown | Format::Code) || path.is_dir()
}

/// Files that mark the root of a project.
///
/// Checked in this order only so the search is deterministic; any one of them
/// answers "the tree starts here".
const PROJECT_MARKERS: [&str; 6] = [
    ".git",
    "Cargo.toml",
    "package.json",
    "tsconfig.json",
    "go.mod",
    "pyproject.toml",
];

/// `file` as one folded path against `cwd`.
///
/// Folded, not merely joined: `<cwd>/../there` still *contains* `<cwd>` as a
/// component, so climbing it walks back through the directory the path had just
/// left and finds its markers anyway. `absolutize` also keeps Windows `\dir`
/// on the cwd's volume, where a plain join buried it under the cwd.
fn folded(p: Platform, file: &str, cwd: &str) -> String {
    ppath::absolutize(p, cwd, file)
}

/// The project a file belongs to, used as its corpus root.
///
/// A markdown corpus is discovered from a `README.md` that links to the
/// document. Code has no such thing: nothing links to `page.tsx`, so the search
/// falls back to the file's own directory — and then every import of a sibling
/// directory is *outside the corpus* and refused. The project root is the
/// honest answer to "what may this file link to".
///
/// `cwd` is what a relative `file` is relative *to*, and the climb runs against
/// it rather than against the empty path. `Path::parent` of `..` is `""`, which
/// names the working directory — not an ancestor of `../elsewhere` at all — so
/// climbing into it adopted *this* project as the corpus root for a directory
/// that merely sat beside it, and every entry of that listing was then refused
/// for escaping a corpus it had never been in.
///
/// The root this finds is a floor, not the answer: [`widen_root`] takes it
/// from here.
pub fn corpus_root(file: &Path, cwd: &Path) -> Option<PathBuf> {
    let here = PathBuf::from(folded(
        Platform::HOST,
        &file.to_string_lossy(),
        &cwd.to_string_lossy(),
    ));
    let mut at = here.parent()?;
    // Deep enough for any real tree, bounded so a symlink loop cannot spin.
    for _ in 0..64 {
        if PROJECT_MARKERS.iter().any(|m| at.join(m).exists()) {
            return Some(at.to_path_buf());
        }
        at = at.parent()?;
    }
    None
}

/// Widen a corpus until it holds everything the document links to.
///
/// A project marker answers "where does this tree start" for one package, and
/// in a monorepo that is the wrong tree: `apps/client` has its own
/// `package.json`, so it becomes the root, and `../../../../../../packages/ui`
/// — an import the reader *resolved*, to a file that exists — is then refused
/// for escaping a corpus drawn six levels too low.
///
/// The link says where the tree starts. Fold its `../` climb and the directory
/// it bottoms out in holds both the document and the target, by construction;
/// the widest such directory over all of a document's links is the smallest
/// corpus that can hold the document *and* everything it points at. No package
/// manager is consulted, so a Cargo workspace and a `pyproject` tree get the
/// same answer as pnpm.
///
/// Only ever widens. A link that stays inside the project leaves the root
/// alone, and one that resolves to nothing was never a path to widen for.
pub fn widen_root(root: PathBuf, file: &Path, cwd: &Path, links: &[String]) -> PathBuf {
    let s = |x: &Path| x.to_string_lossy().to_string();
    PathBuf::from(widen_on(Platform::HOST, s(&root), &s(file), &s(cwd), links))
}

/// [`widen_root`] over strings, by `host`'s path rules, so every platform's
/// rules are tested on any host.
fn widen_on(host: Platform, root: String, file: &str, cwd: &str, links: &[String]) -> String {
    let Some(dir) = ppath::join(host, &folded(host, file, cwd), "..") else {
        return root;
    };
    let mut root = root;
    for url in links {
        // Only a relative path can climb. A scheme leaves the reader, and a
        // rooted link already means "the corpus root" by definition.
        let raw = url.split(['#', '?']).next().unwrap_or("");
        if raw.is_empty() || url::is_external(url) || ppath::markdown_is_rooted(host, raw) {
            continue;
        }
        let Some(target) = ppath::join(host, &dir, raw) else {
            continue;
        };
        if ppath::contains(host, &root, &target) {
            continue;
        }
        match ppath::common(host, &root, &target) {
            Some(up) => root = up,
            None => continue,
        }
    }
    root
}

/// The document behind the format seam. Which format is this wiring's decision
/// and nothing else's: everything above takes a `Box<dyn Source>`
/// (SPEC.md §The `Source` seam).
pub fn build_source(input: &Input, args: &cli::Args) -> Result<Box<dyn Source>, Fail> {
    if let Some(name) = args.lens.as_deref() {
        // A lens reads records, and each dialect says where its records live
        // (SPEC.md §Lenses). Pointed at anything else it refuses by name
        // rather than rendering a document as one enormous record.
        if !lens::accepts(name, input.format) {
            return Err(lens::needs_records(name, input.format));
        }
    }
    // A directory named on the command line is a listing, not `os error 21`
    // (SPEC.md §Directories). A `README.md` inside it still wins, which is
    // what `index_path` already prefers.
    if let Some(p) = input.path.as_deref() {
        if p.is_dir() {
            return Ok(Box::new(crate::source::dir::DirSource::open(p)));
        }
    }
    match input.format {
        Format::Csv => Ok(Box::new(csv_source(input, args)?)),
        // A document under a lens is its records, not its tree.
        Format::Json if args.lens.is_some() => Ok(Box::new(lens::array_source_with(input, args)?)),
        Format::Json => Ok(Box::new(json_source(input)?)),
        Format::Jsonl => Ok(Box::new(lens::jsonl_source_with(input, args)?)),
        Format::Code => Ok(Box::new(code_source(input)?)),
        Format::Text => Ok(Box::new(text_source(input)?)),
        Format::Markdown => Ok(Box::new(MarkdownSource::new(markdown_document(input)?))),
    }
}

/// A source file. Code needs the whole file — the symbols come from lexing it
/// end to end — so unlike a log there is nothing to be lazy about, and piped
/// input has no path to name a language with.
fn code_source(input: &Input) -> Result<crate::source::code::CodeSource, Fail> {
    let path = match input.path.as_deref() {
        Some(p) => p,
        // Read from a pipe there is no extension to go on. Plain text is the
        // honest fallback rather than guessing a language.
        None => return Err(Fail::usage(String::from("code must be read from a file, not a pipe"))),
    };
    crate::source::code::CodeSource::open(path)
        .map_err(|e| Fail::runtime(format!("{}: {e}", path.display())))
}

/// The text source for this input: from the path when there is one, so a 2GB
/// log is never read whole, and from the piped bytes when there is not.
fn text_source(input: &Input) -> Result<TextSource, Fail> {
    match &input.path {
        Some(path) => TextSource::open(path)
            .map_err(|e| Fail::runtime(format!("{}: {e}", path.display()))),
        None => Ok(TextSource::from_bytes(input.bytes.clone().unwrap_or_default())),
    }
}

/// The CSV source for this input: from the path when there is one, so the file
/// is never read whole, and from the piped bytes when there is not.
fn csv_source(input: &Input, args: &cli::Args) -> Result<CsvSource, Fail> {
    match &input.path {
        Some(path) => CsvSource::open(path, args.delim)
            .map_err(|e| Fail::runtime(format!("{}: {e}", path.display()))),
        None => Ok(CsvSource::from_bytes(
            input.bytes.clone().unwrap_or_default(),
            args.delim,
        )),
    }
}

/// The record source for this input: from the path when there is one, so a
/// multi-GB log is never read whole, and from the piped bytes when there is not.
pub(super) fn jsonl_source(input: &Input) -> Result<JsonlSource, Fail> {
    match &input.path {
        Some(path) => JsonlSource::open(path)
            .map_err(|e| Fail::runtime(format!("{}: {e}", path.display()))),
        None => Ok(JsonlSource::from_bytes(input.bytes.clone().unwrap_or_default())),
    }
}

/// The JSON source for this input: from the path when there is one, so a
/// multi-GB document is never read whole, and from the piped bytes when there
/// is not.
fn json_source(input: &Input) -> Result<JsonSource, Fail> {
    match &input.path {
        Some(path) => {
            JsonSource::open(path).map_err(|e| Fail::runtime(format!("{}: {e}", path.display())))
        }
        None => Ok(JsonSource::from_bytes(input.bytes.clone().unwrap_or_default())),
    }
}

/// `--to-jsonl`: the document's top-level array, one element per line, on
/// `out`. Streams — the document is never held in memory (SPEC.md §JSON).
pub fn to_jsonl(input: &Input, out: &mut dyn std::io::Write) -> Result<(), Fail> {
    let reader = match (&input.path, &input.bytes) {
        (Some(p), _) => {
            Reader::open(p).map_err(|e| Fail::runtime(format!("{}: {e}", p.display())))?
        }
        (None, bytes) => Reader::memory(bytes.clone().unwrap_or_default()),
    };
    json::export::to_jsonl(reader, out).map_err(|e| Fail::runtime(format!("--to-jsonl: {e}")))
}

/// Parse markdown, reading the file now if it was not read already.
pub fn markdown_document(input: &Input) -> Result<md::Document, Fail> {
    let text = match (&input.bytes, &input.path) {
        (Some(raw), _) => md::sanitize::decode(raw.clone()),
        (None, Some(path)) => md::sanitize::read_file(path)
            .map_err(|e| Fail::runtime(format!("{}: {}", path.display(), e)))?,
        (None, None) => String::new(),
    };
    Ok(md::parse(&text))
}

/// `--toc`: the heading outline for markdown, the column names for CSV. One
/// entry per line either way, so the output stays a list a script can read.
pub fn toc_text(input: &Input, args: &cli::Args) -> Result<String, Fail> {
    if input.format == Format::Markdown {
        return Ok(render_outline(&outline(&markdown_document(input)?)));
    }
    // A record file has no headings; the closest thing to an outline is the
    // first records, one summary line each. Bounded, because a `--toc` over a
    // million-record log would be neither a table of contents nor quick.
    // A JSON document's outline is its root's immediate members: the one level
    // that can be listed without walking the file.
    if input.format == Format::Json && args.lens.is_some() {
        let mut src = lens::array_source_with(input, args)?;
        return Ok(src.summaries(TOC_RECORDS).iter().map(|s| format!("{s}\n")).collect());
    }
    if input.format == Format::Json {
        let mut src = json_source(input)?;
        return Ok(src.toc().iter().map(|s| format!("{s}\n")).collect());
    }
    // Plain text has no outline and will not invent one: `--toc` prints
    // nothing and exits 0, which is what "this document has no headings"
    // looks like to the script that asked (SPEC.md §Plain text).
    if input.format == Format::Text {
        return Ok(String::new());
    }
    if input.format == Format::Jsonl {
        let mut src = lens::jsonl_source_with(input, args)?;
        return Ok(src.summaries(TOC_RECORDS).iter().map(|s| format!("{s}\n")).collect());
    }
    let mut src = csv_source(input, args)?;
    src.set_width(crate::dump_width(args));
    Ok(src.columns().iter().map(|c| format!("{c}\n")).collect())
}


/// Records `--toc` lists for a `.jsonl`.
const TOC_RECORDS: usize = 1000;

/// Heading outline, taken from the parsed document so `--toc` and the pager's
/// outline overlay always agree with the renderer.
pub fn outline(doc: &md::Document) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    collect_headings(&doc.blocks, &mut out);
    out
}

fn collect_headings(blocks: &[md::Block], out: &mut Vec<(u8, String)>) {
    for b in blocks {
        match b {
            md::Block::Heading { level, content, .. } => {
                out.push((*level, md::ast::inline_text(content)));
            }
            md::Block::Quote { blocks, .. } => collect_headings(blocks, out),
            _ => {}
        }
    }
}

pub fn render_outline(outline: &[(u8, String)]) -> String {
    let mut s = String::new();
    for (level, text) in outline {
        s.push_str(&" ".repeat((*level as usize - 1) * 2));
        s.push_str(text);
        s.push('\n');
    }
    s
}


#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: Option<&str>, format: Format) -> Input {
        Input {
            label: String::from("t"),
            bytes: None,
            path: path.map(PathBuf::from),
            format,
            tty: None,
        }
    }

    /// The decision that broke `Enter` on a directory listing and on a code
    /// file: with no navigator attached, following a link shows the target in
    /// the status bar and goes nowhere.
    ///
    /// This asserts the *decision*, not the pager. A test that attaches a
    /// navigator by hand passes either way — which is exactly why the bug
    /// survived one.
    #[test]
    fn everything_that_produces_links_gets_a_corpus() {
        // A directory listing: every entry is a link.
        let dir = std::env::temp_dir();
        let d = input(dir.to_str(), Format::Text);
        assert!(navigable(&d), "a directory listing must navigate");

        // A code file: its imports are links.
        assert!(navigable(&input(Some("/p/src/a.rs"), Format::Code)));
        // Markdown, as it always has.
        assert!(navigable(&input(Some("/p/doc.md"), Format::Markdown)));
    }

    /// Without a project root, a code file's corpus is the folder it happens
    /// to sit in, and every import of a sibling directory is refused for
    /// escaping it.
    #[test]
    fn a_projects_root_is_found_from_a_marker_above_the_file() {
        let t = std::env::temp_dir().join(format!("tread-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("src/app/deep")).unwrap();
        std::fs::write(t.join("package.json"), "{}\n").unwrap();
        let f = t.join("src/app/deep/page.tsx");
        std::fs::write(&f, "").unwrap();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert_eq!(corpus_root(&f, &cwd).as_deref(), Some(t.as_path()));
        // Nothing above the filesystem root, and no panic looking.
        assert!(corpus_root(Path::new("/"), &cwd).is_none());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn formats_with_no_links_get_no_corpus() {
        // Data formats emit no links; a corpus would only add a history and an
        // index they never use.
        assert!(!navigable(&input(Some("/p/a.csv"), Format::Csv)));
        assert!(!navigable(&input(Some("/p/a.json"), Format::Json)));
        assert!(!navigable(&input(Some("/p/a.jsonl"), Format::Jsonl)));
        // A plain-text *file* is not a listing and has nothing to follow.
        assert!(!navigable(&input(Some("/p/a.txt"), Format::Text)));
        // Piped input has no location to resolve anything against.
        assert!(!navigable(&input(None, Format::Markdown)));
    }

    /// `widen_on` over `&str` links, for the tables below.
    fn widen(p: Platform, root: &str, file: &str, cwd: &str, links: &[&str]) -> String {
        let links: Vec<String> = links.iter().map(|l| l.to_string()).collect();
        widen_on(p, root.to_string(), file, cwd, &links)
    }

    /// The monorepo case, which the project markers get wrong: `apps/client`
    /// has its own `package.json`, so the corpus stops one package short of the
    /// import that climbs out of it. On Windows `\repo` is rooted on the cwd's
    /// volume: it must not be joined under the cwd.
    #[test]
    fn a_corpus_widens_to_hold_what_the_document_links_to() {
        let post = "/repo/apps/client/src/components/Blog/post.ts";
        let win_post = r"C:\repo\apps\client\src\components\Blog\post.ts";
        for (p, root, file, cwd, up) in [
            (Platform::Linux, "/repo/apps/client", post, "/anywhere", "/repo"),
            (Platform::Windows, r"C:\repo\apps\client", post, r"C:\anywhere", r"C:\repo"),
            (Platform::Windows, r"C:\repo\apps\client", win_post, r"D:\x", r"C:\repo"),
        ] {
            let links = [
                "../../../../../packages/ui/src/entitlements/index.ts#parseEntitlements",
                "../../constants/entitlements.ts#SLUG_TO_LEVEL",
            ];
            let got = widen(p, root, file, cwd, &links);
            assert!(ppath::same(p, &got, up), "{p:?} {file}: the repository is the corpus, got {got}");
            // A document whose links all stay inside it keeps the root it had.
            assert_eq!(widen(p, root, file, cwd, &["../../constants/entitlements.ts"]), root);
            // Neither a URL nor a root-relative link says anything about the tree.
            let other = ["https://example.com/x", "/src/other.ts", "#Symbol"];
            assert_eq!(widen(p, root, file, cwd, &other), root, "{p:?}");
        }
    }

    /// A relative argument must fold against the working directory first, or
    /// the climb is measured from nowhere and nothing widens.
    #[test]
    fn widening_folds_a_relative_document_against_the_cwd() {
        let links = ["../../../packages/ui/index.ts"];
        let got = widen(Platform::Linux, "/repo/apps/client", "client/src/post.ts", "/repo/apps", &links);
        assert_eq!(got, "/repo");
        let (root, file, cwd) = (r"C:\repo\apps\client", r"client\src\post.ts", r"C:\repo\apps");
        let got = widen(Platform::Windows, root, file, cwd, &links);
        assert!(ppath::same(Platform::Windows, &got, r"C:\repo"), "{got}");
    }
}
