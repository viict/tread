//! Getting yanked text out of the process: OSC 52 under a multiplexer, a
//! cache-file fallback, and the status-bar wording for both.
//!
//! OSC 52 is the only clipboard channel a zero-dependency reader has, and it is
//! refused by plenty of terminals. Every yank is therefore *also* written to
//! `~/.cache/mdr/last-yank.txt` and the status bar says so, so a copy is never
//! silently lost.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use crate::term::{base64, osc52_sequence, ClipReport, MAX_CLIPBOARD_BYTES};

/// Terminal multiplexer wrapping required around an OSC 52 sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mux {
    None,
    /// `\x1bPtmux;` ... `\x1b\\` with every inner ESC doubled.
    Tmux,
    /// GNU screen: DCS-wrapped, and chunked because screen's DCS buffer is
    /// small (it drops the whole string when it overflows).
    Screen,
}

/// screen truncates DCS strings beyond roughly 1 KB; stay well under.
pub const SCREEN_CHUNK: usize = 768;

/// Which wrapping the current environment needs. `$TMUX` wins over `$STY` and
/// `TERM=screen*`, because tmux also advertises itself as a screen terminal.
pub fn detect_mux(tmux: bool, sty: bool, term: Option<&str>) -> Mux {
    if tmux {
        return Mux::Tmux;
    }
    if sty || term.map(|t| t.starts_with("screen")).unwrap_or(false) {
        return Mux::Screen;
    }
    Mux::None
}

/// Read the environment and classify it.
pub fn mux_from_env() -> Mux {
    detect_mux(
        std::env::var_os("TMUX").is_some(),
        std::env::var_os("STY").is_some(),
        std::env::var("TERM").ok().as_deref(),
    )
}

/// The bytes to write for `text`, plus what actually made it onto the
/// clipboard. Oversized payloads are cut on a char boundary and reported rather
/// than emitted as a sequence the terminal will mangle.
pub fn clipboard_sequence(text: &str, mux: Mux) -> (String, ClipReport) {
    match mux {
        Mux::None => osc52_sequence(text, false),
        Mux::Tmux => osc52_sequence(text, true),
        Mux::Screen => screen_sequence(text),
    }
}

fn screen_sequence(text: &str) -> (String, ClipReport) {
    let mut cut = text.len().min(MAX_CLIPBOARD_BYTES);
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let report = ClipReport { sent: cut, truncated: cut < text.len() };
    let inner = format!("\x1b]52;c;{}\x07", base64(&text.as_bytes()[..cut]));
    let mut out = String::with_capacity(inner.len() + 64);
    // `inner` is pure ASCII (escape, base64, BEL), so byte chunking is safe.
    let bytes = inner.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let end = (at + SCREEN_CHUNK).min(bytes.len());
        out.push_str("\x1bP");
        out.push_str(&inner[at..end]);
        out.push_str("\x1b\\");
        at = end;
    }
    (out, report)
}

// ---------------------------------------------------------------------------
// Fallback file
// ---------------------------------------------------------------------------

/// `$XDG_CACHE_HOME/mdr/last-yank.txt`, else `$HOME/.cache/mdr/last-yank.txt`.
pub fn fallback_path(xdg_cache: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match (xdg_cache, home) {
        (Some(x), _) if !x.as_os_str().is_empty() => x.to_path_buf(),
        (_, Some(h)) if !h.as_os_str().is_empty() => h.join(".cache"),
        _ => return None,
    };
    Some(base.join("mdr").join("last-yank.txt"))
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

/// Write the full (never truncated) text to the cache file. Returns the path on
/// success; a failure is not fatal, the clipboard may still have worked.
pub fn write_fallback(text: &str) -> Option<PathBuf> {
    let path = fallback_path(env_path("XDG_CACHE_HOME").as_deref(), env_path("HOME").as_deref())?;
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&path, text.as_bytes()).ok()?;
    Some(path)
}

/// Shorten a path under `$HOME` to `~/...` for the status bar.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(h) = home {
        if let Ok(rest) = path.strip_prefix(h) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

// ---------------------------------------------------------------------------
// Wording
// ---------------------------------------------------------------------------

/// The transient status line for a yank. `report` is `None` when the terminal
/// write itself failed.
pub fn yank_message(what: &str, report: Option<ClipReport>, fallback: Option<&str>) -> String {
    let saved = fallback.map(|p| format!("  \u{b7}  saved to {p}"));
    match report {
        Some(r) if r.truncated => match fallback {
            Some(p) => format!(
                "yanked {what} \u{2014} clipboard truncated to {} bytes; full text in {p}",
                r.sent
            ),
            None => format!(
                "yanked {what} \u{2014} clipboard truncated to {} bytes",
                r.sent
            ),
        },
        Some(_) => format!("yanked {what}{}", saved.unwrap_or_default()),
        None => match fallback {
            Some(p) => format!("clipboard refused the copy \u{2014} {what} saved to {p}"),
            None => format!("could not copy {what}: no clipboard and no cache file"),
        },
    }
}

/// `3 lines` / `1 line`, for the messages above.
pub fn line_count(n: usize) -> String {
    match n {
        1 => "1 line".to_string(),
        _ => format!("{n} lines"),
    }
}
