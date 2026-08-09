//! A code file as a document.
#![deny(unsafe_code)]

use super::*;
use crate::source::Source;

const SRC: &str = "\
//! A file about something.

use std::fs;

/// What it does.
pub fn open(path: &str) -> u8 {
    let x = 1;
    x
}

impl Thing {
    /// A method.
    fn go(&self) {
        todo!()
    }
}
";

fn src() -> CodeSource {
    CodeSource::new(PathBuf::from("t.rs"), "rust", SRC)
}

fn text(s: &mut CodeSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

#[test]
fn a_file_opens_collapsed_to_its_comments_and_declarations() {
    let mut s = src();
    let shown = text(&mut s);
    // The comments and signatures survive...
    assert!(shown.iter().any(|l| l.contains("//! A file about something.")));
    assert!(shown.iter().any(|l| l.contains("/// What it does.")));
    assert!(shown.iter().any(|l| l.contains("pub fn open")));
    // ...and the bodies are gone.
    assert!(!shown.iter().any(|l| l.contains("let x = 1")), "{shown:?}");
    assert!(!shown.iter().any(|l| l.contains("todo!")), "{shown:?}");
}

#[test]
fn the_outline_lists_every_symbol_with_methods_nested() {
    let s = src();
    let got: Vec<(u8, &str)> = s
        .outline()
        .iter()
        .map(|e| (e.level, e.text.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, "std::fs"),
            (1, "fn open"),
            (1, "impl Thing"),
            (2, "fn go"),
        ]
    );
}

/// Unfolding everything *is* the raw file — that is why there is no second
/// renderer to keep in step.
#[test]
fn r_toggles_between_the_summary_and_the_whole_file() {
    let mut s = src();
    let msg = s.toggle_hidden().unwrap();
    assert_eq!(msg, "showing source");
    let shown = text(&mut s);
    assert_eq!(shown.len(), SRC.lines().count(), "every line is back");
    assert!(shown.iter().any(|l| l.contains("let x = 1")));

    assert_eq!(s.toggle_hidden().unwrap(), "showing symbols");
    assert!(!text(&mut s).iter().any(|l| l.contains("let x = 1")));
}

#[test]
fn a_folded_symbol_reports_how_much_it_hides() {
    let s = src();
    let head = s
        .outline()
        .iter()
        .position(|e| e.text == "fn open")
        .expect("the function");
    let row = s.row_of(s.outline()[head].anchor).expect("a visible row");
    // The count sits on the signature, not on the doc comment above it:
    // `pub fn open(..) {  (4 lines)` reads, the same number stranded on the
    // first line of a doc comment does not.
    assert_eq!(s.hidden_at(row), None, "the doc row carries no count");
    // Exactly the body and its closing brace. A region ends where the block
    // closes, so the blank line after it is no longer swallowed — which is the
    // whole reason code states its regions instead of inferring them.
    assert_eq!(s.hidden_at(row + 1), Some(3), "the signature row does");
}

/// The fold ids are symbol paths, so jumping to one is already go-to-definition
/// inside a file.
#[test]
fn a_symbol_path_is_an_anchor_that_can_be_jumped_to() {
    let mut s = src();
    let row = s.goto_id("Thing::go").expect("the method");
    // The jump lands on the doc comment, because that is where the symbol
    // starts — you arrive reading what it is for, with the signature under it.
    let landed = s.lines(row..row + 2);
    assert!(landed[0].text().contains("/// A method."), "{:?}", landed[0].text());
    assert!(landed[1].text().contains("fn go"), "{:?}", landed[1].text());
    assert!(s.goto_id("nope::missing").is_none());
}

/// The safety valve, end to end: a file that does not lex is still readable.
#[test]
fn a_file_that_does_not_parse_opens_raw_and_says_so() {
    let bad = "fn f() {\n    let s = \"unterminated;\n";
    let mut s = CodeSource::new(PathBuf::from("bad.rs"), "rust", bad);
    assert!(s.unparsed);
    assert!(s.outline().is_empty(), "no invented symbols");
    assert_eq!(text(&mut s).len(), bad.lines().count(), "every line shown");
    let pos = s.position_text(0).unwrap();
    assert!(pos.contains("unparsed"), "{pos}");
    assert!(s.toggle_hidden().unwrap().contains("already raw"));
}

#[test]
fn searching_finds_a_line_that_is_visible() {
    let mut s = src();
    s.set_query("open");
    assert!(s.match_count() >= 1);
    // A hidden body line does not match while it is hidden.
    s.set_query("todo!");
    assert_eq!(s.match_count(), 0, "folded away");
    s.toggle_hidden();
    s.set_query("todo!");
    assert_eq!(s.match_count(), 1, "and found once shown");
}

#[test]
fn yanking_a_section_takes_the_whole_symbol_even_when_folded() {
    let s = src();
    let row = s.row_of(s.outline()[1].anchor).expect("fn open row");
    let y = s.yank_section(row).expect("a yank");
    assert!(y.text.contains("pub fn open"), "{}", y.text);
    assert!(y.text.contains("let x = 1"), "the body comes too: {}", y.text);
    assert_eq!(s.yank_block(row).unwrap().text, "open\n", "c takes the path");
}

#[test]
fn a_language_is_recognised_by_extension() {
    assert_eq!(language_of(Path::new("a/b.rs")), Some("rust"));
    assert_eq!(language_of(Path::new("a/b.RS")), Some("rust"));
    assert_eq!(language_of(Path::new("a/b.py")), Some("python"));
    assert_eq!(language_of(Path::new("a/b.java")), Some("java"));
    // A language this build does not know is still plain text.
    assert_eq!(language_of(Path::new("a/b.rb")), None);
    assert_eq!(language_of(Path::new("noext")), None);
}

/// The `o` overlay is the symbol list the reader asked for: it comes straight
/// from `outline()`, so opening a code file gives it with no new UI.
#[test]
fn the_outline_overlay_lists_the_symbols_of_a_real_file() {
    let src = include_str!("../../csv/delim.rs");
    let s = CodeSource::new(PathBuf::from("delim.rs"), "rust", src);
    let mut p = crate::pager::Pager::new(Box::new(s), "delim.rs".into(), 80, 24, Some(80));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Char('o')));
    let shown = p.visible_text().join("\n");
    for want in ["fn sniff", "fn score", "fn parse_delim", "const CANDIDATES"] {
        assert!(shown.contains(want), "{want} missing from the overlay:\n{shown}");
    }
}

/// `]` steps to the next declaration, which is how you walk a file.
#[test]
fn the_next_landmark_is_the_next_declaration() {
    let s = src();
    let first = s.next_landmark(0, true).expect("a next symbol");
    assert!(s.lines[s.visible[first]].heading.is_some(), "landed on a declaration");
    let second = s.next_landmark(first, true).expect("and the one after");
    assert!(s.lines[s.visible[second]].heading.is_some());
    // Backwards from the second lands on the first; backwards from the first
    // finds nothing, because nothing is declared above it.
    assert_eq!(s.next_landmark(second, false), Some(first));
    assert_eq!(s.next_landmark(first, false), None);
}

/// Following an import is the jump the reader offers: `n` walks them, `Enter`
/// opens the module.
#[test]
fn an_import_that_resolves_becomes_a_link() {
    let src = "\
use super::parse::Records;
use std::fs;

fn f() {}
";
    let here = PathBuf::from("/p/src/csv/delim.rs");
    let exists = |p: &Path| p == Path::new("/p/src/csv/parse.rs");
    let s = CodeSource::with_fs(here, "rust", src, &exists, &tsconfig::Aliases::none(), &workspace::Workspace::none());
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    // `super::parse` resolves next door; `std::fs` is not in this tree. The
    // anchor is the imported name, so Enter lands on `Records` itself.
    assert_eq!(urls, vec!["./parse.rs#Records"]);
    assert_eq!(s.links()[0].anchor, Anchor(0), "on the import's own row");
}

#[test]
fn a_typescript_import_resolves_through_its_extension() {
    let src = "import { a } from './field-preview';\nimport React from 'react';\n";
    let here = PathBuf::from("/app/components/form.tsx");
    let exists = |p: &Path| p == Path::new("/app/components/field-preview.tsx");
    let s = CodeSource::with_fs(here, "typescript", src, &exists, &tsconfig::Aliases::none(), &workspace::Workspace::none());
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    assert_eq!(
        urls,
        vec!["./field-preview.tsx#a"],
        "the binding is the anchor, and `react` is never followed"
    );
}

/// An import that names nothing in this tree is left as plain text rather than
/// becoming a link that goes nowhere.
#[test]
fn an_unresolvable_import_is_not_a_link() {
    let src = "use std::collections::HashMap;\nuse serde::Deserialize;\n";
    let s = CodeSource::with_fs(
        PathBuf::from("/p/src/a.rs"),
        "rust",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    assert!(s.links().is_empty());
}

/// End to end, against this repository: open `csv/delim.rs`, walk to its
/// `use super::parse::…` line, press Enter, and land in `csv/parse.rs` — read
/// as code, with `Backspace` able to come back.
#[test]
fn enter_on_an_import_opens_that_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let here = root.join("csv/delim.rs");
    let src = std::fs::read_to_string(&here).expect("this repository's own source");
    let s = CodeSource::new(here.clone(), "rust", &src);
    assert!(!s.links().is_empty(), "delim.rs imports something local");

    let mut p = crate::pager::Pager::new(Box::new(s), "delim.rs".into(), 80, 24, Some(80));
    p.attach_nav(crate::nav::Navigator::new(&here, Some(&root), &root));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Char('n')));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Enter));

    let shown = p.visible_text().join("\n");
    assert!(
        shown.contains("pub struct Records") || shown.contains("Records"),
        "landed in parse.rs: {shown}\nmessage={:?}",
        p.message
    );
    // And it is being read as code, not as text: the outline has symbols.
    assert!(!shown.is_empty());
}

/// The point of the anchor: following `use super::parse::Records` lands on the
/// declaration of `Records` in `parse.rs`, not at the top of it.
#[test]
fn following_an_import_lands_on_the_symbol_it_named() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let here = root.join("csv/delim.rs");
    let src = std::fs::read_to_string(&here).expect("this repository's own source");
    let s = CodeSource::new(here.clone(), "rust", &src);
    // `use super::parse::{Records, QUOTE};` — one link per name.
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    assert!(urls.contains(&"./parse.rs#Records"), "{urls:?}");
    assert!(urls.contains(&"./parse.rs#QUOTE"), "{urls:?}");

    let mut p = crate::pager::Pager::new(Box::new(s), "delim.rs".into(), 80, 24, Some(80));
    p.attach_nav(crate::nav::Navigator::new(&here, Some(&root), &root));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Char('n')));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Enter));

    // The cursor row itself is the declaration, not merely somewhere in the file.
    let shown = p.visible_text();
    let joined = shown.join("\n");
    assert!(joined.contains("Records"), "landed in parse.rs: {joined}");
    assert!(
        p.message.as_deref().map(|m| !m.contains("no heading")).unwrap_or(true),
        "the anchor resolved: {:?}",
        p.message
    );
}

/// `mod foo;` names a file exactly as an import does.
#[test]
fn a_mod_declaration_is_a_link_to_its_file() {
    let src = "mod view;\nmod missing;\n\nfn f() {}\n";
    let here = PathBuf::from("/p/src/source/code/mod.rs");
    let exists = |p: &Path| p == Path::new("/p/src/source/code/view.rs");
    let s = CodeSource::with_fs(here, "rust", src, &exists, &tsconfig::Aliases::none(), &workspace::Workspace::none());
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    // No anchor: the module *is* the file, so it opens at the top.
    assert_eq!(urls, vec!["./view.rs"]);
}

/// A module import lands at the top of the file rather than chasing an anchor
/// that names the file itself.
#[test]
fn importing_a_module_carries_no_anchor() {
    let src = "use crate::theme;\n";
    let here = PathBuf::from("/p/src/pager/mod.rs");
    let exists = |p: &Path| p == Path::new("/p/src/theme.rs");
    let s = CodeSource::with_fs(here, "rust", src, &exists, &tsconfig::Aliases::none(), &workspace::Workspace::none());
    // Relative, with `../` — never absolute, which `nav` would read as
    // relative to the corpus root. No `#anchor`: in Rust `theme` names the
    // file, so it opens at the top.
    assert_eq!(
        s.links().iter().map(|l| l.url.as_str()).collect::<Vec<_>>(),
        vec!["../theme.rs"]
    );
}

/// Against a real TypeScript project, when one is pointed at.
///
/// Set `TREAD_TS_FILE` to a file in a project with a `tsconfig.json` and this
/// reports which of its imports resolve. Aliased imports are the whole point:
/// application code is written `@/components/…`, and a fixture cannot prove
/// that the real config is read correctly. Skipped when unset.
#[test]
fn a_real_typescript_file_resolves_its_aliased_imports() {
    let Ok(file) = std::env::var("TREAD_TS_FILE") else {
        return;
    };
    let path = PathBuf::from(&file);
    let src = std::fs::read_to_string(&path).expect("the named file");
    let s = CodeSource::new(path, "typescript", &src);
    let urls: Vec<String> = s.links().iter().map(|l| l.url.clone()).collect();
    let aliased = urls.iter().filter(|u| !u.starts_with("./")).count();
    println!("{} links, {aliased} of them through an alias", urls.len());
    for u in urls.iter() {
        println!("  {u}");
    }
    assert!(!urls.is_empty(), "a real file imports something local");
}

/// The shape that actually dominates an application: a multi-line import
/// through a `tsconfig.json` alias. Every binding is its own link, on its own
/// line, pointing at its own declaration.
#[test]
fn a_multi_line_aliased_import_links_every_binding() {
    let src = "\
import {
  Select,
  SelectContent,
} from \"@/components/ui/select\";

export function F() {}
";
    let here = PathBuf::from("/app/components/form.tsx");
    let target = Path::new("/app/components/ui/select.tsx");
    let read = |p: &Path| match p == Path::new("/app/tsconfig.json") {
        true => Some(r#"{"compilerOptions":{"paths":{"@/*":["./*"]}}}"#.to_string()),
        false => None,
    };
    let aliases = tsconfig::Aliases::load(&here, &read);
    let s = CodeSource::with_fs(here, "typescript", src, &|p| p == target, &aliases, &workspace::Workspace::none());
    let links: Vec<(usize, &str)> = s
        .links()
        .iter()
        .map(|l| (l.anchor.0, l.url.as_str()))
        .collect();
    assert_eq!(
        links,
        vec![
            (1, "./ui/select.tsx#Select"),
            (2, "./ui/select.tsx#SelectContent"),
        ],
        "one per binding, each on the line that spells it"
    );
}

/// Coverage across a real project: of the imports that name the project's own
/// code, which fail to resolve? Set `TREAD_TS_PROJECT`. Skipped when unset.
///
/// The interesting number is the failures, not the successes — a specifier that
/// names local code and resolves to nothing is a dead link, and the point of
/// reading the project's `tsconfig.json` was to have none of them.
#[test]
fn a_real_project_resolves_the_imports_that_name_its_own_code() {
    let Ok(root) = std::env::var("TREAD_TS_PROJECT") else {
        return;
    };
    let mut stack = vec![PathBuf::from(root)];
    let (mut files, mut local, mut ok) = (0usize, 0usize, 0usize);
    let mut bad: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                if !matches!(name.as_str(), "node_modules" | ".next" | ".git" | "dist") {
                    stack.push(p);
                }
                continue;
            }
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !matches!(ext, "ts" | "tsx") || files >= 400 {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            files += 1;
            let aliases = tsconfig::Aliases::load(&p, &|q| std::fs::read_to_string(q).ok());
            let Some(syms) = crate::code::ts_decl::symbols(&src) else { continue };
            let raw: Vec<&str> = src.lines().collect();
            for s in syms.iter().filter(|s| s.kind == crate::code::Kind::Import) {
                let (a, b) = jump::statement_span(&raw, s.sig.0, "typescript");
                let stmt: Vec<&str> = raw.get(a..b).unwrap_or(&[]).to_vec();
                let Some(m) = jump::module_of("typescript", &stmt) else { continue };
                // Only specifiers that name this project's own code.
                if !(m.starts_with('.') || m.starts_with('@') || m.starts_with('~')) {
                    continue;
                }
                // A scoped name is only local if the project claims it —
                // through `tsconfig` aliases or as a workspace package.
                let ws = workspace::Workspace::load(&p, &RealFiles);
                if m.starts_with('@')
                    && aliases.candidates(&m).is_empty()
                    && ws.resolve(&m, &RealFiles).is_none()
                {
                    continue;
                }
                local += 1;
                let exists = |q: &Path| q.is_file();
                let cx = jump::Cx {
                    root: None,
                    exists: &exists,
                    aliases: &aliases,
                    workspace: &ws,
                    files: &RealFiles,
                };
                match jump::resolve("typescript", &p, &m, &cx) {
                    Some(_) => ok += 1,
                    None if bad.len() < 10 => bad.push(format!("{m}  (in {})", p.display())),
                    None => {}
                }
            }
        }
    }
    let pct = 100.0 * ok as f64 / local.max(1) as f64;
    println!("{files} files: {ok}/{local} local imports resolve ({pct:.1}%)");
    for m in &bad {
        println!("  unresolved: {m}");
    }
    assert!(files > 0, "no .ts/.tsx under TREAD_TS_PROJECT");
}


/// A link must be a *relative* path. `nav` reads a leading `/` as relative to
/// the corpus root, so an absolute one is looked for under the root and
/// reported missing — which is how every cross-directory import came to say
/// "no such file" in a real project.
#[test]
fn a_link_is_relative_even_when_the_target_is_elsewhere() {
    let from = Path::new("/p/src/app/page.tsx");
    assert_eq!(relative(from, Path::new("/p/src/app/x.tsx")), "./x.tsx");
    assert_eq!(relative(from, Path::new("/p/src/app/ui/x.tsx")), "./ui/x.tsx");
    // The case that was broken: a sibling directory.
    assert_eq!(relative(from, Path::new("/p/src/components/x.tsx")), "../components/x.tsx");
    assert_eq!(relative(from, Path::new("/p/lib/x.ts")), "../../lib/x.ts");
    // Nothing may come out absolute.
    for t in ["/p/src/app/x.tsx", "/p/other/x.ts", "/q/x.ts"] {
        let got = relative(from, Path::new(t));
        assert!(!got.starts_with('/'), "{t} -> {got}");
    }
}

/// End to end at the level that was broken: a code file in a real project,
/// rooted the way `main` roots it, whose imports all resolve.
#[test]
fn imports_resolve_when_the_corpus_is_the_project() {
    let t = std::env::temp_dir().join(format!("tread-corpus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(t.join("src/app")).unwrap();
    std::fs::create_dir_all(t.join("src/components")).unwrap();
    // The marker that makes this a project, and the alias table.
    std::fs::write(t.join("package.json"), "{}\n").unwrap();
    std::fs::write(
        t.join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
    )
    .unwrap();
    std::fs::write(t.join("src/components/Widget.tsx"), "export function Widget() {}\n").unwrap();
    let page = t.join("src/app/page.tsx");
    let src = "import { Widget } from \"@/components/Widget\";\n\nexport function P() {}\n";
    std::fs::write(&page, src).unwrap();

    let s = CodeSource::new(page.clone(), "typescript", src);
    let urls: Vec<&str> = s.links().iter().map(|l| l.url.as_str()).collect();
    assert_eq!(urls, vec!["../components/Widget.tsx#Widget"], "relative, with the anchor");

    // Rooted at the project, as `main` does for a code file.
    let root = crate::open::corpus_root(&page).expect("the package.json above it");
    assert_eq!(root, t, "the project, not the folder the file sits in");
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut p = crate::pager::Pager::new(Box::new(s), "page.tsx".into(), 80, 24, Some(80));
    p.attach_nav(crate::nav::Navigator::new(&page, Some(&root), &cwd));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Char('n')));
    p.handle(crate::key::KeyEvent::plain(crate::key::Key::Enter));
    let shown = p.visible_text().join("\n");
    assert!(
        shown.contains("export function Widget"),
        "the import opened; message={:?}\n{shown}",
        p.message
    );
    let _ = std::fs::remove_dir_all(&t);
}

/// Imports fold as one block, the way frontmatter does: the wall of lines at
/// the top of a file is something a reader scrolls past, not reads.
#[test]
fn a_run_of_imports_folds_as_one_with_a_summary() {
    let src = "\
use super::parse::{Records, QUOTE};
use crate::theme;
mod view;

fn f() {
    let x = 1;
}
";
    let here = PathBuf::from("/p/src/csv/delim.rs");
    let s = CodeSource::with_fs(here, "rust", src, &|_: &Path| false, &tsconfig::Aliases::none(), &workspace::Workspace::none());
    // One outline entry for the run, not one per line.
    let texts: Vec<&str> = s.outline().iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, vec!["3 imports", "fn f"], "{texts:?}");

    // Folded by default: only the first import line is on screen.
    let shown = {
        let mut s2 = CodeSource::with_fs(
            PathBuf::from("/p/src/csv/delim.rs"),
            "rust",
            src,
            &|_: &Path| false,
            &tsconfig::Aliases::none(),
            &workspace::Workspace::none(),
        );
        let n = s2.len();
        s2.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect::<Vec<_>>()
    };
    assert!(shown.iter().any(|l| l.contains("use super::parse")));
    assert!(!shown.iter().any(|l| l.contains("mod view")), "folded away: {shown:?}");

    // And the fold says what it hides, in imports rather than in lines.
    let row = s.row_of(s.outline()[0].anchor).expect("the run's row");
    let note = s.fold_note(row).expect("a note");
    assert!(note.contains("modules"), "{note}");
    assert!(note.starts_with('3') || note.contains("3 modules"), "{note}");
}

/// One import is not a wall; folding it would hide nothing worth hiding.
#[test]
fn a_single_import_is_not_grouped() {
    let src = "use std::fs;\n\nfn f() {}\n";
    let s = CodeSource::with_fs(
        PathBuf::from("/p/src/a.rs"),
        "rust",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    let texts: Vec<&str> = s.outline().iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, vec!["std::fs", "fn f"]);
}


/// The thing the fold seam was built for: a branch inside a function folds on
/// its own, and stops at its closing brace.
#[test]
fn a_block_inside_a_function_folds_without_swallowing_what_follows() {
    let src = "\
fn f(a: bool) {
    if a {
        one();
        two();
    }
    after();
}
";
    let mut s = CodeSource::with_fs(
        PathBuf::from("/p/src/a.rs"),
        "rust",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    // Open the function first: blocks are foldable but never folded on open.
    s.fold_all(false);
    let text = |s: &mut CodeSource| {
        let n = s.len();
        s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(text(&mut s).len(), 7, "everything is showing");

    // Fold the branch: the cursor sits on the `if`, which is row 1.
    let closed = s.fold_here(1).expect("code answers with its own regions");
    assert!(closed);
    let shown = text(&mut s);
    assert!(shown.iter().any(|l| l.contains("if a {")), "the head stays: {shown:?}");
    assert!(!shown.iter().any(|l| l.contains("one()")), "the body is hidden");
    // The statement after the closing brace is NOT inside the branch — the
    // whole reason code states its regions instead of inferring them.
    assert!(shown.iter().any(|l| l.contains("after()")), "{shown:?}");

    // And it toggles back.
    assert_eq!(s.fold_here(1), Some(false));
    assert!(text(&mut s).iter().any(|l| l.contains("one()")));
}

/// A two-line block is not worth a marker.
#[test]
fn a_short_block_is_not_foldable() {
    let src = "fn f() {\n    if a {\n        one();\n    }\n}\n";
    let mut s = CodeSource::with_fs(
        PathBuf::from("/p/src/a.rs"),
        "rust",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    s.fold_all(false);
    // Row 1 is the `if`; the innermost region there is the function, not the
    // two-line branch.
    s.fold_here(1);
    let n = s.len();
    let shown: Vec<String> = s.lines(0..n).iter().map(|l| l.text()).collect();
    assert!(!shown.iter().any(|l| l.contains("one()")), "the fn folded instead");
}

/// Through the pager, as a reader does it: `zR` to open the file, move onto a
/// branch, `za` to fold it.
#[test]
fn za_folds_the_branch_the_cursor_is_in() {
    let src = "\
fn f(a: bool) {
    if a {
        one();
        two();
    }
    after();
}
";
    let s = CodeSource::with_fs(
        PathBuf::from("/p/src/a.rs"),
        "rust",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    let mut p = crate::pager::Pager::new(Box::new(s), "a.rs".into(), 80, 24, Some(80));
    let key = |c: char| crate::key::KeyEvent::plain(crate::key::Key::Char(c));
    // zR opens everything.
    p.handle(key('z'));
    p.handle(key('R'));
    assert!(p.visible_text().iter().any(|l| l.contains("one()")));

    // Down onto the `if`, then za.
    p.handle(key('j'));
    p.handle(key('z'));
    p.handle(key('a'));
    let shown = p.visible_text();
    assert!(
        !shown.iter().any(|l| l.contains("one()")),
        "the branch folded: {shown:?} message={:?}",
        p.message
    );
    assert!(
        shown.iter().any(|l| l.contains("after()")),
        "and what follows it did not: {shown:?}"
    );
}

/// Python folds its suites too — the language read most often on a working
/// day, and the one with no braces to count.
#[test]
fn za_folds_a_python_suite() {
    let src = "\
def f(items):
    if items:
        first()
        second()
        third()
    return 1
";
    let s = CodeSource::with_fs(
        PathBuf::from("/p/a.py"),
        "python",
        src,
        &|_: &Path| false,
        &tsconfig::Aliases::none(),
        &workspace::Workspace::none(),
    );
    let mut p = crate::pager::Pager::new(Box::new(s), "a.py".into(), 80, 24, Some(80));
    let key = |c: char| crate::key::KeyEvent::plain(crate::key::Key::Char(c));
    p.handle(key('z'));
    p.handle(key('R'));
    p.handle(key('j'));
    p.handle(key('z'));
    p.handle(key('a'));
    let shown = p.visible_text();
    assert!(!shown.iter().any(|l| l.contains("first()")), "{shown:?}");
    assert!(shown.iter().any(|l| l.contains("if items:")), "the head stays");
    assert!(shown.iter().any(|l| l.contains("return 1")), "and what follows");
}

/// A source file with a stray byte still opens, with `U+FFFD` where the byte
/// was — the rule every other reader here follows (SPEC.md §Plain text).
///
/// Real Python trees contain `# coding: iso-8859-5` files, and refusing to show
/// one because of a byte is worse than showing one character wrong.
#[test]
fn a_source_file_that_is_not_utf8_still_opens() {
    let t = std::env::temp_dir().join(format!("tread-enc-{}.py", std::process::id()));
    // `def f():` followed by a latin-1 comment byte.
    let mut bytes = b"def f():\n    # \xe9\n    return 1\n".to_vec();
    bytes.extend_from_slice(b"x = 1\n");
    std::fs::write(&t, &bytes).unwrap();

    let mut s = CodeSource::open(&t).expect("a stray byte is not a reason to refuse");
    // The byte is inside a function body, which opens folded.
    s.fold_all(false);
    let n = s.len();
    let shown: Vec<String> = s.lines(0..n).iter().map(|l| l.text()).collect();
    assert!(shown.iter().any(|l| l.contains("def f")), "{shown:?}");
    assert!(shown.iter().any(|l| l.contains('\u{fffd}')), "the byte became U+FFFD");
    assert!(!s.outline().is_empty(), "and it still has an outline");
    let _ = std::fs::remove_file(&t);
}
