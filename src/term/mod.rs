//! Safe terminal layer over [`crate::sys`]. Contains no `unsafe` code.
//!
//! * [`Term`] — RAII raw-mode guard, alternate screen, cursor, single-write
//!   frame flush, OSC 52 clipboard.
//! * [`Frame`] — the per-frame buffer.
//! * [`Style`] — ANSI style value type with a diffing writer.
//!
//! **The mouse is never captured.** No `?1000h` / `?1002h` / `?1006h` sequence
//! is emitted anywhere in this module tree, so terminal-native click-drag
//! selection keeps working at all times (SPEC.md §Hard constraints #5).
#![deny(unsafe_code)]

mod clip;
mod frame;
mod style;

// The renderer (a later roll) is the main consumer of these re-exports; in a
// binary crate `pub use` alone does not count as a use.
#[allow(unused_imports)]
pub use clip::{base64, osc52_sequence, ClipReport, MAX_CLIPBOARD_BYTES};
#[allow(unused_imports)]
pub use frame::{is_osc_safe, Frame};
#[allow(unused_imports)]
pub use style::{write_transition, Style, BOLD, DIM, ITALIC, REVERSE, STRIKE, UNDERLINE};

use crate::sys::{self, Fd, ReadOutcome, SavedTermios};
use std::sync::Mutex;

const ENTER_ALT: &str = "\x1b[?1049h";
const LEAVE_ALT: &str = "\x1b[?1049l";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const SGR_RESET: &str = "\x1b[0m";

// ---------------------------------------------------------------------------
// Emergency restore (used by main's panic hook)
// ---------------------------------------------------------------------------

struct Emergency {
    in_fd: Fd,
    out_fd: Fd,
    saved: SavedTermios,
    alt: bool,
}

static EMERGENCY: Mutex<Option<Emergency>> = Mutex::new(None);

fn lock_emergency() -> std::sync::MutexGuard<'static, Option<Emergency>> {
    EMERGENCY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Put the terminal back into a usable state: leave the alternate screen, show
/// the cursor, reset SGR, restore termios.
///
/// Safe to call from a panic hook and safe to call repeatedly. Does nothing
/// when no [`Term`] is live. `main` must install a panic hook that calls this,
/// because the release profile uses `panic = "abort"` and therefore never runs
/// `Term`'s `Drop`.
pub fn emergency_restore() {
    let taken = lock_emergency().take();
    if let Some(e) = taken {
        let mut s = String::new();
        if e.alt {
            s.push_str(LEAVE_ALT);
        }
        s.push_str(SHOW_CURSOR);
        s.push_str(SGR_RESET);
        let _ = sys::write_all(e.out_fd, s.as_bytes());
        sys::restore(e.in_fd, &e.saved);
    }
}

// ---------------------------------------------------------------------------
// Term
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct TermOptions {
    /// Use the alternate screen. `false` is `--no-alt`: render into scrollback.
    pub alt_screen: bool,
    /// Plain (colourless) output.
    ///
    /// `Term` deliberately reads no environment of its own: `--plain`,
    /// `NO_COLOR` and "stdout is not a terminal" are folded into this one flag
    /// by `main::plain_mode`, so the interactive and dump paths can never
    /// disagree about what counts as plain (SPEC.md §CLI `--plain`).
    pub plain: bool,
}

impl Default for TermOptions {
    fn default() -> Self {
        TermOptions {
            alt_screen: true,
            plain: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TermError {
    /// No controlling terminal: neither stdin nor `/dev/tty` is usable.
    NoTty,
    /// `tcgetattr`/`tcsetattr` failed.
    RawMode,
    /// Write failed; carries the platform error code.
    Io(i32),
}

/// RAII terminal handle.
pub struct Term {
    in_fd: Fd,
    out_fd: Fd,
    owns_in_fd: bool,
    saved: Option<SavedTermios>,
    alt: bool,
    plain: bool,
    cols: u16,
    rows: u16,
}

impl Term {
    pub fn new(opts: TermOptions) -> Result<Term, TermError> {
        let (in_fd, owns_in_fd) = sys::tty_fd().ok_or(TermError::NoTty)?;
        // Prefer real stdout so redirection still works; fall back to the tty.
        let out_fd = if sys::is_tty(sys::STDOUT) {
            sys::STDOUT
        } else {
            in_fd
        };
        let interactive = sys::is_tty(out_fd);
        // No environment is consulted here; see `TermOptions::plain`.
        let plain = opts.plain || !interactive;
        let saved = enter_raw(in_fd, owns_in_fd)?;
        let alt = opts.alt_screen && interactive;
        let (cols, rows) = sys::winsize_of(out_fd)
            .or_else(sys::winsize)
            .unwrap_or((80, 24));
        let mut t = Term {
            in_fd,
            out_fd,
            owns_in_fd,
            saved: Some(saved),
            alt,
            plain,
            cols,
            rows,
        };
        *lock_emergency() = Some(Emergency {
            in_fd,
            out_fd,
            saved,
            alt,
        });
        let mut intro = String::new();
        if alt {
            intro.push_str(ENTER_ALT);
        }
        if interactive {
            intro.push_str(HIDE_CURSOR);
        }
        t.write(intro.as_bytes())?;
        Ok(t)
    }

    /// Cached `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
    /// Re-query the size after SIGWINCH; returns the new `(cols, rows)`.
    pub fn refresh_size(&mut self) -> (u16, u16) {
        if let Some((c, r)) = sys::winsize_of(self.out_fd).or_else(sys::winsize) {
            self.cols = c;
            self.rows = r;
        }
        (self.cols, self.rows)
    }
    /// True (and cleared) when a resize arrived since the last call.
    pub fn resize_pending(&self) -> bool {
        sys::winch_pending()
    }
    /// True (and cleared) when an OS-level interrupt arrived.
    pub fn interrupt_pending(&self) -> bool {
        sys::interrupt_pending()
    }
    /// True (and cleared) when SIGTERM/SIGHUP/SIGQUIT arrived. The event loop
    /// treats it as a quit so the terminal is restored on the way out.
    pub fn terminate_pending(&self) -> bool {
        sys::terminate_pending()
    }

    /// A frame buffer pre-configured for this terminal's colour mode.
    pub fn frame(&self) -> Frame {
        Frame::new(self.plain)
    }

    /// Issue exactly one `write(2)` for the whole frame.
    pub fn flush(&mut self, frame: &Frame) -> Result<(), TermError> {
        self.write(frame.as_bytes())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), TermError> {
        if bytes.is_empty() {
            return Ok(());
        }
        sys::write_all(self.out_fd, bytes).map_err(TermError::Io)
    }

    /// Read input bytes. Returns [`ReadOutcome::Timeout`] roughly every 100 ms
    /// when idle, which is the event loop's cue to poll [`Term::resize_pending`].
    pub fn read(&mut self, buf: &mut [u8]) -> ReadOutcome {
        sys::read_input(self.in_fd, buf)
    }

    /// Restore the terminal. Idempotent; `Drop` calls it.
    pub fn restore(&mut self) {
        let saved = match self.saved.take() {
            Some(s) => s,
            None => return,
        };
        let mut s = String::new();
        if self.alt {
            s.push_str(LEAVE_ALT);
        } else {
            s.push_str("\r\n");
        }
        s.push_str(SHOW_CURSOR);
        s.push_str(SGR_RESET);
        let _ = sys::write_all(self.out_fd, s.as_bytes());
        sys::restore(self.in_fd, &saved);
        *lock_emergency() = None;
        if self.owns_in_fd {
            sys::close_fd(self.in_fd);
            self.owns_in_fd = false;
        }
    }
}

/// Install the signal handlers and switch the input descriptor to raw mode,
/// closing an owned descriptor if that fails.
fn enter_raw(in_fd: Fd, owns_in_fd: bool) -> Result<SavedTermios, TermError> {
    sys::install_signal_handlers();
    match sys::set_raw(in_fd) {
        Some(s) => Ok(s),
        None => {
            if owns_in_fd {
                sys::close_fd(in_fd);
            }
            Err(TermError::RawMode)
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emergency_restore_is_a_noop_without_a_term() {
        emergency_restore();
        emergency_restore();
    }

    #[test]
    fn control_sequences_are_the_expected_ones() {
        assert_eq!(ENTER_ALT, "\x1b[?1049h");
        assert_eq!(LEAVE_ALT, "\x1b[?1049l");
        // 1049 is the alt-screen pair; none of these is mouse tracking.
        for s in [ENTER_ALT, LEAVE_ALT, HIDE_CURSOR, SHOW_CURSOR] {
            for bad in ["?1000", "?1002", "?1003", "?1006", "?1015"] {
                assert!(!s.contains(bad));
            }
        }
    }

    #[test]
    fn default_options_use_the_alternate_screen_with_colour() {
        let o = TermOptions::default();
        assert!(o.alt_screen && !o.plain);
    }

    #[test]
    fn term_reports_no_tty_when_none_is_available() {
        // In a CI/test harness without a controlling terminal this must be a
        // clean error rather than a panic. When a tty *is* present we simply
        // do not exercise the constructor (it would take over the terminal).
        match crate::sys::tty_fd() {
            None => assert_eq!(
                Term::new(TermOptions::default()).err(),
                Some(TermError::NoTty)
            ),
            Some((fd, owned)) => {
                if owned {
                    crate::sys::close_fd(fd);
                }
            }
        }
    }
}
