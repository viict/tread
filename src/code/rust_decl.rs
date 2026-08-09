//! Rust: finding the declarations.
//!
//! Runs over the *blanked* source ([`scan::blank`]), so a `fn` inside a comment
//! or a brace inside a string cannot be seen. Everything here is therefore line
//! arithmetic on text that is guaranteed to be code.
//!
//! What it does not do: understand `cfg`, expand a macro, or know that two
//! `impl` blocks describe one type. A reader gets what is written in the file,
//! which is the honest answer for a tool that never compiles anything.
#![deny(unsafe_code)]

use super::decl::{self, ident, starts_word, word};
use super::rust::{balanced, lex};
use super::{Kind, Symbol};

/// Keywords that begin a declaration, longest first so `macro_rules!` is not
/// read as an identifier.
const DECLS: [(&str, Kind); 11] = [
    ("macro_rules!", Kind::Macro),
    ("struct", Kind::Type),
    ("static", Kind::Const),
    ("trait", Kind::Trait),
    ("union", Kind::Type),
    ("const", Kind::Const),
    ("type", Kind::Alias),
    ("impl", Kind::Impl),
    ("enum", Kind::Type),
    ("mod", Kind::Mod),
    ("use", Kind::Import),
];

/// Modifiers that may sit in front of a declaration keyword.
const MODIFIERS: [&str; 6] = ["pub", "async", "unsafe", "extern", "default", "const"];

/// The symbols in `src`, or `None` when the file does not lex cleanly.
///
/// `None` is the safety valve the whole feature rests on: a mis-lexed brace
/// swallows the rest of the file into one body and *hides* it, so a file that
/// does not balance gets no outline at all and is shown raw instead
/// (SPEC.md §Code).
pub fn symbols(src: &str) -> Option<Vec<Symbol>> {
    decl::symbols(src, lex, balanced, recognise)
}

/// What, if anything, this line declares.
fn recognise(line: &str, raw: &str) -> Option<(Kind, String)> {
    let (kind, rest) = keyword(line)?;
    // A `use` path is all identifiers, so the blanked line carries it intact;
    // the raw line is taken anyway so the two languages read the same way.
    if kind == Kind::Import {
        if let Some(after) = word(raw.trim_start(), "use") {
            return Some((kind, after.trim().trim_end_matches(';').trim().to_string()));
        }
    }
    Some((kind, name_of(kind, rest)))
}

/// Split a line into its declaration keyword and what follows, skipping any
/// modifiers. `pub(crate) async fn foo` -> `(Func, "foo ...")`.
fn keyword(line: &str) -> Option<(Kind, &str)> {
    let mut rest = line.trim_start();
    loop {
        // `fn` is checked inside the loop because `const fn` is a function,
        // while a bare `const` is a constant — the last keyword wins.
        if let Some(r) = word(rest, "fn") {
            return Some((Kind::Func, r));
        }
        if let Some((kw, kind)) = DECLS.iter().find(|(kw, _)| starts_word(rest, kw)) {
            let r = &rest[kw.len()..];
            // `const fn` keeps looking; `const NAME` does not.
            if *kw == "const" && word(r.trim_start(), "fn").is_some() {
                rest = r.trim_start();
                continue;
            }
            return Some((*kind, r));
        }
        let next = MODIFIERS.iter().find_map(|m| skip_modifier(rest, m))?;
        rest = next;
    }
}

/// Consume a modifier and anything parenthesised after it (`pub(crate)`) or
/// quoted (`extern "C"`).
fn skip_modifier<'a>(rest: &'a str, m: &str) -> Option<&'a str> {
    let r = word(rest, m)?;
    let r = r.trim_start();
    let r = match r.starts_with('(') {
        true => &r[r.find(')').map(|i| i + 1).unwrap_or(r.len())..],
        false => r,
    };
    Some(r.trim_start())
}

/// The declared name: the identifier after the keyword.
///
/// `impl` is different — what follows is a type, possibly `Trait for Type`, and
/// the useful label is the type.
fn name_of(kind: Kind, rest: &str) -> String {
    if kind == Kind::Impl {
        return impl_name(rest);
    }
    if kind == Kind::Import {
        // A `use` names a path, not an identifier; the path is the label.
        return rest.trim().trim_end_matches(';').trim().to_string();
    }
    ident(rest)
}

/// `impl<'a> Trait for Type<T> where ...` -> `Type`.
fn impl_name(rest: &str) -> String {
    let head = rest.split('{').next().unwrap_or(rest);
    let head = head.split(" where ").next().unwrap_or(head);
    let target = match head.split(" for ").nth(1) {
        Some(t) => t,
        None => head,
    };
    let target = target.trim().trim_start_matches('<');
    // Strip a leading generic list, then take the base identifier.
    let target = match target.starts_with('\'') || target.starts_with(char::is_alphabetic) {
        true => target,
        false => target.trim_start_matches(|c: char| c != ' ').trim_start(),
    };
    target
        .split(['<', ' ', '{', '('])
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// The body deliberately includes the closing brace: folding a symbol should
/// leave its signature and nothing else, the way a folded heading leaves its
/// title.
fn extent(lines: &[&str], depths: &[i32], i: usize) -> (usize, (usize, usize)) {
    let start = depths[i];
    for (n, line) in lines.iter().enumerate().skip(i) {
        let opens = line.contains('{');
        let ends = line.contains(';');
        if opens {
            // Find the line where depth comes back to where it started.
            let close = (n + 1..lines.len())
                .find(|&k| depths[k] <= start)
                .unwrap_or(lines.len());
            return (n + 1, (n + 1, close));
        }
        if ends && !opens {
            return (n + 1, (n + 1, n + 1)); // `use x;`, `const A: u8 = 1;`
        }
    }
    (i + 1, (i + 1, i + 1))
}

/// Reads the *raw* lines: blanking replaced every comment with spaces, which is
/// exactly what this needs to see. Ordinary `//` comments count alongside `///`
/// — a reader wants the note someone left above a function, and the compiler's
/// opinion about which comments are documentation is not the reader's.
fn doc_above(raw: &[&str], i: usize) -> (usize, usize) {
    let mut start = i;
    while start > 0 {
        let prev = raw[start - 1].trim_start();
        let is_doc = prev.starts_with("//")
            || prev.starts_with('#')
            || prev.starts_with("/*")
            || prev.starts_with('*');
        if !is_doc {
            break;
        }
        start -= 1;
    }
    (start, i)
}

#[cfg(test)]
#[path = "rust_decl_tests.rs"]
mod tests;
