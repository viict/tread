//! The JS/TS lexer, against the constructs that break naive scanners.
#![deny(unsafe_code)]

use super::*;

fn code(src: &str) -> String {
    lex(src)
        .into_iter()
        .filter(|s| s.tok == Tok::Code)
        .map(|s| &src[s.start..s.end])
        .collect()
}

fn ok(src: &str) -> bool {
    balanced(src, &lex(src))
}

#[test]
fn tokens_tile_the_input_with_no_gaps() {
    let src = "const a = `x${ b }y`; // c\nfunction f() {}\n";
    let toks = lex(src);
    assert_eq!(toks[0].start, 0);
    assert_eq!(toks.last().unwrap().end, src.len());
    for w in toks.windows(2) {
        assert_eq!(w[0].end, w[1].start, "gap between {:?} and {:?}", w[0], w[1]);
    }
}

/// The ambiguity with no local answer: `a / b` divides, `/ab+/` is a regex.
#[test]
fn a_division_is_not_a_regex() {
    // Division: the braces after it must still count.
    let src = "const x = a / b; function f() { }";
    assert!(ok(src), "{}", code(src));
    assert!(code(src).contains("a / b"), "{}", code(src));

    // A regex: its contents are not code, so a brace inside cannot count.
    let src = "const re = /[}{]+/g; function f() { }";
    assert!(ok(src), "the braces inside the regex are not counted");
    assert!(!code(src).contains("[}{]"), "{}", code(src));
}

#[test]
fn a_regex_after_a_keyword_or_bracket_is_still_a_regex() {
    assert!(ok("if (x) { return /}/.test(s); }"));
    assert!(ok("const a = [/}/, /{/];"));
    assert!(ok("split(/[},{]/).map(x => x);"));
}

/// `` `a ${ f(`b`) } c` `` — a literal containing code containing a literal.
#[test]
fn template_literals_nest_through_their_interpolations() {
    let src = "const s = `a ${ f(`b ${ c }`) } d`; function g() { }";
    assert!(ok(src), "{}", code(src));
    // The code inside `${}` is code...
    assert!(code(src).contains("f("), "{}", code(src));
    // ...but the literal text around it is not.
    assert!(!code(src).contains("a "), "{}", code(src));
}

#[test]
fn braces_inside_a_template_do_not_count() {
    assert!(ok("const s = `}{`; function f() { }"));
    assert!(ok("const s = `${ { a: 1 } }`;"));
    assert!(ok("const s = `\\``;"), "an escaped backtick does not close it");
}

#[test]
fn braces_inside_strings_and_comments_do_not_count() {
    assert!(ok("function f() { const s = '}'; }"));
    assert!(ok("function f() { /* } */ }"));
    assert!(ok("function f() { // }\n}"));
    assert!(ok(r#"const s = "\"}";"#));
}

/// JSX expressions balance like any other braces.
#[test]
fn jsx_needs_no_special_handling() {
    let src = "const C = () => <div className={cls}>{items.map(i => <li>{i}</li>)}</div>;";
    assert!(ok(src), "{}", code(src));
    assert!(ok("const a = x < y && z > w;"), "comparisons are not tags");
}

#[test]
fn an_unbalanced_or_unterminated_file_is_reported() {
    assert!(!ok("function f() {"), "unclosed brace");
    assert!(!ok("function f() }"));
    assert!(!ok("const s = `open"), "unterminated template");
    assert!(!ok("/* open"), "unterminated block comment");
    assert!(ok("// just a comment"));
    assert!(ok(""));
}

#[test]
fn jsdoc_is_marked_apart_from_an_ordinary_block_comment() {
    let toks = lex("/** doc */\n/* plain */\n// line\n");
    let docs: Vec<bool> = toks
        .iter()
        .filter_map(|t| match t.tok {
            Tok::Block { doc } => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(docs, vec![true, false]);
}

#[test]
fn hostile_input_never_panics() {
    let cases = [
        "`", "${", "`${", "`${`", "/", "//", "/*", "*/", "'", "\"", "\\",
        "`${ `${ `${", "}}}", "{{{", "/[/", "/\\", "'\n'", "`\\", "\u{fffd}",
        "const s = `${'}'}`;", "a=/}/;{", "<div>{",
    ];
    for c in cases {
        let toks = lex(c);
        let _ = balanced(c, &toks);
        if !toks.is_empty() {
            assert_eq!(toks[0].start, 0, "{c:?}");
            assert_eq!(toks.last().unwrap().end, c.len(), "{c:?}");
        }
    }
}

#[test]
fn every_prefix_of_a_realistic_file_lexes() {
    let src = "\
import { a } from './a';
/** Does the thing. */
export async function run(x: number): Promise<void> {
  const re = /a\\/b/g;
  const s = `x ${ a / 2 } ${ `y${x}` }`;
  if (x / 2 > 1) { return; }
}
export class C {
  #n = 0;
  get value() { return this.#n; }
}
";
    for i in 0..src.len() {
        if !src.is_char_boundary(i) {
            continue;
        }
        let head = &src[..i];
        let toks = lex(head);
        let _ = balanced(head, &toks);
    }
    assert!(ok(src), "the whole file lexes: {}", code(src));
}

/// Against a real corpus of JavaScript and TypeScript, when one is pointed at.
///
/// Set `TREAD_JS_CORPUS` to a directory of real code — a `node_modules`, a
/// checkout — and this reports what fraction lexes balanced. Fixtures only
/// prove the cases someone thought of; a few thousand files someone else wrote
/// are what actually finds the gaps. Skipped when the variable is unset, so CI
/// (which has no JavaScript) stays green.
#[test]
fn a_real_javascript_corpus_lexes() {
    let Ok(root) = std::env::var("TREAD_JS_CORPUS") else {
        return;
    };
    let limit: usize = std::env::var("TREAD_JS_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let mut stack = vec![std::path::PathBuf::from(root)];
    let (mut seen, mut good) = (0usize, 0usize);
    let mut bad: Vec<String> = Vec::new();
    while let Some(dir) = stack.pop() {
        if seen >= limit {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !matches!(ext, "ts" | "js" | "tsx" | "jsx" | "mjs" | "cjs") {
                continue;
            }
            if p.to_string_lossy().contains(".min.") {
                continue;
            }
            // Lossily, as the reader itself does: a file with a stray byte is
            // one the reader opens, so it is one this must judge.
            let Ok(raw) = std::fs::read(&p) else { continue };
            let src = String::from_utf8_lossy(&raw).into_owned();
            seen += 1;
            match balanced(&src, &lex(&src)) {
                true => good += 1,
                false if bad.len() < 20 => bad.push(p.display().to_string()),
                false => {}
            }
            if seen >= limit {
                break;
            }
        }
    }
    let pct = 100.0 * good as f64 / seen.max(1) as f64;
    println!("lexed {good}/{seen} ({pct:.1}%) balanced");
    for b in &bad {
        println!("  unbalanced: {b}");
    }
    assert!(seen > 0, "no files found under TREAD_JS_CORPUS");
}
