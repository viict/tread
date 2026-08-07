//! What may leave the reader, and what may never be handed to the operating
//! system (SPEC.md §"Opening a link outside the reader").
//!
//! A document is untrusted input, so this module is the whole risk surface of
//! opening a link: the decision is made from the URL's *scheme* alone, on an
//! allowlist, before anything platform-specific is reached. Everything here is
//! a pure function of a string — no filesystem, no process, no `Platform` —
//! which is what lets `cargo test` cover every refused scheme by name.
//!
//! The spawning half lives in [`crate::sys::browser`]; nothing in this file
//! knows which program runs, and nothing in that file knows which schemes are
//! allowed. What that file *does* know is that it will not spawn without a
//! [`Vetted`], and [`vetted`] below is the only thing in the crate that makes
//! one — so the precondition on the one `spawn` is carried by the type rather
//! than by a sentence in a doc comment.
//!
//! The URL *syntax* — what a scheme even is — lives lower still, in
//! [`crate::url`], where the renderer can read it without importing the
//! navigator. This module is the policy: the list, and the refusal.
#![deny(unsafe_code)]

use crate::sys::browser::Vetted;
use crate::url::{scheme_allowed, scheme_of};

/// Does this link leave the reader?
///
/// Re-exported from [`crate::url`]: one definition, read by the renderer that
/// colours the link and by the `Enter` that follows it, so a link cannot be
/// painted as one thing and opened as another.
pub use crate::url::is_external;

/// The only schemes ever handed to the system opener (SPEC.md §"Opening a link
/// outside the reader"). Lower-case; comparison is ASCII case-insensitive,
/// because URL schemes are.
pub const OPENABLE: [&str; 3] = ["http", "https", "mailto"];

/// Why a URL will not be opened. Carries the offending scheme so the status bar
/// can refuse it **by name**, which is the only way a reader can tell a
/// `javascript:` link in a document from a broken one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A scheme that is not on the allowlist, as spelled in the document.
    Scheme(String),
    /// No scheme at all: not something that leaves the reader.
    NotExternal,
}

impl Refused {
    /// The status-bar line for this refusal.
    pub fn message(&self, url: &str) -> String {
        match self {
            Refused::Scheme(s) => {
                format!(
                    "refusing to open a {s}: link \u{2014} only http, https and mailto \u{2014} {url}"
                )
            }
            Refused::NotExternal => format!("not an external link: {url}"),
        }
    }
}

/// May this URL be handed to the system opener?
///
/// `Ok` carries the lower-cased scheme, which is the one fact the caller may
/// want to report; `Err` names what was refused. The URL itself is returned to
/// nobody in a modified form: whatever the document wrote is what would be
/// passed, as a single argument, so no re-quoting or re-encoding happens
/// anywhere on this path.
pub fn openable(url: &str) -> Result<String, Refused> {
    let scheme = match scheme_of(url) {
        Some(s) => s.to_ascii_lowercase(),
        None => return Err(Refused::NotExternal),
    };
    match scheme_allowed(&scheme, &OPENABLE) {
        true => Ok(scheme),
        false => Err(Refused::Scheme(scheme)),
    }
}

/// The same decision, as the token [`crate::sys::browser::open`] requires.
///
/// This is the *only* constructor of a [`Vetted`] reachable from anywhere in the
/// crate, which is what makes SPEC.md §"Opening a link outside the reader"
/// structural instead of a convention: a second caller that wanted to spawn an
/// opener could not, because it has no way to build the argument without coming
/// through here and naming [`OPENABLE`].
///
/// The `Err` arm after [`openable`] has already said yes is unreachable — both
/// ask [`scheme_allowed`] about the same list — and it is still written as a
/// `Result` rather than an `expect`, because "unreachable" is the kind of claim
/// that stops being true quietly.
pub fn vetted(url: &str) -> Result<Vetted<'_>, Refused> {
    let scheme = openable(url)?;
    Vetted::new(url, &OPENABLE).ok_or(Refused::Scheme(scheme))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_allowed_schemes_open() {
        for url in [
            "http://example.com/x",
            "https://example.com/x?a=b#c",
            "mailto:someone@example.com",
        ] {
            assert!(openable(url).is_ok(), "{url} should open");
        }
    }

    /// Schemes are case-insensitive, in the allowlist and in the refusal.
    #[test]
    fn scheme_matching_ignores_case() {
        assert_eq!(openable("HTTPS://x").unwrap(), "https");
        assert_eq!(openable("MailTo:a@b").unwrap(), "mailto");
        assert_eq!(
            openable("JavaScript:alert(1)"),
            Err(Refused::Scheme("javascript".to_string()))
        );
    }

    /// Every scheme SPEC.md §"Opening a link outside the reader" names, plus
    /// the neighbours a hostile document would reach for. Each is refused *by
    /// name*, so the status bar can say which.
    #[test]
    fn every_other_scheme_is_refused_by_name() {
        for (url, scheme) in [
            ("file:///etc/passwd", "file"),
            ("javascript:alert(document.cookie)", "javascript"),
            ("vbscript:msgbox(1)", "vbscript"),
            ("data:text/html;base64,PHNjcmlwdD4=", "data"),
            ("ftp://example.com/x", "ftp"),
            ("ssh://host/x", "ssh"),
            ("smb://host/share", "smb"),
            ("chrome://settings", "chrome"),
            ("ms-msdt:/id", "ms-msdt"),
            ("search-ms:query=x", "search-ms"),
            ("tel:+15550100", "tel"),
            ("about:blank", "about"),
        ] {
            let err = openable(url).unwrap_err();
            assert_eq!(err, Refused::Scheme(scheme.to_string()), "{url}");
            let msg = err.message(url);
            assert!(msg.contains(scheme), "{msg} must name the scheme");
            assert!(msg.contains(url), "{msg} must show the url");
        }
    }

    #[test]
    fn a_path_is_not_external_and_is_not_openable() {
        for url in [
            "models/A.md",
            "./x.md",
            "../up.md",
            "#anchor",
            "",
            "a/b:c",
            "3com:x",
        ] {
            assert!(!is_external(url), "{url} is not external");
            assert_eq!(openable(url), Err(Refused::NotExternal), "{url}");
        }
    }

    #[test]
    fn external_covers_refused_schemes_too() {
        for url in ["https://x", "javascript:x", "file:///x", "mailto:a@b"] {
            assert!(is_external(url), "{url} leaves the reader");
        }
    }

    /// The allowlist is the interesting invariant, so pin its contents rather
    /// than only its behaviour: a fourth entry has to be a deliberate edit.
    #[test]
    fn the_allowlist_is_exactly_three_schemes() {
        assert_eq!(OPENABLE, ["http", "https", "mailto"]);
    }
}
