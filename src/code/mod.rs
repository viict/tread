//! Reading code: the language grammars (SPEC.md §Code).
//!
//! Pure, like `md`, `csv` and `json` — this module knows nothing about
//! rendering, files or the terminal. It answers one question per language:
//! *where are the comments and the declarations*. `source::code` turns that
//! answer into rows.
//!
//! These are not parsers. A reader needs to find declarations and the comments
//! attached to them, which is a lexer plus a recognizer over the token stream —
//! and stopping there is what makes a language a few hundred lines instead of a
//! dependency. Nothing here understands types, resolves a name, or expands a
//! macro, and SPEC.md §Code says so out loud.
#![deny(unsafe_code)]
// The `Source` that will call this is the next roll; until then only the tests
// drive it. Drop this allow once `source::code` is the one asking.
#![allow(dead_code)]

pub mod decl;
pub mod java;
pub mod java_decl;
pub mod py;
pub mod py_decl;
pub mod rust;
pub mod rust_decl;
pub mod scan;
pub mod ts;
pub mod ts_decl;

/// What a symbol *is*. Drives the outline's styling and, later, filtering.
///
/// Deliberately a flat list across languages rather than one enum per language:
/// a reader scanning a file wants "this is a type, that is a function", and the
/// spelling the language uses for it is not the interesting part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `fn`, and a method inside an `impl`.
    Func,
    /// `struct`, `enum`, `union`.
    Type,
    /// `trait`.
    Trait,
    /// `class`.
    Class,
    /// `interface`.
    Interface,
    /// `impl X` / `impl T for X` — a container, not a leaf.
    Impl,
    /// `mod`.
    Mod,
    /// `const`, `static`.
    Const,
    /// `type` alias.
    Alias,
    /// `use` / `import`: not a definition, but the thing a reader follows.
    Import,
    /// `macro_rules!`.
    Macro,
}

impl Kind {
    /// One-word label for the outline and the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Func => "fn",
            Kind::Type => "type",
            Kind::Trait => "trait",
            Kind::Class => "class",
            Kind::Interface => "interface",
            Kind::Impl => "impl",
            Kind::Mod => "mod",
            Kind::Const => "const",
            Kind::Alias => "alias",
            Kind::Import => "use",
            Kind::Macro => "macro",
        }
    }
}

/// One declaration, in source-line terms (0-based, end-exclusive).
///
/// Every range is a range of *lines*, not bytes: the views are line-oriented,
/// the collapse tree is line-oriented, and keeping byte offsets here would mean
/// converting them in three places instead of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: Kind,
    /// The bare name — `sniff`.
    pub name: String,
    /// The name qualified by its container — `DirSource::open`. The fold id and
    /// the anchor a `#link` targets, so it must be unique within the file;
    /// [`disambiguate`] guarantees that.
    pub path: String,
    /// Nesting depth, 0 = top level. Becomes the outline level.
    pub depth: u8,
    /// Doc comments and attributes immediately above, shown verbatim.
    pub doc: (usize, usize),
    /// The signature: the declaration's own lines, up to and including the line
    /// that opens the body.
    pub sig: (usize, usize),
    /// The body, which is what folds away. Empty when there is none — a `use`,
    /// or a `const` on one line.
    pub body: (usize, usize),
}

impl Symbol {
    /// Lines this symbol owns in total, doc comment included.
    pub fn span(&self) -> (usize, usize) {
        (self.doc.0.min(self.sig.0), self.body.1.max(self.sig.1))
    }

    /// How many lines folding this symbol would hide.
    pub fn hidden(&self) -> usize {
        self.body.1.saturating_sub(self.body.0)
    }
}

/// Make every `path` unique by suffixing repeats, so a fold id names exactly
/// one symbol.
///
/// Two `fn new` in two `impl` blocks already differ by container, but a file
/// with two `impl` blocks for the same type — or a `#[cfg]` pair declaring the
/// same function twice — genuinely repeats. The fold state and `#anchor` links
/// are keyed by this string, so a duplicate would fold two places at once.
pub fn disambiguate(symbols: &mut [Symbol]) {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for s in symbols.iter_mut() {
        match seen.iter_mut().find(|(p, _)| *p == s.path) {
            Some((_, n)) => {
                *n += 1;
                s.path = format!("{}-{}", s.path, n);
            }
            None => seen.push((s.path.clone(), 1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(path: &str) -> Symbol {
        Symbol {
            kind: Kind::Func,
            name: path.into(),
            path: path.into(),
            depth: 0,
            doc: (0, 0),
            sig: (0, 1),
            body: (1, 1),
        }
    }

    #[test]
    fn a_repeated_path_is_suffixed_so_a_fold_id_names_one_symbol() {
        let mut v = vec![sym("new"), sym("open"), sym("new"), sym("new")];
        disambiguate(&mut v);
        let paths: Vec<&str> = v.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, vec!["new", "open", "new-2", "new-3"]);
    }

    #[test]
    fn a_symbols_span_covers_its_doc_comment_and_body() {
        let s = Symbol {
            doc: (3, 5),
            sig: (5, 6),
            body: (6, 20),
            ..sym("f")
        };
        assert_eq!(s.span(), (3, 20));
        assert_eq!(s.hidden(), 14);
    }
}
