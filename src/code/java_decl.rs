//! Java: finding the declarations.
//!
//! Java is brace-structured, so the shared line arithmetic ([`super::decl`])
//! applies unchanged and only the recognizer is new.
//!
//! The awkward shape is the method: Java has no keyword for one. `public static
//! void main(String[] args) {` is a pile of modifiers, a return type, a name
//! and a parenthesis, and what marks it out is the *shape* rather than any
//! word — which is why this reads right-to-left from the `(` instead of
//! left-to-right from a keyword.
#![deny(unsafe_code)]

use super::decl::{self, ident, is_ident_char, word};
use super::java::{balanced, lex};
use super::{Kind, Symbol};

/// Modifiers that may precede any declaration, in any order.
const MODIFIERS: [&str; 12] = [
    "public", "private", "protected", "static", "final", "abstract", "default", "native",
    "synchronized", "transient", "volatile", "strictfp",
];

/// The symbols in `src`, or `None` when it does not lex cleanly.
pub fn symbols(src: &str) -> Option<Vec<Symbol>> {
    decl::symbols(src, lex, balanced, recognise)
}

/// What, if anything, this line declares.
fn recognise(line: &str, _raw: &str) -> Option<(Kind, String)> {
    let trimmed = line.trim_start();
    // `package a.b.c;` and `import a.b.C;` name a place, not a thing.
    for (kw, kind) in [("package", Kind::Mod), ("import", Kind::Import)] {
        if let Some(rest) = word(trimmed, kw) {
            let name = rest.trim().trim_end_matches(';').trim();
            // `import static a.b.C.d;`
            let name = word(name, "static").unwrap_or(name);
            return Some((kind, name.to_string()));
        }
    }
    let rest = strip_modifiers(trimmed);
    for (kw, kind) in [
        ("class", Kind::Class),
        ("interface", Kind::Interface),
        ("record", Kind::Class),
        ("enum", Kind::Type),
        ("@interface", Kind::Interface),
    ] {
        if let Some(after) = word(rest, kw) {
            return Some((kind, ident(after)));
        }
    }
    method(rest)
}

/// A method or constructor, recognised by shape.
///
/// Reads back from the first `(`: the identifier immediately before it is the
/// name, and something must precede *that* — a return type, or nothing at all
/// for a constructor, which is why a bare `name(` is accepted only when the
/// line also opens a body or ends in `;` (an interface method).
fn method(rest: &str) -> Option<(Kind, String)> {
    let open = rest.find('(')?;
    let head = rest[..open].trim_end();
    let name = tail_ident(head)?;
    if name.is_empty() {
        return None;
    }
    // `if (`, `for (`, `while (`, `switch (`, `catch (`, `return (` are not
    // declarations, and at depth 0 or 1 a stray call is not either.
    if matches!(
        name.as_str(),
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "super" | "this"
    ) {
        return None;
    }
    let before = head[..head.len() - name.len()].trim_end();
    let opens_body = rest[open..].contains('{') || rest.trim_end().ends_with(';');
    // A constructor has nothing before the name; a method has a return type.
    match before.is_empty() && !opens_body {
        true => None,
        false => Some((Kind::Func, name)),
    }
}

/// The identifier at the end of `s`, ignoring any generic argument list.
fn tail_ident(s: &str) -> Option<String> {
    let s = s.trim_end();
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_char(*c))
        .last()
        .map(|(i, _)| i)?;
    Some(s[start..].to_string())
}

/// Drop leading modifiers, annotations and generic parameter lists.
fn strip_modifiers(line: &str) -> &str {
    let mut rest = line;
    loop {
        // `@Override` and friends sit on their own line or in front.
        if rest.starts_with('@') {
            let cut = rest.find(char::is_whitespace).unwrap_or(rest.len());
            rest = rest[cut..].trim_start();
            continue;
        }
        // `<T> T identity(T x)` — a method's own type parameters.
        if rest.starts_with('<') {
            match rest.find('>') {
                Some(i) => {
                    rest = rest[i + 1..].trim_start();
                    continue;
                }
                None => return rest,
            }
        }
        match MODIFIERS.iter().find_map(|m| word(rest, m)) {
            Some(r) => rest = r,
            None => return rest,
        }
    }
}

#[cfg(test)]
#[path = "java_decl_tests.rs"]
mod tests;
