//! Handing a URL to the system's opener: the one place in the crate that knows
//! *which program* opens a link (SPEC.md §"Opening a link outside the reader").
//!
//! Compiled on **every** target, exactly like [`crate::sys::win_abi`], because
//! the interesting part is a table — which program and which argument vector
//! each OS wants — and a table can be pinned by `cargo test` on the Linux host
//! for the OSes that host is not. [`argv`] is that table, a pure function; only
//! [`open`] touches the OS, and it is the only item here that is `cfg`-flavoured
//! (through [`HOST`]).
//!
//! Two rules from SPEC.md are structural rather than incidental:
//!
//! * **Never through a shell.** The URL is one element of an argument vector
//!   handed to [`std::process::Command`], which on unix `exec`s directly and on
//!   Windows quotes for the `CreateProcess` convention. No command *string* is
//!   built anywhere in this file — there is nothing for a shell to re-interpret,
//!   which is why `cmd /c start` (whose `&`, `|` and `^` are metacharacters) is
//!   not used on Windows.
//! * **The reader never waits.** The child is spawned with all three standard
//!   streams closed and is never waited on, so a browser that takes four seconds
//!   to appear, or writes to stderr, cannot stall or corrupt the alternate
//!   screen. Finished children are reaped opportunistically on the next call, so
//!   a session that opens links all afternoon leaves no zombies behind.
//!
//! *Which* schemes are allowed is not this module's question — that list lives
//! in [`crate::nav::external`], because it is policy — but that the check
//! happened is. [`open`] does not take a `&str`: it takes a [`Vetted`], whose
//! only constructor is handed an allowlist and refuses a URL whose scheme is not
//! on it. A future second caller therefore cannot reach the one `spawn` in the
//! crate with an unvetted scheme; it would not compile.
#![deny(unsafe_code)]

use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::url::{scheme_allowed, scheme_of};

/// The opener conventions this crate knows. Kept as its own enum rather than
/// reusing `plat::Platform`: `plat` sits *above* `sys` (it is a consumer of it),
/// and `sys` may not depend upwards. Two variants of one table is the price;
/// [`HOST`] is derived from `cfg!` in one place, and the host test pins that
/// against the same `cfg!`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    /// Anything with an XDG desktop: `xdg-open`.
    Xdg,
    Macos,
    Windows,
}

impl Desktop {
    /// Every convention tabulated here, for exhaustive tests.
    #[cfg(test)]
    pub const ALL: [Desktop; 3] = [Desktop::Xdg, Desktop::Macos, Desktop::Windows];
}

/// This build's convention.
pub const HOST: Desktop = if cfg!(windows) {
    Desktop::Windows
} else if cfg!(target_os = "macos") {
    Desktop::Macos
} else {
    Desktop::Xdg
};

/// One process to run: a program name and the arguments it gets, in order.
///
/// A vector, never a string. Callers pass it straight to `Command`; the tests
/// assert on it without running anything, which is the only way the Windows and
/// macOS spellings are checked at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argv {
    pub program: &'static str,
    pub args: Vec<String>,
}

/// The opener invocation for `url` on `desktop`.
///
/// `url` is placed verbatim as its own argument and is never concatenated into
/// anything. It is also never the *first* argument on any platform where that
/// could matter: every URL that reaches here has passed the allowlist, so it
/// starts with `http`, `https` or `mailto`, and cannot be mistaken for an option
/// by the program being run.
pub fn argv(desktop: Desktop, url: &str) -> Argv {
    match desktop {
        Desktop::Xdg => Argv {
            program: "xdg-open",
            args: vec![url.to_string()],
        },
        Desktop::Macos => Argv {
            program: "open",
            args: vec![url.to_string()],
        },
        // `url.dll,FileProtocolHandler` is one argument — the module and its
        // entry point — and the URL is the next. This is the documented
        // shell-free way to hand a URL to the Windows shell; `cmd /c start` is
        // the one that would need quoting rules to be safe.
        Desktop::Windows => Argv {
            program: "rundll32",
            args: vec![
                "url.dll,FileProtocolHandler".to_string(),
                url.to_string(),
            ],
        },
    }
}

/// Why a URL did not reach a browser. Never fatal: SPEC.md asks for a
/// status-bar message, not an error exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The opener is not installed (a headless box with no `xdg-open`).
    Missing(&'static str),
    /// It exists but could not be started; carries the OS's reason.
    Failed { program: &'static str, why: String },
}

impl OpenError {
    /// The status-bar line for this failure.
    pub fn message(&self) -> String {
        match self {
            OpenError::Missing(p) => format!("no system opener: {p} is not installed"),
            OpenError::Failed { program, why } => format!("could not run {program}: {why}"),
        }
    }
}

/// Children spawned and not yet reaped.
///
/// Dropping a `Child` does not wait for it, so on unix each opener would linger
/// as a zombie until the reader exits. They are polled — never waited on — at
/// the start of the next open, which is the only moment their existence is
/// interesting and is off the paint path entirely.
static SPAWNED: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());

/// A URL whose scheme some allowlist admitted: the only thing [`open`] accepts.
///
/// The field is private and [`Vetted::new`] is the only constructor, so the
/// precondition SPEC.md §"Opening a link outside the reader" states is carried
/// by the type instead of by this sentence. Which schemes are on the list is
/// still decided above `sys` — the caller passes it — so nothing here has an
/// opinion about `javascript:`, only about whether *somebody* checked.
///
/// Borrows rather than owns: the pager already holds the URL string, and copying
/// it would be a second place the bytes could be edited between the check and
/// the spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vetted<'a> {
    url: &'a str,
}

impl<'a> Vetted<'a> {
    /// `Some` when `url` has a scheme and that scheme is on `allowed`
    /// (ASCII case-insensitively — URL schemes are). `None` otherwise, which
    /// includes a URL with no scheme at all: a corpus path is not something to
    /// hand to the operating system.
    pub fn new(url: &'a str, allowed: &[&str]) -> Option<Vetted<'a>> {
        let scheme = scheme_of(url)?;
        scheme_allowed(scheme, allowed).then_some(Vetted { url })
    }

    /// The URL, exactly as the document wrote it. Nothing on this path re-quotes
    /// or re-encodes it.
    pub fn url(&self) -> &'a str {
        self.url
    }
}

/// Hand a vetted `url` to this platform's opener and return immediately.
///
/// Returns the program that was started, for the status bar. There is no
/// `&str` overload on purpose: see [`Vetted`].
pub fn open(url: Vetted<'_>) -> Result<&'static str, OpenError> {
    let Argv { program, args } = argv(HOST, url.url());
    reap();
    let child = Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(c) => {
            if let Ok(mut held) = SPAWNED.lock() {
                held.push(c);
            }
            Ok(program)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Err(OpenError::Missing(program)),
        Err(e) => Err(OpenError::Failed {
            program,
            why: e.to_string(),
        }),
    }
}

/// Drop every child that has already exited. Non-blocking: a browser still
/// running is left exactly where it is.
fn reap() {
    if let Ok(mut held) = SPAWNED.lock() {
        held.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_)) | Err(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The argument vector for every platform, asserted and never run. This is
    /// the whole test of the Windows and macOS spellings — there is no machine
    /// in this loop that could execute either.
    #[test]
    fn each_platform_gets_its_documented_argv() {
        let url = "https://example.com/a?b=c";
        assert_eq!(
            argv(Desktop::Xdg, url),
            Argv {
                program: "xdg-open",
                args: vec![url.to_string()]
            }
        );
        assert_eq!(
            argv(Desktop::Macos, url),
            Argv {
                program: "open",
                args: vec![url.to_string()]
            }
        );
        assert_eq!(
            argv(Desktop::Windows, url),
            Argv {
                program: "rundll32",
                args: vec![
                    "url.dll,FileProtocolHandler".to_string(),
                    url.to_string()
                ]
            }
        );
    }

    /// `cmd /c start` is what this must never become, on any platform.
    #[test]
    fn no_platform_runs_a_shell() {
        for d in Desktop::ALL {
            let a = argv(d, "https://x");
            for forbidden in ["cmd", "cmd.exe", "sh", "bash", "start", "powershell"] {
                assert_ne!(a.program, forbidden, "{d:?} must not run a shell");
                assert!(
                    !a.args.iter().any(|s| s == forbidden),
                    "{d:?} must not pass {forbidden}"
                );
            }
            assert!(!a.program.contains(' '), "{d:?}: program is one word");
        }
    }

    /// A hostile URL survives as exactly one argument, byte for byte: nothing
    /// on this path quotes, escapes, splits or concatenates it, so there is
    /// nothing for a shell (which is never involved) to re-read.
    #[test]
    fn a_hostile_url_stays_one_untouched_argument() {
        let nasty = "https://x/?a=1&b=2|c;rm -rf /`whoami`$(id)\"'\n\\%windir%^&";
        for d in Desktop::ALL {
            let a = argv(d, nasty);
            assert_eq!(
                a.args.iter().filter(|s| s.contains(nasty)).count(),
                1,
                "{d:?}: the url appears once"
            );
            assert!(a.args.contains(&nasty.to_string()), "{d:?}: verbatim");
            // The URL is always the last argument, and nothing was appended to
            // it or prefixed onto it.
            assert_eq!(a.args.last().map(String::as_str), Some(nasty));
        }
    }

    /// Every allowed scheme reaches the same one-argument shape.
    #[test]
    fn mailto_is_passed_like_any_other_url() {
        for d in Desktop::ALL {
            let a = argv(d, "mailto:someone@example.com?subject=hi there");
            assert_eq!(
                a.args.last().map(String::as_str),
                Some("mailto:someone@example.com?subject=hi there")
            );
        }
    }

    /// The structural half of SPEC.md §"Opening a link outside the reader":
    /// `open` cannot be reached without a scheme that some allowlist admitted,
    /// and this is what "admitted" means.
    #[test]
    fn only_a_scheme_on_the_list_can_be_vetted() {
        let list = ["http", "https", "mailto"];
        for url in ["http://x", "https://x/a?b#c", "mailto:a@b", "HTTPS://x"] {
            assert_eq!(Vetted::new(url, &list).map(|v| v.url()), Some(url), "{url}");
        }
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,x",
            "models/A.md",
            "#anchor",
            "",
        ] {
            assert!(Vetted::new(url, &list).is_none(), "{url} must not vet");
        }
    }

    /// An empty allowlist admits nothing, so a caller that forgot its list gets
    /// no token rather than a default-open.
    #[test]
    fn an_empty_allowlist_vets_nothing() {
        assert!(Vetted::new("https://example.com", &[]).is_none());
    }

    /// The URL reaches `argv` byte for byte through the token.
    #[test]
    fn vetting_does_not_touch_the_url() {
        let url = "https://x/?a=1&b=2|c;\"'`$( )";
        let v = Vetted::new(url, &["https"]).expect("https is on the list");
        assert_eq!(v.url(), url);
        assert_eq!(argv(HOST, v.url()).args.last().map(String::as_str), Some(url));
    }

    #[test]
    fn host_matches_the_compilation_target() {
        assert_eq!(HOST == Desktop::Windows, cfg!(windows));
        assert_eq!(
            HOST == Desktop::Macos,
            cfg!(all(target_os = "macos", not(windows)))
        );
        assert!(Desktop::ALL.contains(&HOST));
    }

    /// The two failures a reader can actually hit both read as English and name
    /// the program, because "nothing happened" is the worst possible answer.
    #[test]
    fn failures_name_the_program() {
        let missing = OpenError::Missing("xdg-open").message();
        assert!(missing.contains("xdg-open"), "{missing}");
        assert!(missing.contains("not installed"), "{missing}");
        let failed = OpenError::Failed {
            program: "open",
            why: "permission denied".to_string(),
        }
        .message();
        assert!(failed.contains("open") && failed.contains("permission denied"), "{failed}");
    }
}
