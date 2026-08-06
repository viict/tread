//! Pure platform *flavour* — everything above `sys` that has to know whether
//! this is Linux, macOS or Windows, with no syscall involved.
//!
//! `sys` answers "how do I talk to the OS". This module answers the two
//! questions that are just *conventions*: how a native path is spelled
//! ([`path`]) and where a user's files live ([`dirs`]). Both are expressed as
//! pure functions taking an explicit [`Platform`], so the Windows and macOS
//! rules are exercised by `cargo test` on a Linux host — the same discipline
//! `sys::abi` applies to constants, applied to userland conventions.
//!
//! Nothing here touches the filesystem, and nothing here contains `unsafe`.
//! Production callers pass [`Platform::HOST`]; tests pass all three.
#![deny(unsafe_code)]

pub mod dirs;
pub mod path;

#[cfg(test)]
#[path = "path_tests.rs"]
mod path_tests;

#[cfg(test)]
#[path = "dirs_tests.rs"]
mod dirs_tests;

/// Which operating system's conventions to apply.
///
/// Linux and macOS share path syntax and differ only in [`dirs`]; Windows
/// differs in both. Kept as one enum so a call site never has to hold two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    /// The platform this binary was compiled for. The only value production
    /// code passes; every test names its platform explicitly instead.
    pub const HOST: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Linux
    };

    pub const fn is_windows(self) -> bool {
        matches!(self, Platform::Windows)
    }

    /// Every platform this crate builds for, for exhaustive tests.
    #[cfg(test)]
    pub const ALL: [Platform; 3] = [Platform::Linux, Platform::Macos, Platform::Windows];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_matches_the_compilation_target() {
        // Belt and braces: the cfg chain above is the single place the crate
        // decides which conventions apply, so pin it against cfg! directly.
        assert_eq!(Platform::HOST.is_windows(), cfg!(windows));
        assert_eq!(
            Platform::HOST == Platform::Macos,
            cfg!(all(target_os = "macos", not(windows)))
        );
        assert!(Platform::ALL.contains(&Platform::HOST));
    }
}
