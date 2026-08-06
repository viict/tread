//! Where a user's files live, per platform. Pure: the environment is read once
//! into [`Env`] and every rule below is a function of that value.
//!
//! Two locations matter to `tread`:
//!
//! * the **home directory**, only ever used to shorten a path for display;
//! * the **yank fallback file**, which every yank is written to so an OSC 52
//!   refusal never loses a copy silently (SPEC.md §Keybindings).
//!
//! The chosen locations, and why:
//!
//! | Platform | Yank fallback |
//! | --- | --- |
//! | Linux | `$XDG_CACHE_HOME/tread/last-yank.txt`, else `$HOME/.cache/tread/…` |
//! | macOS | `$XDG_CACHE_HOME/tread/…`, else `$HOME/Library/Caches/tread/…` |
//! | Windows | `%LOCALAPPDATA%\tread\…`, else `%TEMP%\tread\…`, else `%USERPROFILE%\AppData\Local\tread\…` |
//!
//! macOS uses `~/Library/Caches` rather than `~/.cache`: it is the documented
//! location, it is what the OS's own cache eviction and every backup exclusion
//! rule already know about, and a dotfile cache under `$HOME` is invisible to
//! all of that. An explicitly set `XDG_CACHE_HOME` still wins there, because a
//! user who exports it has said where their caches go.
//!
//! Windows has no `$HOME`; `%USERPROFILE%` is the equivalent, and
//! `%LOCALAPPDATA%` (not `%APPDATA%`) is the roaming-excluded per-machine
//! store, which is what a scratch file wants. `%TEMP%` is the fallback for a
//! stripped service environment.
#![deny(unsafe_code)]

use std::path::PathBuf;

use super::path;
use super::Platform;

/// The cache subdirectory and file the yank fallback is written to.
pub const YANK_RELATIVE: &str = "tread/last-yank.txt";

/// Every environment variable this module consults, read once.
///
/// Empty values count as unset: an exported-but-empty `HOME=` would otherwise
/// make the fallback file land at the filesystem root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub home: Option<String>,
    pub userprofile: Option<String>,
    pub xdg_cache_home: Option<String>,
    pub local_app_data: Option<String>,
    pub temp: Option<String>,
}

impl Env {
    /// Read the real process environment. The only impure function here.
    ///
    /// A value that is not valid UTF-8 counts as unset rather than being
    /// lossily rewritten: the rules below build a path out of it, and a `HOME`
    /// with a stray byte would otherwise send the yank fallback to a directory
    /// that is *almost* the user's. Not writing the file is the safe outcome —
    /// it is a best-effort copy channel, and the status bar says so.
    pub fn from_process() -> Env {
        let get = |k: &str| {
            std::env::var_os(k)
                .and_then(|v| v.into_string().ok())
                .filter(|v| !v.is_empty())
        };
        Env {
            home: get("HOME"),
            userprofile: get("USERPROFILE"),
            xdg_cache_home: get("XDG_CACHE_HOME"),
            local_app_data: get("LOCALAPPDATA"),
            temp: get("TEMP").or_else(|| get("TMP")),
        }
    }

    /// Build one from `(key, value)` pairs; empty values are dropped, exactly
    /// as [`Env::from_process`] drops them.
    #[cfg(test)]
    pub fn of(pairs: &[(&str, &str)]) -> Env {
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
                .filter(|v| !v.is_empty())
        };
        Env {
            home: get("HOME"),
            userprofile: get("USERPROFILE"),
            xdg_cache_home: get("XDG_CACHE_HOME"),
            local_app_data: get("LOCALAPPDATA"),
            temp: get("TEMP").or_else(|| get("TMP")),
        }
    }
}

/// The user's home directory: `$HOME` on unix, `%USERPROFILE%` on Windows
/// (with `$HOME` still honoured there, because MSYS/Git-Bash sets it and a user
/// in that shell means it).
pub fn home(p: Platform, env: &Env) -> Option<String> {
    match p {
        Platform::Windows => env.userprofile.clone().or_else(|| env.home.clone()),
        _ => env.home.clone(),
    }
}

/// The directory caches go in, before `tread/` is appended.
fn cache_root(p: Platform, env: &Env) -> Option<String> {
    if let Some(x) = &env.xdg_cache_home {
        return Some(x.clone());
    }
    match p {
        Platform::Linux => Some(path::join(p, env.home.as_deref()?, ".cache")?),
        Platform::Macos => Some(path::join(p, env.home.as_deref()?, "Library/Caches")?),
        Platform::Windows => windows_cache_root(env),
    }
}

fn windows_cache_root(env: &Env) -> Option<String> {
    if let Some(l) = &env.local_app_data {
        return Some(l.clone());
    }
    if let Some(t) = &env.temp {
        return Some(t.clone());
    }
    let up = env.userprofile.as_deref()?;
    path::join(Platform::Windows, up, "AppData/Local")
}

/// Absolute path of the yank fallback file, or `None` when the environment
/// says nothing about where the user's files are.
pub fn yank_fallback(p: Platform, env: &Env) -> Option<PathBuf> {
    let root = cache_root(p, env)?;
    let full = path::join(p, &root, YANK_RELATIVE)?;
    Some(PathBuf::from(full))
}

/// Shorten a path under the home directory for the status bar.
///
/// `~` is a unix shell convention, so Windows gets the path verbatim: `~\…`
/// means nothing to `cmd`, PowerShell or Explorer, and a status bar that shows
/// a path the user cannot paste back is worse than a long one.
pub fn display_path(p: Platform, path_str: &str, home_dir: Option<&str>) -> String {
    if p.is_windows() {
        return path_str.to_string();
    }
    let h = match home_dir {
        Some(h) if !h.is_empty() => h,
        _ => return path_str.to_string(),
    };
    if !path::contains(p, h, path_str) || path::same(p, h, path_str) {
        return path_str.to_string();
    }
    format!("~{}{}", path::sep(p), path::rel_to(p, h, path_str))
}
