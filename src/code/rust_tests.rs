//! The Rust lexer, against the constructs that break naive scanners.
#![deny(unsafe_code)]

use super::*;

/// Code bytes only, concatenated — what brace counting actually sees.
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
    let src = "let a = \"x\"; // c\nfn f() {}\n";
    let toks = lex(src);
    assert_eq!(toks[0].start, 0);
    assert_eq!(toks.last().unwrap().end, src.len());
    for w in toks.windows(2) {
        assert_eq!(w[0].end, w[1].start, "no gap between {:?} and {:?}", w[0], w[1]);
    }
}

/// `/* /* */ */` is one comment. Ending at the first `*/` leaves the rest of
/// the file lexing as code.
#[test]
fn block_comments_nest() {
    let src = "a /* one /* two */ still */ b";
    assert_eq!(code(src), "a  b");
    assert!(ok(src));
}

/// The trap that matters most: a lifetime read as a string swallows every
/// brace after it.
#[test]
fn a_lifetime_is_not_a_char_literal() {
    let src = "fn f<'a>(x: &'a str) { }";
    assert!(ok(src), "the braces after the lifetimes still count");
    assert!(code(src).contains("'a"), "a lifetime stays code: {}", code(src));

    // ...but a real char literal is one, including the awkward spellings.
    for src in ["let c = 'x';", "let c = '\\'';", "let c = '\\n';", "let c = 'é';"] {
        assert!(ok(src), "{src}");
        assert!(!code(src).contains("'x"), "{src} -> {}", code(src));
    }
}

/// `'a'` really is a char literal even though `'a` alone is a lifetime — the
/// closing quote decides, not the first character.
#[test]
fn a_single_letter_char_literal_is_still_a_literal() {
    let src = "let c = 'a'; fn g() {}";
    assert!(ok(src));
    let toks = lex(src);
    assert!(
        toks.iter().any(|t| t.tok == Tok::Str && &src[t.start..t.end] == "'a'"),
        "{toks:?}"
    );
}

#[test]
fn raw_strings_end_only_at_a_matching_hash_count() {
    // A bare quote inside does not close it, and neither does one hash.
    let src = r####"let s = r##"a " b "# c"##; fn f() {}"####;
    assert!(ok(src));
    assert!(!code(src).contains('a'), "the raw body is not code: {}", code(src));

    // `r` as an identifier is not a raw string.
    assert!(ok("let r = 1; fn f() {}"));
    assert_eq!(code("let r = 1;"), "let r = 1;");
}

#[test]
fn braces_inside_strings_and_comments_do_not_count() {
    assert!(ok(r#"fn f() { let s = "}"; }"#));
    assert!(ok("fn f() { /* } */ }"));
    assert!(ok("fn f() { // }\n}"));
    assert!(ok(r#"let s = "\"}"; "#));
    assert!(ok("let b = b\"}\";"));
}

#[test]
fn an_unbalanced_or_unterminated_file_is_reported() {
    assert!(!ok("fn f() {"), "unclosed brace");
    assert!(!ok("fn f() }"), "closed what was never opened");
    assert!(!ok("let s = \"open"), "unterminated string");
    assert!(!ok("/* open"), "unterminated block comment");
    assert!(!ok("fn f(] {}"), "mismatched bracket");
    // A line comment needs no terminator.
    assert!(ok("// just a comment"));
    assert!(ok(""));
}

#[test]
fn doc_comments_are_marked_apart_from_ordinary_ones() {
    let toks = lex("/// outer\n//! inner\n// plain\n/** block */\n");
    let docs: Vec<bool> = toks
        .iter()
        .filter_map(|t| match t.tok {
            Tok::Line { doc } | Tok::Block { doc } => Some(doc),
            _ => None,
        })
        .collect();
    assert_eq!(docs, vec![true, true, false, true]);
}

/// Total means total: no input panics and no input loops forever.
#[test]
fn hostile_input_never_panics() {
    let cases = [
        "'", "\"", "/*", "*/", "r#", "r#\"", "b'", "'\\", "//", "'''", "\"\"\"",
        "r##\"unclosed", "/*/", "/**/", "'a'b'", "\\", "{{{{", "}}}}", "\u{fffd}",
        "let s = \"\u{0}\";", "fn f() { '\u{7f}' }", "é'x'é",
    ];
    for c in cases {
        let toks = lex(c);
        let _ = balanced(c, &toks);
        // Tiling must hold even for garbage, or a caller slicing by span panics.
        if !toks.is_empty() {
            assert_eq!(toks[0].start, 0, "{c:?}");
            assert_eq!(toks.last().unwrap().end, c.len(), "{c:?}");
        }
    }
}

/// Every byte prefix of a real file lexes without panicking — the truncation
/// case, which is how a half-written file on disk looks.
#[test]
fn every_prefix_of_a_real_file_lexes() {
    let src = include_str!("scan.rs");
    for i in 0..src.len() {
        if !src.is_char_boundary(i) {
            continue;
        }
        let head = &src[..i];
        let toks = lex(head);
        let _ = balanced(head, &toks);
    }
}

/// The oracle. If the lexer is wrong about Rust, it is wrong about this
/// repository — which is 60-odd files of exactly the constructs that break
/// scanners, including this test's own escaped quotes and raw strings.
#[test]
fn every_rust_file_in_this_repository_lexes_balanced() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                let src = match std::fs::read_to_string(&p) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                checked += 1;
                if !balanced(&src, &lex(&src)) {
                    failed.push(p.display().to_string());
                }
            }
        }
    }
    assert!(checked > 40, "expected to find the crate's sources, saw {checked}");
    assert!(failed.is_empty(), "these did not lex balanced: {failed:#?}");
}
