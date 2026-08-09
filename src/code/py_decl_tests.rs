//! Finding Python declarations.
#![deny(unsafe_code)]

use super::*;

fn syms(src: &str) -> Vec<Symbol> {
    symbols(src).expect("a balanced file")
}

fn named(src: &str) -> Vec<(String, Kind)> {
    syms(src).into_iter().map(|s| (s.path, s.kind)).collect()
}

#[test]
fn functions_classes_and_methods_are_found_by_indentation() {
    let src = "\
import os
from pathlib import Path

MAX_ROWS = 32

def top(a, b):
    return a + b

class Store:
    def __init__(self, path):
        self.path = path

    async def load(self, key):
        return None

def after():
    pass
";
    assert_eq!(
        named(src),
        vec![
            ("os".into(), Kind::Import),
            ("pathlib".into(), Kind::Import),
            ("MAX_ROWS".into(), Kind::Const),
            ("top".into(), Kind::Func),
            ("Store".into(), Kind::Class),
            ("Store::__init__".into(), Kind::Func),
            ("Store::load".into(), Kind::Func),
            ("after".into(), Kind::Func),
        ]
    );
    // The methods nest under the class; everything else is top level.
    let depths: Vec<u8> = syms(src).iter().map(|s| s.depth).collect();
    assert_eq!(depths, vec![0, 0, 0, 0, 0, 1, 1, 0]);
}

/// The inversion that makes Python different: its documentation is the first
/// statement of the body, so it must be pulled into the signature or folding
/// hides exactly the thing worth reading.
#[test]
fn a_docstring_survives_folding() {
    let src = "\
def sniff(sample):
    \"\"\"Guess the delimiter.

    Longer prose here.
    \"\"\"
    best = 0
    return best
";
    let s = &syms(src)[0];
    assert_eq!(s.sig, (0, 5), "the def line and the whole docstring");
    assert_eq!(s.body, (5, 7), "only the code folds away");
}

#[test]
fn a_one_line_docstring_is_recognised() {
    let src = "def f():\n    \"\"\"Short.\"\"\"\n    return 1\n";
    let s = &syms(src)[0];
    assert_eq!(s.sig, (0, 2));
    assert_eq!(s.body, (2, 3));
}

/// A parameter list spread over several lines is still one signature.
#[test]
fn a_multi_line_signature_is_one_signature() {
    let src = "\
def wide(
    a,
    b,
):
    return a
";
    let s = &syms(src)[0];
    assert_eq!(s.sig, (0, 4));
    assert_eq!(s.body, (4, 5));
}

#[test]
fn a_nested_helper_is_not_listed() {
    let src = "\
def outer():
    def helper():
        pass
    class Local:
        pass
    return 1
";
    assert_eq!(named(src), vec![("outer".into(), Kind::Func)]);
}

#[test]
fn keywords_in_comments_and_strings_are_not_declarations() {
    let src = "\
# def commented():
s = 'def quoted():'
def real():
    pass
";
    let names: Vec<String> = syms(src).into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"real".to_string()), "{names:?}");
    assert!(!names.contains(&"commented".to_string()), "{names:?}");
    assert!(!names.contains(&"quoted".to_string()), "{names:?}");
}

/// A decorator belongs to what it decorates.
#[test]
fn decorators_above_a_function_belong_to_it() {
    let src = "\
@app.route(\"/\")
@cached
def index():
    return 1
";
    let s = &syms(src)[0];
    assert_eq!(s.name, "index");
    assert_eq!(s.doc, (0, 2), "both decorators");
}

#[test]
fn a_file_that_does_not_lex_has_no_symbols() {
    assert!(symbols("f(\n  1,\n").is_none(), "unclosed bracket");
    assert!(symbols("s = '''open\n").is_none(), "unterminated docstring");
    assert!(symbols("").is_some());
}

/// Against a real corpus of Python, when one is pointed at: set
/// `TREAD_PY_CORPUS`. Fixtures only prove the cases someone thought of.
/// Skipped when unset, so CI (which has no corpus) stays green.
#[test]
fn a_real_python_corpus_parses() {
    let Ok(root) = std::env::var("TREAD_PY_CORPUS") else {
        return;
    };
    let mut stack = vec![std::path::PathBuf::from(root)];
    let (mut seen, mut good, mut total) = (0usize, 0usize, 0usize);
    let mut bad: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        if seen >= 3000 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("py") || seen >= 3000 {
                continue;
            }
            // Lossily, as the reader itself does: a file with a stray byte is
            // one the reader opens, so it is one this must judge.
            let Ok(raw) = std::fs::read(&p) else { continue };
            let src = String::from_utf8_lossy(&raw).into_owned();
            seen += 1;
            match symbols(&src) {
                Some(s) => {
                    good += 1;
                    total += s.len();
                    let n = src.lines().count();
                    for sym in &s {
                        let (a, b) = sym.span();
                        assert!(a <= b && b <= n, "{} {}: {a}..{b} of {n}", p.display(), sym.path);
                    }
                }
                None if bad.len() < 8 => bad.push(p.display().to_string()),
                None => {}
            }
        }
    }
    let pct = 100.0 * good as f64 / seen.max(1) as f64;
    println!("{good}/{seen} python files parse ({pct:.1}%), {total} symbols");
    for b in &bad {
        println!("  unparsed: {b}");
    }
    assert!(seen > 0, "no .py under TREAD_PY_CORPUS");
}

/// Python has no braces, so a suite is a `:` line and what is indented under
/// it. `def` and `class` are left out: a declaration already owns its body.
#[test]
fn suites_fold_but_declarations_are_left_to_their_own_regions() {
    let src = "\
def f(items):
    if items:
        first()
        second()
        third()
    for i in items:
        loop_body()
        more()
        last()
    return 1
";
    let lines: Vec<&str> = src.lines().collect();
    let got = blocks(&lines, 3);
    // The `if` and the `for` — not the `def`, which its declaration owns.
    assert_eq!(got, vec![(1, 2, 5), (5, 6, 9)], "{got:?}");
}

#[test]
fn a_short_suite_and_a_multi_line_condition_are_handled() {
    let src = "\
def f(a):
    if a:
        one()
    if (
        a
        and b
    ):
        two()
        three()
        four()
";
    let lines: Vec<&str> = src.lines().collect();
    let got = blocks(&lines, 3);
    // The one-line suite is skipped; the block after a multi-line condition is
    // found, and opens at the `):` rather than at the `if (`.
    assert_eq!(got, vec![(6, 7, 10)], "{got:?}");
}

/// A `:` inside a string is not a block.
#[test]
fn a_colon_in_a_literal_does_not_open_a_suite() {
    let src = "s = 'if x:'\nt = 1\n";
    let toks = crate::code::py::lex(src);
    let blanked = crate::code::scan::blank(src, &toks);
    let lines: Vec<&str> = blanked.lines().collect();
    assert!(blocks(&lines, 1).is_empty());
}


/// Found on LangChain: a class declared *inside a method* was taken as the
/// container, which ended the real class and silently dropped every method
/// after it.
///
/// ```python
/// class Outer:
///     def a(self):
///         class Inner: ...   # deeper than the member level
///     def b(self): ...       # was lost
/// ```
#[test]
fn a_class_nested_in_a_method_does_not_end_its_class() {
    let src = "\
class Outer:
    def a(self):
        class Inner(Base):
            pass
        return Inner

    def b(self):
        return 1

def free():
    pass
";
    assert_eq!(
        named(src),
        vec![
            ("Outer".into(), Kind::Class),
            ("Outer::a".into(), Kind::Func),
            ("Outer::b".into(), Kind::Func),
            ("free".into(), Kind::Func),
        ],
        "the nested class is not listed, and `b` survives it"
    );
}

/// The same rule from the other side: a helper defined inside a top-level
/// function is not a member of anything.
#[test]
fn a_declaration_inside_a_function_is_not_a_member() {
    let src = "\
def outer():
    class Local(Base):
        pass

    def helper():
        pass
    return 1

class After:
    def m(self):
        pass
";
    assert_eq!(
        named(src),
        vec![
            ("outer".into(), Kind::Func),
            ("After".into(), Kind::Class),
            ("After::m".into(), Kind::Func),
        ]
    );
}
