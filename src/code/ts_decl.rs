//! JavaScript and TypeScript: finding the declarations.
//!
//! The shapes worth listing are less uniform than Rust's. A function can be a
//! `function` statement, a `const` bound to an arrow, a class method, or a
//! getter — and `const` alone is a value, so what follows the `=` decides which
//! it is. That decision is the only interesting thing here; the line arithmetic
//! is shared ([`super::decl`]).
#![deny(unsafe_code)]

use super::decl::{self, ident, starts_word, word};
use super::{Kind, Symbol};
use super::ts::{balanced, lex};

/// Modifiers that may precede a declaration, in any order.
const MODIFIERS: [&str; 11] = [
    "export", "default", "declare", "async", "static", "public", "private",
    "protected", "readonly", "abstract", "const",
];

/// The symbols in `src`, or `None` when it does not lex cleanly.
pub fn symbols(src: &str) -> Option<Vec<Symbol>> {
    decl::symbols(src, lex, balanced, recognise)
}

/// What, if anything, this line declares.
fn recognise(line: &str, raw: &str) -> Option<(Kind, String)> {
    let trimmed = line.trim_start();
    if word(trimmed, "import").is_some() {
        // From the raw line: the module path is a string literal, which the
        // blanked line has replaced with spaces.
        let rest = word(raw.trim_start(), "import").unwrap_or("");
        return Some((Kind::Import, import_name(rest)));
    }
    let rest = strip_modifiers(trimmed);
    for (kw, kind) in [
        ("function", Kind::Func),
        ("class", Kind::Class),
        ("interface", Kind::Interface),
        ("namespace", Kind::Mod),
        ("module", Kind::Mod),
        ("enum", Kind::Type),
        ("type", Kind::Alias),
    ] {
        if let Some(after) = word(rest, kw) {
            // `function*` and `class extends` still name what follows.
            let after = after.trim_start_matches('*').trim_start();
            return Some((kind, ident(after)));
        }
    }
    // `const x = () => {}` is a function; `const x = 1` is a value.
    for kw in ["const", "let", "var"] {
        if let Some(after) = word(rest, kw) {
            let name = ident(after);
            if name.is_empty() {
                return None;
            }
            return Some((binding_kind(after), name));
        }
    }
    member(rest)
}

/// A binding's kind, from what it is bound to.
fn binding_kind(after: &str) -> Kind {
    let Some((_, value)) = after.split_once('=') else {
        return Kind::Const;
    };
    let v = value.trim_start();
    // `= async () =>`, `= () =>`, `= function`, `= (a: T): R =>`
    let v = word(v, "async").unwrap_or(v);
    if starts_word(v, "function") {
        return Kind::Func;
    }
    match v.starts_with('(') || v.contains("=>") {
        true => Kind::Func,
        false => Kind::Const,
    }
}

/// A class member: a method, a getter, or a property.
///
/// Only reached at depth 1 inside a class or interface, so a bare `name(` here
/// is a method rather than a call.
fn member(rest: &str) -> Option<(Kind, String)> {
    let rest = rest.trim_start_matches(['*', '#']).trim_start();
    for kw in ["get", "set"] {
        if let Some(after) = word(rest, kw) {
            let name = ident(after);
            if !name.is_empty() {
                return Some((Kind::Func, name));
            }
        }
    }
    let name = ident(rest);
    if name.is_empty() {
        return None;
    }
    let after = rest[name.len()..].trim_start();
    // A method is `name(` or `name<T>(`; a property is `name =` or `name:`.
    let is_call = after.starts_with('(')
        || (after.starts_with('<') && after.contains('('));
    match is_call {
        true => Some((Kind::Func, name)),
        false => match after.starts_with('=') || after.starts_with(':') {
            true => Some((Kind::Const, name)),
            false => None,
        },
    }
}

/// The module an `import` names, which is what a reader follows.
///
/// `import { a } from './x'` and `import './y'` both answer the path — the
/// bindings are noise in an outline, and the path is what jumping needs.
fn import_name(rest: &str) -> String {
    let after = match rest.rfind(" from ") {
        Some(i) => &rest[i + 6..],
        None => rest,
    };
    let quoted = after.trim().trim_matches(|c| c == '\'' || c == '"' || c == ';');
    match quoted.trim().is_empty() {
        true => rest.trim().trim_end_matches(';').trim().to_string(),
        false => quoted.trim().to_string(),
    }
}

/// Drop any leading modifiers.
fn strip_modifiers(line: &str) -> &str {
    let mut rest = line;
    loop {
        // `const` is a modifier only in `export const enum`; on its own it
        // binds a value, so it must not be stripped before the binding check.
        let next = MODIFIERS
            .iter()
            .filter(|m| **m != "const" || starts_word(rest, "const enum"))
            .find_map(|m| word(rest, m));
        match next {
            Some(r) => rest = r,
            None => return rest,
        }
    }
}

#[cfg(test)]
#[path = "ts_decl_tests.rs"]
mod tests;
