//! Finding Java declarations.
#![deny(unsafe_code)]

use super::*;

fn syms(src: &str) -> Vec<Symbol> {
    symbols(src).expect("a balanced file")
}

fn named(src: &str) -> Vec<(String, Kind)> {
    syms(src).into_iter().map(|s| (s.path, s.kind)).collect()
}

#[test]
fn types_and_their_members_are_found() {
    let src = "\
package com.example.app;

import java.util.List;

/** A widget. */
public class Widget {
    private final String name;

    public Widget(String name) {
        this.name = name;
    }

    @Override
    public String toString() {
        return name;
    }

    static <T> T identity(T x) {
        return x;
    }
}
";
    let got = named(src);
    assert_eq!(got[0], ("com.example.app".into(), Kind::Mod));
    assert_eq!(got[1], ("java.util.List".into(), Kind::Import));
    assert_eq!(got[2], ("Widget".into(), Kind::Class));
    let members: Vec<&str> = got[3..].iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        members,
        vec!["Widget::Widget", "Widget::toString", "Widget::identity"],
        "constructor, override and generic method"
    );
}

#[test]
fn interfaces_enums_and_records_are_recognised() {
    let src = "\
public interface Shape {
    double area();
}
enum Colour { RED, GREEN }
public record Point(int x, int y) {
}
";
    let got = named(src);
    assert_eq!(got[0], ("Shape".into(), Kind::Interface));
    assert_eq!(got[1], ("Shape::area".into(), Kind::Func), "an abstract method");
    assert_eq!(got[2], ("Colour".into(), Kind::Type));
    assert_eq!(got[3], ("Point".into(), Kind::Class));
}

/// `if (`, `for (` and a bare call are not declarations, however much they
/// look like one to a shape-based recogniser.
#[test]
fn control_flow_is_not_a_declaration() {
    let src = "\
public class A {
    void go() {
        if (x) { }
        for (int i = 0; i < 3; i++) { }
        while (y) { }
        helper();
    }
}
";
    assert_eq!(named(src), vec![("A".into(), Kind::Class), ("A::go".into(), Kind::Func)]);
}

#[test]
fn a_doc_comment_and_annotations_above_belong_to_the_member() {
    let src = "\
public class A {
    /**
     * Does the thing.
     */
    @Override
    @SuppressWarnings(\"unchecked\")
    public void go() {
    }
}
";
    let s = &syms(src)[1];
    assert_eq!(s.name, "go");
    assert_eq!(s.doc, (1, 6), "javadoc and both annotations");
}

#[test]
fn keywords_in_comments_and_strings_are_not_declarations() {
    let src = "\
public class A {
    // public void commented() {}
    String s = \"public void quoted() {}\";
    void real() {
    }
}
";
    let all = syms(src);
    let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"real"), "{names:?}");
    assert!(!names.contains(&"commented"), "{names:?}");
    assert!(!names.contains(&"quoted"), "{names:?}");
}

#[test]
fn a_file_that_does_not_lex_has_no_symbols() {
    assert!(symbols("class A {").is_none());
    assert!(symbols("/* open").is_none());
    assert!(symbols("").is_some());
}

/// Against real Java, when a corpus is pointed at: set `TREAD_JAVA_CORPUS`.
/// Skipped when unset, since CI has no Java.
///
/// Run over the Spring Framework it reads 9,458 files and 177,806 symbols with
/// nothing refused (`docs/code.md`). That is the check worth repeating after
/// touching the lexer: fixtures only prove the shapes someone thought of.
#[test]
fn a_real_java_corpus_parses() {
    let Ok(root) = std::env::var("TREAD_JAVA_CORPUS") else {
        return;
    };
    let mut stack = vec![std::path::PathBuf::from(root)];
    let (mut seen, mut good, mut total) = (0usize, 0usize, 0usize);
    let mut bad: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("java") {
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
    println!("{good}/{seen} java files parse, {total} symbols");
    for b in &bad {
        println!("  unparsed: {b}");
    }
    assert!(seen > 0, "no .java under TREAD_JAVA_CORPUS");
}

