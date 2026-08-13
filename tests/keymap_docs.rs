//! The keybinding table in README.md must match `src/pager/keys.rs`.
//!
//! Integration tests cannot import a binary crate's items, so this reads both
//! files as text. That is enough: `BINDINGS` is a `const` array of literals, so
//! the `keys:` and `desc:` strings are right there in the source. The point is
//! only to fail loudly when someone adds a key and forgets the docs.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Pull `("j / ↓", "line down")` pairs out of the `BINDINGS` literal.
fn bindings_from_source() -> Vec<(String, String)> {
    let src = read("src/pager/keys.rs");
    let body = src
        .split_once("pub const BINDINGS")
        .expect("BINDINGS array")
        .1;
    let mut out = Vec::new();
    let mut keys: Option<String> = None;
    for line in body.lines() {
        let t = line.trim();
        if let Some(v) = field(t, "keys:") {
            keys = Some(v);
        } else if let Some(v) = field(t, "desc:") {
            if let Some(k) = keys.take() {
                out.push((k, v));
            }
        }
    }
    assert!(out.len() > 20, "only parsed {} bindings", out.len());
    out
}

/// `keys: "j / \u{2193}",` -> `j / ↓`, with escapes resolved.
fn field(line: &str, name: &str) -> Option<String> {
    let rest = line.strip_prefix(name)?.trim();
    let inner = rest.strip_prefix('"')?;
    let end = inner.rfind('"')?;
    Some(unescape(&inner[..end]))
}

/// Resolve the `\u{XXXX}` and `\"` escapes the source uses. Nothing else
/// appears in these literals, and an unknown escape is left alone rather than
/// silently dropped.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match cs.next() {
            Some('u') => {
                if cs.peek() == Some(&'{') {
                    cs.next();
                    let mut hex = String::new();
                    for c in cs.by_ref() {
                        if c == '}' {
                            break;
                        }
                        hex.push(c);
                    }
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(c) => out.push(c),
                        None => panic!("bad \\u{{{hex}}} escape"),
                    }
                }
            }
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[test]
fn readme_documents_every_binding() {
    let readme = read("README.md");
    let mut missing = Vec::new();
    for (keys, desc) in bindings_from_source() {
        let row = format!("| `{keys}` | {desc} |");
        if !readme.contains(&row) {
            missing.push(row);
        }
    }
    assert!(
        missing.is_empty(),
        "README.md key table is out of date; add:\n{}",
        missing.join("\n")
    );
}

#[test]
fn the_readme_table_invents_nothing() {
    let readme = read("README.md");
    let known: Vec<String> = bindings_from_source()
        .into_iter()
        .map(|(k, d)| format!("| `{k}` | {d} |"))
        .collect();
    // Only inspect the "## Keys" section, so other tables are left alone.
    let keys_section = readme
        .split_once("## Keys")
        .expect("## Keys heading")
        .1
        .split("\n## ")
        .next()
        .unwrap();
    for line in keys_section.lines() {
        let t = line.trim();
        if !t.starts_with("| `") {
            continue;
        }
        assert!(
            known.iter().any(|k| k == t),
            "README documents a binding that does not exist: {t}"
        );
    }
}

/// The help overlay and `--help` must not promise keys the code lacks either.
#[test]
fn the_cli_help_mentions_the_core_keys() {
    let cli = read("src/cli.rs");
    // `r` is pinned by its sentence rather than by the letter: a bare "r"
    // matches almost any word in the help text, so it would pass with the key
    // deleted — which is exactly the drift this test exists to catch.
    for needle in ["za", "zM", "zR", "Tab", "Backspace", "q", "r shows the raw record"] {
        assert!(cli.contains(needle), "--help never mentions {needle}");
    }
}
