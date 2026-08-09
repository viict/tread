//! Finding Rust declarations, against real shapes.
#![deny(unsafe_code)]

use super::*;

fn syms(src: &str) -> Vec<Symbol> {
    symbols(src).expect("a balanced file")
}

fn paths(src: &str) -> Vec<String> {
    syms(src).into_iter().map(|s| s.path).collect()
}

#[test]
fn a_function_carries_its_doc_comment_and_folds_its_body() {
    let src = "\
/// Guess the delimiter.
///
/// More prose.
pub fn sniff(sample: &[u8]) -> u8 {
    let x = 1;
    x
}
";
    let s = &syms(src)[0];
    assert_eq!(s.kind, Kind::Func);
    assert_eq!(s.name, "sniff");
    assert_eq!(s.doc, (0, 3), "three comment lines above");
    assert_eq!(s.sig, (3, 4), "the signature line itself");
    // The body includes the closing brace, so folding leaves the signature.
    assert_eq!(s.body, (4, 7));
    assert_eq!(s.hidden(), 3);
}

#[test]
fn methods_are_qualified_by_the_impl_they_are_in() {
    let src = "\
impl DirSource {
    pub fn open(path: &Path) -> DirSource {
        todo!()
    }
    fn build(&mut self) {
    }
}

fn free() {}
";
    assert_eq!(
        paths(src),
        vec!["DirSource", "DirSource::open", "DirSource::build", "free"]
    );
    let depths: Vec<u8> = syms(src).iter().map(|s| s.depth).collect();
    assert_eq!(depths, vec![0, 1, 1, 0], "members nest under the impl");
}

#[test]
fn an_impl_of_a_trait_is_named_for_its_type() {
    assert_eq!(paths("impl Source for DirSource {\n}\n"), vec!["DirSource"]);
    assert_eq!(paths("impl<'a> Iterator for Rows<'a> {\n}\n"), vec!["Rows"]);
    assert_eq!(paths("impl Item {\n}\n"), vec!["Item"]);
}

/// `const fn` is a function; a bare `const` is a constant. The last keyword
/// wins, and modifiers in front must not confuse it.
#[test]
fn modifiers_do_not_hide_the_keyword() {
    let src = "\
pub(crate) const fn a() -> u8 { 0 }
pub const B: u8 = 1;
pub(super) async unsafe fn c() {}
const D: [u8; 2] = *b\"xy\";
";
    let got: Vec<(String, Kind)> = syms(src).into_iter().map(|s| (s.name, s.kind)).collect();
    assert_eq!(
        got,
        vec![
            ("a".into(), Kind::Func),
            ("B".into(), Kind::Const),
            ("c".into(), Kind::Func),
            ("D".into(), Kind::Const),
        ]
    );
}

#[test]
fn every_top_level_kind_is_recognised() {
    let src = "\
use super::parse::{Records, QUOTE};
struct S { a: u8 }
enum E { A, B }
trait T { fn m(&self); }
type Alias = u8;
mod inner { }
static S2: u8 = 0;
macro_rules! mac { () => {} }
";
    let kinds: Vec<Kind> = syms(src).iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&Kind::Import), "{kinds:?}");
    assert!(kinds.contains(&Kind::Type), "{kinds:?}");
    assert!(kinds.contains(&Kind::Trait), "{kinds:?}");
    assert!(kinds.contains(&Kind::Alias), "{kinds:?}");
    assert!(kinds.contains(&Kind::Mod), "{kinds:?}");
    assert!(kinds.contains(&Kind::Const), "{kinds:?}");
    assert!(kinds.contains(&Kind::Macro), "{kinds:?}");
}

/// A helper defined inside a function is noise in an outline.
#[test]
fn an_item_nested_inside_a_function_is_not_listed() {
    let src = "\
fn outer() {
    fn helper() {}
    struct Local;
}
";
    assert_eq!(paths(src), vec!["outer"]);
}

/// The safety valve: no outline at all rather than a wrong one.
#[test]
fn a_file_that_does_not_lex_has_no_symbols() {
    assert!(symbols("fn f() {").is_none(), "unclosed brace");
    assert!(symbols("let s = \"open").is_none(), "unterminated string");
    assert!(symbols("/* open").is_none());
    assert!(symbols("").is_some(), "an empty file is fine, just empty");
}

/// A `fn` mentioned in a comment or a string is not a declaration.
#[test]
fn keywords_in_comments_and_strings_are_not_declarations() {
    let src = "\
// fn commented() {}
/// fn documented() {}
fn real() {
    let s = \"fn quoted() {}\";
}
";
    assert_eq!(paths(src), vec!["real"]);
}

/// Against a real file in this repository, so the shapes are not invented.
#[test]
fn a_real_source_file_yields_the_symbols_it_declares() {
    let src = include_str!("../csv/delim.rs");
    let got = paths(src);
    for want in [
        "CANDIDATES",
        "DEFAULT_DELIM",
        "SNIFF_ROWS",
        "sniff",
        "score",
        "parse_delim",
        "sep_line",
    ] {
        assert!(got.iter().any(|p| p == want), "{want} missing from {got:?}");
    }
}

/// Every file in this repository yields symbols, and every symbol's ranges are
/// inside the file and in order. A range that runs backwards or past the end
/// would panic the moment a view sliced with it.
#[test]
fn every_repository_file_yields_sane_ranges() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut files = 0usize;
    let mut total = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let n = src.lines().count();
            let Some(syms) = symbols(&src) else {
                panic!("{} did not lex", p.display());
            };
            files += 1;
            total += syms.len();
            for s in &syms {
                let (a, b) = s.span();
                assert!(a <= b, "{} {}: span backwards {a}..{b}", p.display(), s.path);
                assert!(b <= n, "{} {}: span past end {b} > {n}", p.display(), s.path);
                assert!(s.doc.0 <= s.doc.1 && s.sig.0 <= s.sig.1 && s.body.0 <= s.body.1);
            }
        }
    }
    assert!(files > 40, "saw {files} files");
    assert!(total > 400, "expected the crate to declare a lot, saw {total}");
}
