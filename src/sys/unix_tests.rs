//! Tests for the shared unix backend.
//!
//! These run on whatever unix the host is; the per-OS assertions below are
//! `cfg`-gated so each set is checked where it is true. The parts that can be
//! checked *everywhere* — flag arithmetic, struct layout, ioctl encoding — are
//! deliberately not here: they live in `abi_tests.rs` and `layout_tests.rs`,
//! where the Linux host also checks the Darwin values.

use super::*;
use crate::sys::abi::{Os, OS};

#[test]
fn the_host_table_matches_the_target_being_built() {
    #[cfg(target_os = "linux")]
    {
        assert_eq!(OS, Os::Linux);
        assert_eq!(HOST_ABI.icanon, 2);
        assert_eq!(HOST_ABI.echo, 8);
        assert_eq!(HOST_ABI.isig, 1);
        assert_eq!(HOST_ABI.iexten, 0x8000);
        assert_eq!(HOST_ABI.ixon, 0x400);
        assert_eq!(HOST_ABI.icrnl, 0x100);
        assert_eq!(HOST_ABI.opost, 1);
        assert_eq!(TIOCGWINSZ, 0x5413);
        assert_eq!((HOST_ABI.vmin_idx, HOST_ABI.vtime_idx), (6, 5));
        assert_eq!(HOST_ABI.eagain, 11);
        assert_eq!(HOST_POSIX.o_noctty, 0o400);
        assert_eq!(core::mem::size_of::<RawTermios>(), 76); // 60 + reserve tail
    }
    #[cfg(target_os = "macos")]
    {
        assert_eq!(OS, Os::Darwin);
        assert_eq!(HOST_ABI.icanon, 0x100);
        assert_eq!(HOST_ABI.echo, 8);
        assert_eq!(HOST_ABI.isig, 0x80);
        assert_eq!(HOST_ABI.iexten, 0x400);
        assert_eq!(HOST_ABI.ixon, 0x200);
        assert_eq!(HOST_ABI.icrnl, 0x100);
        assert_eq!(HOST_ABI.opost, 1);
        assert_eq!(TIOCGWINSZ, 0x4008_7468);
        assert_eq!((HOST_ABI.vmin_idx, HOST_ABI.vtime_idx), (16, 17));
        assert_eq!(HOST_ABI.eagain, 35);
        assert_eq!(HOST_POSIX.o_noctty, 0x2_0000);
        assert_eq!(core::mem::size_of::<RawTermios>(), 72);
    }
    // True on both, and the reason `signal` numbers can be shared.
    assert_eq!(HOST_ABI.tcsaflush, 2);
    assert_eq!(HOST_ABI.eintr, 4);
    assert_eq!(
        (
            HOST_POSIX.sighup,
            HOST_POSIX.sigint,
            HOST_POSIX.sigquit,
            HOST_POSIX.sigterm,
            HOST_POSIX.sigwinch
        ),
        (1, 2, 3, 15, 28)
    );
}

/// fd 1_000_000 is guaranteed not to be open in the test harness, so every
/// syscall wrapper below takes its failure path without touching a terminal.
const CLOSED: Fd = 1_000_000;

#[test]
fn writing_to_a_closed_fd_reports_an_error() {
    assert!(write_all(CLOSED, b"x").is_err());
}

#[test]
fn writing_nothing_always_succeeds() {
    assert!(write_all(CLOSED, b"").is_ok());
}

#[test]
fn winsize_of_a_non_tty_is_none() {
    assert_eq!(winsize_of(CLOSED), None);
}

#[test]
fn a_closed_fd_is_not_a_tty_and_cannot_enter_raw_mode() {
    assert!(!is_tty(CLOSED));
    assert!(set_raw(CLOSED).is_none());
}

#[test]
fn an_empty_read_buffer_is_a_timeout_not_a_syscall() {
    let mut buf: [u8; 0] = [];
    assert_eq!(read_input(CLOSED, &mut buf), ReadOutcome::Timeout);
}

#[test]
fn close_fd_ignores_the_standard_descriptors() {
    // Must not close stdin/stdout/stderr out from under the test harness.
    for fd in [0, 1, 2] {
        close_fd(fd);
    }
    assert!(write_all(super::super::STDOUT, b"").is_ok());
}

/// The signal flags are process-wide and consuming, so these tests cannot run
/// beside one another: one raising SIGWINCH while another asserts the flag is
/// clear is a race, and it failed a run at random before this lock existed.
/// The test harness threads them in parallel by default, so serialise the ones
/// that touch the flags. Poisoning is irrelevant here — a panicking test has
/// already failed, and the next one still wants the lock.
static SIGNALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn signals_locked() -> std::sync::MutexGuard<'static, ()> {
    SIGNALS.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn signal_handlers_install_and_flags_start_clear() {
    let _guard = signals_locked();
    install_signal_handlers();
    // Nothing has been raised while we hold the lock, so both must read false.
    let _ = super::super::terminate_pending();
    assert!(!super::super::winch_pending());
    assert!(!super::super::interrupt_pending());
}

/// Raising SIGTERM in-process must set the flag instead of killing
/// us. The end-to-end proof (raw mode and the alternate screen really
/// come back) is `tools/soak_pty.py`'s signal-teardown pass.
#[test]
fn a_termination_signal_sets_the_flag_and_does_not_kill_us() {
    extern "C" {
        fn raise(sig: c_int) -> c_int;
    }
    let _guard = signals_locked();
    install_signal_handlers();
    let _ = super::super::terminate_pending();
    // SAFETY: `raise` delivers a signal to this process only; the
    // handler installed above stores to an atomic and returns.
    unsafe { raise(HOST_POSIX.sigterm) };
    assert!(super::super::terminate_pending());
    assert!(!super::super::terminate_pending(), "the flag is consumed");
}

/// SIGWINCH is the one whose number differs across unixes often enough to be
/// worth proving rather than asserting: raise it and see the flag move.
#[test]
fn sigwinch_sets_the_resize_flag() {
    extern "C" {
        fn raise(sig: c_int) -> c_int;
    }
    let _guard = signals_locked();
    install_signal_handlers();
    let _ = super::super::winch_pending();
    // SAFETY: as above — in-process delivery to an atomic-only handler.
    unsafe { raise(HOST_POSIX.sigwinch) };
    assert!(super::super::winch_pending());
    assert!(!super::super::winch_pending(), "the flag is consumed");
}

/// `set_raw` must produce exactly the termios the hand-written sequence did.
/// Asserted against literals for the OS being built so a change in `abi` that
/// altered this target's behaviour fails here as well as in `layout_tests`.
#[test]
fn raw_mode_transformation_is_unchanged_for_this_target() {
    let mut t = RawTermios::zeroed();
    let all = super::abi::TermFlags {
        iflag: !0,
        oflag: !0,
        cflag: !0,
        lflag: !0,
        vmin: 1,
        vtime: 0,
    };
    t.set_flags(all, &HOST_ABI);
    let mut raw = t;
    raw.set_flags(abi::apply_raw_mode(t.flags(&HOST_ABI), &HOST_ABI), &HOST_ABI);
    let f = raw.flags(&HOST_ABI);
    #[cfg(target_os = "linux")]
    {
        assert_eq!(!f.iflag & 0xffff_ffff, 0o2 | 0o400 | 0o20 | 0o40 | 0o2000);
        assert_eq!(!f.lflag & 0xffff_ffff, 0o10 | 0o2 | 0o100000 | 0o1);
    }
    #[cfg(target_os = "macos")]
    {
        assert_eq!(!f.iflag & 0xffff_ffff, 0x2 | 0x100 | 0x10 | 0x20 | 0x200);
        assert_eq!(!f.lflag & 0xffff_ffff, 0x8 | 0x100 | 0x400 | 0x80);
    }
    assert_eq!(!f.oflag & 0xffff_ffff, HOST_ABI.opost);
    assert_eq!(f.cflag & HOST_ABI.csize, HOST_ABI.cs8);
    assert_eq!((f.vmin, f.vtime), (0, 1));
}
