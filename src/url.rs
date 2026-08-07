//! URL *syntax*: the scheme of a link destination, and whether it has one.
//!
//! This is a leaf module — no imports, no platform, no state — and it exists to
//! be one. Three layers need the same answer to "does this link leave the
//! reader": the renderer, which colours it (SPEC.md §Navigation, "coloured
//! apart from links that stay inside the reader"); `nav`, which resolves it;
//! and `sys::browser`, which will not spawn an opener for a URL whose scheme no
//! allowlist admitted. When the predicate lived in `nav`, `render` imported the
//! navigator to paint a span, which points the dependency the wrong way for a
//! module whose whole job is AST + width -> `Vec<Line>`. Here every one of them
//! reads *down*.
//!
//! What is deliberately **not** here is policy. Which schemes may be handed to
//! the operating system is [`crate::nav::external::OPENABLE`]'s business, and
//! the two are different questions: `javascript:` is external — and must be
//! painted as external before `Enter` is pressed — *and* is refused.
#![deny(unsafe_code)]

/// The URL scheme of `raw`, if it has one.
///
/// `a/b:c` is not a scheme — a colon only counts before the first slash, or
/// `models/DNS:MODEL.md` would be a link to a `models` scheme. RFC 3986 spells
/// the grammar `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`, which is why
/// `3com:x` has no scheme either.
pub fn scheme_of(raw: &str) -> Option<&str> {
    let end = raw.find(':')?;
    if end == 0 || raw[..end].contains('/') {
        return None;
    }
    let mut chars = raw[..end].chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        Some(&raw[..end])
    } else {
        None
    }
}

/// Does this link leave the reader?
///
/// Any link with a scheme does: what the reader can show is a *path* inside the
/// corpus, and a scheme is precisely how a document says "not that". Wider than
/// the opener's allowlist on purpose — see the module docs.
pub fn is_external(url: &str) -> bool {
    scheme_of(url).is_some()
}

/// Is `scheme` on `allowed`? ASCII case-insensitive, because URL schemes are.
///
/// The comparison itself, so the allowlist above and the spawn guard below
/// cannot disagree about what "on the list" means.
pub fn scheme_allowed(scheme: &str, allowed: &[&str]) -> bool {
    allowed.iter().any(|a| a.eq_ignore_ascii_case(scheme))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_is_what_precedes_the_first_colon() {
        assert_eq!(scheme_of("https://x"), Some("https"));
        assert_eq!(scheme_of("mailto:a@b"), Some("mailto"));
        assert_eq!(scheme_of("ms-msdt:/id"), Some("ms-msdt"));
        assert_eq!(scheme_of("HTTPS://x"), Some("HTTPS"));
    }

    /// The cases a corpus path would otherwise be mistaken for a URL.
    #[test]
    fn a_path_has_no_scheme() {
        for raw in [
            "models/A.md",
            "models/DNS:MODEL.md",
            "./x",
            "../up.md",
            "#anchor",
            "",
            ":leading",
            "3com:x",
            "a b:c",
        ] {
            assert_eq!(scheme_of(raw), None, "{raw}");
            assert!(!is_external(raw), "{raw}");
        }
    }

    #[test]
    fn external_covers_refused_schemes_too() {
        for url in ["https://x", "javascript:x", "file:///x", "mailto:a@b"] {
            assert!(is_external(url), "{url} leaves the reader");
        }
    }

    #[test]
    fn allowlist_matching_ignores_case() {
        let list = ["http", "https", "mailto"];
        assert!(scheme_allowed("https", &list));
        assert!(scheme_allowed("HTTPS", &list));
        assert!(scheme_allowed("MailTo", &list));
        assert!(!scheme_allowed("javascript", &list));
        assert!(!scheme_allowed("", &list));
    }
}
