//! Resolving an import to a file on disk.
//!
//! This is the "go to definition" a reader actually wants and the only one that
//! can be answered honestly without types: an import *names* a module, so
//! following it is a lookup, not a guess. Resolving an arbitrary identifier
//! would need to know which of four `new`s was meant, and a wrong jump is worse
//! than none (SPEC.md §Code).
//!
//! The filesystem is reached through an `exists` closure rather than directly,
//! so every rule below is tested on the host without laying down files.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use super::tsconfig::Aliases;
use super::workspace::Workspace;

/// Extensions tried for a JavaScript or TypeScript import, in order. A `.ts`
/// wins over a `.js` because in a mixed tree the `.js` is usually the build
/// output of the `.ts` next to it.
const TS_EXTS: [&str; 7] = ["ts", "tsx", "d.ts", "js", "jsx", "mjs", "cjs"];

/// Where an import points, as a path, or `None` when it cannot be followed.
///
/// `file` is the importing file itself — not its directory, because Rust's
/// `super::` depends on the file's *name*: from `csv/delim.rs` it means the
/// `csv` module, whose files sit in `csv/`, while from `csv/mod.rs` it means
/// the parent of `csv`. `root` is the crate root for Rust, unused for
/// TypeScript.
pub fn resolve(lang: &str, file: &Path, module: &str, cx: &Cx) -> Option<PathBuf> {
    let dir = file.parent()?;
    match lang {
        "typescript" => ts(dir, module, cx),
        "rust" => rust(file, cx.root?, module, cx.exists),
        _ => None,
    }
}

/// Everything resolution needs besides the import itself: where the project
/// starts, what it calls things, and how to look at the disk.
///
/// One value rather than six parameters — they travel together through every
/// call, and each new source of truth (aliases, then workspaces) would
/// otherwise widen the signature again.
pub struct Cx<'a> {
    /// Crate root, for Rust's `crate::`.
    pub root: Option<&'a Path>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    pub aliases: &'a Aliases,
    pub workspace: &'a Workspace,
    pub files: &'a dyn super::workspace::Files,
}

/// Where this file's own submodules live, and where its siblings do.
///
/// `a/mod.rs` *is* module `a`: its children are in `a/` and its siblings one
/// level up. `a/b.rs` is module `a::b`: its children are in `a/b/` and its
/// siblings in `a/`.
fn module_dirs(file: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = file.parent()?;
    let stem = file.file_stem()?.to_str()?;
    match matches!(stem, "mod" | "lib" | "main") {
        true => Some((dir.to_path_buf(), dir.parent()?.to_path_buf())),
        false => Some((dir.join(stem), dir.to_path_buf())),
    }
}

/// `./x`, `../y/z` — a path relative to the importing file.
///
/// A bare specifier (`react`, `lucide-react`) is deliberately not resolved: it
/// lives in a package directory that is not the reader's code, and following it
/// lands in `node_modules` rather than anywhere useful.
fn ts(dir: &Path, module: &str, cx: &Cx) -> Option<PathBuf> {
    let (exists, aliases) = (cx.exists, cx.aliases);
    if !module.starts_with('.') {
        // A workspace package is the project's own code under another name, so
        // it is tried before giving up on a bare specifier.
        if let Some(p) = cx.workspace.resolve(module, cx.files).and_then(|p| probe(&p, exists)) {
            return Some(p);
        }
        // `@/components/ui/button` is not a package — it is the project's own
        // code under an alias declared in `tsconfig.json`. Without this, an
        // application's imports are almost all dead links, because that is how
        // application code is written (SPEC.md §Code).
        return aliases
            .candidates(module)
            .into_iter()
            .find_map(|c| probe(&c, exists));
    }
    let base = normalise(dir, module);
    probe(&base, exists)
}

/// A path without an extension, tried the way a bundler tries it: as a file
/// with each known extension, then as a directory with an index.
///
/// The extension is **appended**, never substituted. `@/payload.config` has a
/// `.config` that `with_extension` would replace, turning it into `payload.ts`
/// and finding nothing; and `types/storyblok` is really `storyblok.d.ts`, which
/// only appending reaches. Substitution is tried afterwards and only for a
/// genuine module extension, which is the `./x.js` that means `x.ts` case.
fn probe(base: &Path, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if exists(base) {
        return Some(base.to_path_buf());
    }
    let name = base.file_name()?.to_str()?.to_string();
    let dir = base.parent().unwrap_or(Path::new("."));
    for ext in TS_EXTS {
        let p = dir.join(format!("{name}.{ext}"));
        if exists(&p) {
            return Some(p);
        }
    }
    // `import './x.js'` under a TypeScript build really names `x.ts`.
    if matches!(
        base.extension().and_then(|e| e.to_str()),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs")
    ) {
        for ext in ["ts", "tsx"] {
            let p = base.with_extension(ext);
            if exists(&p) {
                return Some(p);
            }
        }
    }
    for ext in TS_EXTS {
        let p = base.join(format!("index.{ext}"));
        if exists(&p) {
            return Some(p);
        }
    }
    None
}

/// `crate::a::b`, `super::a`, `self::a` — a module path.
///
/// A module is either `a/b.rs` or `a/b/mod.rs`, and both are tried. An external
/// crate (`std::fs`, `serde::Deserialize`) resolves to nothing: its source is
/// not in this tree.
fn rust(file: &Path, root: &Path, module: &str, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let (own, parent) = module_dirs(file)?;
    let mut parts: Vec<&str> = module
        .split("::")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    // A braced list (`use a::b::{c, d}`) names the module up to the brace.
    if let Some(i) = parts.iter().position(|p| p.starts_with('{')) {
        parts.truncate(i);
    }
    let mut base = match parts.first().copied() {
        Some("crate") => {
            parts.remove(0);
            root.to_path_buf()
        }
        Some("self") => {
            parts.remove(0);
            own
        }
        Some("super") => {
            parts.remove(0);
            let mut d = parent;
            // Each further `super` is one module up from there.
            while parts.first() == Some(&"super") {
                parts.remove(0);
                d = d.parent()?.to_path_buf();
            }
            d
        }
        _ => return None, // an external crate, or a bare name
    };
    // The tail is usually a type or function, not a module, so try the longest
    // path first and give up a segment at a time.
    while !parts.is_empty() {
        let candidate = parts.iter().fold(base.clone(), |p, s| p.join(s));
        let rs = candidate.with_extension("rs");
        if exists(&rs) {
            return Some(rs);
        }
        let m = candidate.join("mod.rs");
        if exists(&m) {
            return Some(m);
        }
        parts.pop();
    }
    base = base.join("mod.rs");
    exists(&base).then_some(base)
}

/// Join `module` onto `dir`, resolving `.` and `..` without touching the disk.
fn normalise(dir: &Path, module: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for part in module.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    out
}

/// The lines one import statement really occupies, starting at `from`.
///
/// The declaration walker stops a signature at the line that opens a brace,
/// which for `import {` is the first line of five. Everything about an import —
/// the module it names and the bindings it introduces — is spread across the
/// rest, so the statement has to be re-measured here.
pub fn statement_span(raw: &[&str], from: usize, lang: &str) -> (usize, usize) {
    // A statement is short; a runaway scan on a file with no terminator is not
    // worth the risk.
    const MAX: usize = 40;
    let end_marker = |l: &str| match lang {
        "rust" => l.contains(';'),
        _ => l.contains(';') || l.contains(" from "),
    };
    let last = (from + MAX).min(raw.len());
    for (i, line) in raw[from..last].iter().enumerate() {
        if end_marker(line) {
            return (from, from + i + 1);
        }
    }
    (from, (from + 1).min(raw.len()))
}

/// The module an import statement names, read from the whole statement.
pub fn module_of(lang: &str, stmt: &[&str]) -> Option<String> {
    let joined = stmt.join(" ");
    match lang {
        "rust" => {
            let after = joined.split_once("use ")?.1;
            Some(after.trim().trim_end_matches(';').trim().to_string())
        }
        _ => {
            // The module is the quoted string, which for a multi-line import is
            // on the closing line rather than the one with the keyword.
            let after = match joined.rfind(" from ") {
                Some(i) => &joined[i + 6..],
                None => joined.split_once("import")?.1,
            };
            let mut it = after.chars().skip_while(|c| *c != '\'' && *c != '"');
            let quote = it.next()?;
            let s: String = it.take_while(|c| *c != quote).collect();
            (!s.is_empty()).then_some(s)
        }
    }
}

/// A name an import brings in, and where it sits: `(line within the statement,
/// byte range in that line, the name as the *target* spells it)`.
pub type Named = (usize, usize, usize, String);

/// The names an import statement introduces.
///
/// Each becomes its own link, so `n` steps between them and `Enter` lands on
/// that declaration in the target rather than at the top of the file. `X as Y`
/// yields `X`: the target spells it that way, and the anchor has to match what
/// is declared there, not what this file calls it.
///
/// `stmt` is the statement's raw lines — an import is routinely spread over
/// several, and the names then live on lines the keyword is not on.
pub fn imported_names(lang: &str, stmt: &[&str]) -> Vec<Named> {
    let mut out = Vec::new();
    let braced = stmt.iter().any(|l| l.contains('{'));
    for (i, line) in stmt.iter().enumerate() {
        let region = match (lang, braced) {
            // Inside a braced list every identifier is an imported name.
            (_, true) => brace_region(line),
            // `use a::b::Name;` — only the final segment names a symbol.
            ("rust", false) => last_segment(line),
            // `import Name from './x'` — the default binding.
            ("typescript", false) => default_binding(line),
            _ => None,
        };
        let Some((from, to)) = region else { continue };
        idents_in(&line[from..to], from, i, &mut out);
    }
    out
}

/// The part of this line inside `{ … }`, or the whole line when the braces are
/// on other lines of the statement.
fn brace_region(line: &str) -> Option<(usize, usize)> {
    let from = line.find('{').map(|i| i + 1).unwrap_or(0);
    let to = line.find('}').unwrap_or(line.len());
    (from <= to).then_some((from, to))
}

/// `use a::b::Name;` -> the range covering `Name`.
fn last_segment(line: &str) -> Option<(usize, usize)> {
    let body = line.trim_end().trim_end_matches(';');
    let at = body.rfind("::")? + 2;
    (at < body.len()).then_some((at, body.len()))
}

/// `import Name from './x'` -> the range covering `Name`. A namespace import
/// (`* as ns`) names nothing in the target.
fn default_binding(line: &str) -> Option<(usize, usize)> {
    let start = line.find("import")? + 6;
    let end = line.find(" from ")?;
    (start < end && !line[start..end].contains('*')).then_some((start, end))
}

/// Identifiers in `text`, recorded at their real offsets.
fn idents_in(text: &str, offset: usize, line: usize, out: &mut Vec<Named>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut skip_next = false;
    while i < bytes.len() {
        if !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
            i += 1;
            continue;
        }
        let from = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
        {
            i += 1;
        }
        let word = &text[from..i];
        // `X as Y` — keep `X`, drop the local alias that follows.
        if word == "as" {
            skip_next = true;
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        // `type` in `import { type A }`, and the keywords a `use` line carries.
        if matches!(word, "type" | "crate" | "super" | "self" | "pub" | "use" | "import" | "from") {
            continue;
        }
        out.push((line, offset + from, offset + i, word.to_string()));
    }
}

/// The crate root for a Rust file: the nearest ancestor `src` directory.
///
/// `crate::` is relative to it, and it is the only part of a Rust path that
/// cannot be worked out from the module path alone.
pub fn crate_root(file: &Path) -> Option<PathBuf> {
    let mut at = file.parent()?;
    loop {
        if at.file_name().map(|n| n == "src").unwrap_or(false) {
            return Some(at.to_path_buf());
        }
        at = at.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context for the path tests: no workspace, since they are about paths
    /// rather than packages.
    fn cx<'a>(
        root: Option<&'a Path>,
        exists: &'a dyn Fn(&Path) -> bool,
        aliases: &'a Aliases,
    ) -> Cx<'a> {
        Cx { root, exists, aliases, workspace: WS.get_or_init(Workspace::none), files: &NoFiles }
    }

    static WS: std::sync::OnceLock<Workspace> = std::sync::OnceLock::new();

    struct NoFiles;

    impl super::super::workspace::Files for NoFiles {
        fn read(&self, _p: &Path) -> Option<String> {
            None
        }
        fn list(&self, _d: &Path) -> Vec<String> {
            Vec::new()
        }
    }

    /// A fake tree: exactly these paths exist.
    /// Compares `Path` values rather than their spelling: `join` uses the
    /// platform separator, so a string comparison passes on unix and fails on
    /// Windows for the same tree.
    fn tree(paths: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| paths.iter().any(|k| Path::new(k) == p)
    }

    #[test]
    fn a_relative_import_finds_the_file_whatever_its_extension() {
        let e = tree(&["/app/components/field-preview.tsx", "/app/lib/util.ts"]);
        let dir = Path::new("/app/components/form.tsx");
        assert_eq!(
            resolve("typescript", dir, "./field-preview", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/app/components/field-preview.tsx"))
        );
        assert_eq!(
            resolve("typescript", dir, "../lib/util", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/app/lib/util.ts"))
        );
    }

    /// Both found by measuring a real project, and both wrong the same way:
    /// the extension must be *appended*, not substituted.
    #[test]
    fn an_extension_is_appended_rather_than_replacing_what_is_there() {
        // `@/payload.config` — `with_extension` would make this `payload.ts`.
        let e = tree(&["/p/src/payload.config.ts"]);
        assert_eq!(
            resolve("typescript", Path::new("/p/src/a.ts"), "./payload.config", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/payload.config.ts"))
        );
        // A declaration file: `storyblok` is really `storyblok.d.ts`.
        let e = tree(&["/p/types/storyblok.d.ts"]);
        assert_eq!(
            resolve("typescript", Path::new("/p/a.ts"), "./types/storyblok", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/p/types/storyblok.d.ts"))
        );
    }

    /// `import './x.js'` in a TypeScript project names `x.ts` — the one case
    /// where substituting *is* right.
    #[test]
    fn a_js_specifier_may_name_a_ts_file() {
        let e = tree(&["/p/util.ts"]);
        assert_eq!(
            resolve("typescript", Path::new("/p/a.ts"), "./util.js", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/p/util.ts"))
        );
    }

    #[test]
    fn an_aliased_import_resolves_through_the_config() {
        let e = tree(&["/app/components/ui/button.tsx"]);
        let read = |p: &Path| match p == Path::new("/app/tsconfig.json") {
            true => Some(r#"{"compilerOptions":{"paths":{"@/*":["./*"]}}}"#.to_string()),
            false => None,
        };
        let file = Path::new("/app/features/form.tsx");
        let aliases = Aliases::load(file, &read);
        assert_eq!(
            resolve("typescript", file, "@/components/ui/button", &cx(None, &e, &aliases)),
            Some(PathBuf::from("/app/components/ui/button.tsx"))
        );
    }

    #[test]
    fn a_directory_import_finds_its_index() {
        let e = tree(&["/app/ui/index.ts"]);
        assert_eq!(
            resolve("typescript", Path::new("/app/a.ts"), "./ui", &cx(None, &e, &Aliases::none())),
            Some(PathBuf::from("/app/ui/index.ts"))
        );
    }

    /// Following `react` would land in `node_modules`, which is not the
    /// reader's code.
    #[test]
    fn a_bare_specifier_is_not_followed() {
        let e = tree(&["/app/node_modules/react/index.js"]);
        assert_eq!(resolve("typescript", Path::new("/app/a.ts"), "react", &cx(None, &e, &Aliases::none())), None);
        assert_eq!(resolve("typescript", Path::new("/app/a.ts"), "@/ui/button", &cx(None, &e, &Aliases::none())), None);
    }

    #[test]
    fn a_crate_path_resolves_against_the_src_root() {
        let e = tree(&["/p/src/source/dir/mod.rs", "/p/src/theme.rs"]);
        let root = Path::new("/p/src");
        let dir = Path::new("/p/src/pager/mod.rs");
        // The tail names a type, not a module: the longest prefix that is a
        // file wins.
        assert_eq!(
            resolve("rust", dir, "crate::source::dir::DirSource", &cx(Some(root), &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/source/dir/mod.rs"))
        );
        assert_eq!(
            resolve("rust", dir, "crate::theme", &cx(Some(root), &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/theme.rs"))
        );
    }

    #[test]
    fn super_and_self_resolve_against_the_file() {
        let e = tree(&["/p/src/source/collapse.rs", "/p/src/source/code/render.rs"]);
        let dir = Path::new("/p/src/source/code/mod.rs");
        assert_eq!(
            resolve("rust", dir, "super::collapse", &cx(Some(Path::new("/p/src")), &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/source/collapse.rs"))
        );
        assert_eq!(
            resolve("rust", dir, "self::render", &cx(Some(Path::new("/p/src")), &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/source/code/render.rs"))
        );
    }

    #[test]
    fn an_external_crate_resolves_to_nothing() {
        let e = tree(&["/p/src/lib.rs"]);
        let d = Path::new("/p/src/lib.rs");
        let r = Path::new("/p/src");
        assert_eq!(resolve("rust", d, "std::fs", &cx(Some(r), &e, &Aliases::none())), None);
        assert_eq!(resolve("rust", d, "serde::Deserialize", &cx(Some(r), &e, &Aliases::none())), None);
    }

    #[test]
    fn a_braced_list_names_the_module_before_the_brace() {
        let e = tree(&["/p/src/csv/parse.rs"]);
        // From `csv/delim.rs`, `super` is the `csv` module itself.
        let d = Path::new("/p/src/csv/delim.rs");
        assert_eq!(
            resolve("rust", d, "super::parse::{Records, QUOTE}", &cx(Some(Path::new("/p/src")), &e, &Aliases::none())),
            Some(PathBuf::from("/p/src/csv/parse.rs"))
        );
    }

    fn names(lang: &str, stmt: &[&str]) -> Vec<String> {
        imported_names(lang, stmt).into_iter().map(|(_, _, _, n)| n).collect()
    }

    #[test]
    fn a_use_line_names_the_symbol_it_ends_with() {
        assert_eq!(names("rust", &["use super::parse::Records;"]), vec!["Records"]);
        assert_eq!(
            names("rust", &["use super::parse::{Records, QUOTE};"]),
            vec!["Records", "QUOTE"]
        );
        // `X as Y` — the anchor must be what the *target* declares.
        assert_eq!(names("rust", &["use a::b::Thing as Other;"]), vec!["Thing"]);
        assert!(names("rust", &["use a::b::*;"]).is_empty());
    }

    #[test]
    fn an_import_names_each_binding_it_brings_in() {
        assert_eq!(
            names("typescript", &["import { A, B } from './x';"]),
            vec!["A", "B"]
        );
        assert_eq!(names("typescript", &["import Foo from './x';"]), vec!["Foo"]);
        assert_eq!(
            names("typescript", &["import { A as C } from './x';"]),
            vec!["A"]
        );
        assert_eq!(
            names("typescript", &["import { type T, v } from './x';"]),
            vec!["T", "v"],
            "`type` is a modifier, not a name"
        );
        assert!(names("typescript", &["import * as ns from './x';"]).is_empty());
        assert!(names("typescript", &["import './x.css';"]).is_empty());
    }

    /// Imports are routinely spread over several lines, and then the names are
    /// not on the line the keyword is on.
    #[test]
    fn a_multi_line_import_names_the_bindings_on_their_own_lines() {
        let stmt = ["import {", "  Select,", "  SelectContent,", "} from \"@/ui\";"];
        let got = imported_names("typescript", &stmt);
        assert_eq!(
            got.iter().map(|(_, _, _, n)| n.as_str()).collect::<Vec<_>>(),
            vec!["Select", "SelectContent"]
        );
        // ...and each is located on its own line, at its own column.
        assert_eq!(got[0].0, 1, "Select is on the second line of the statement");
        assert_eq!(&stmt[1][got[0].1..got[0].2], "Select");
        assert_eq!(got[1].0, 2);
        assert_eq!(&stmt[2][got[1].1..got[1].2], "SelectContent");
    }

    #[test]
    fn the_crate_root_is_the_nearest_src_directory() {
        assert_eq!(
            crate_root(Path::new("/p/src/source/code/mod.rs")),
            Some(PathBuf::from("/p/src"))
        );
        assert_eq!(crate_root(Path::new("/p/tools/x.rs")), None);
    }
}
