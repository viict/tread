//! Pure, platform-independent core of the `sys` layer.
//!
//! Nothing in this file is `unsafe`, nothing in it calls the operating system,
//! and none of its *logic* is `cfg`-gated — only the choice of **which constant
//! table** describes the host. That is deliberate: every OS ABI fact that can be
//! written down as arithmetic (raw-mode flag masks, `_IOR`/`_IOW` request-number
//! encoding, the read/write retry state machine) lives here, where it compiles
//! for every target and is unit-tested on the Linux host, instead of hiding
//! inside an `unsafe fn` on a machine nobody in CI can run.
//!
//! The backends in `unix.rs` / `stub.rs` are therefore reduced to "call the
//! syscall, hand the result to a function in this module".
//!
//! Authorities: Linux — `<asm-generic/ioctls.h>`, glibc `<bits/termios.h>` and
//! musl `arch/x86_64/bits/termios.h` (identical for every field used). Darwin —
//! `xnu/bsd/sys/{termios,ioccom,ttycom,signal,fcntl,errno}.h`, which are the
//! same for `x86_64-apple-darwin` and `aarch64-apple-darwin`.
#![deny(unsafe_code)]
// On non-unix targets the backend is `stub.rs`, which needs none of this; the
// module still compiles (and is still tested on the host) so that a future
// backend has it available.
#![cfg_attr(not(unix), allow(dead_code))]

// ---------------------------------------------------------------------------
// Which unix
// ---------------------------------------------------------------------------

/// The unix flavours whose terminal ABI is tabulated here.
///
/// Only one variant describes a given build; the other's tables still compile
/// (and are still asserted against known-correct values by the host tests), so
/// the macOS numbers are checked by a Linux CI run and vice versa.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum Os {
    Linux,
    Darwin,
}

/// The host's flavour.
///
/// Any unix that is neither macOS nor Linux is described by the Linux table —
/// `unix.rs` refuses to build on such a target anyway, with a message saying so.
#[cfg(all(unix, target_os = "macos"))]
pub const OS: Os = Os::Darwin;
#[cfg(all(unix, not(target_os = "macos")))]
pub const OS: Os = Os::Linux;

// ---------------------------------------------------------------------------
// termios: the flags raw mode touches
// ---------------------------------------------------------------------------

/// The subset of `struct termios` that [`apply_raw_mode`] reads or writes.
///
/// Widened to `u64` so the same type describes Linux (`tcflag_t` = 4 bytes) and
/// Darwin (`tcflag_t` = 8 bytes); a backend narrows on the way back in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TermFlags {
    pub iflag: u64,
    pub oflag: u64,
    pub cflag: u64,
    pub lflag: u64,
    /// `c_cc[VMIN]`.
    pub vmin: u8,
    /// `c_cc[VTIME]`, in tenths of a second.
    pub vtime: u8,
}

/// One OS's termios/errno numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TermiosAbi {
    // c_iflag
    pub brkint: u64,
    pub inpck: u64,
    pub istrip: u64,
    pub icrnl: u64,
    pub ixon: u64,
    // c_oflag
    pub opost: u64,
    // c_cflag
    pub csize: u64,
    pub cs8: u64,
    // c_lflag
    pub isig: u64,
    pub icanon: u64,
    pub echo: u64,
    pub iexten: u64,
    // c_cc
    pub nccs: usize,
    pub vmin_idx: usize,
    pub vtime_idx: usize,
    // tcsetattr(2)
    pub tcsaflush: i32,
    // errno
    pub eintr: i32,
    pub eagain: i32,
}

/// x86_64/aarch64 Linux, glibc and musl alike.
pub const LINUX: TermiosAbi = TermiosAbi {
    brkint: 0o000002,
    inpck: 0o000020,
    istrip: 0o000040,
    icrnl: 0o000400,
    ixon: 0o002000,
    opost: 0o000001,
    csize: 0o000060,
    cs8: 0o000060,
    isig: 0o000001,
    icanon: 0o000002,
    echo: 0o000010,
    iexten: 0o100000,
    nccs: 32,
    vmin_idx: 6,
    vtime_idx: 5,
    tcsaflush: 2,
    eintr: 4,
    eagain: 11,
};

/// Darwin: `xnu/bsd/sys/termios.h`. `x86_64-apple-darwin` and
/// `aarch64-apple-darwin` share every value — the divergence from Linux is the
/// header's, not the architecture's.
pub const DARWIN: TermiosAbi = TermiosAbi {
    brkint: 0x0000_0002,
    inpck: 0x0000_0010,
    istrip: 0x0000_0020,
    icrnl: 0x0000_0100,
    ixon: 0x0000_0200,
    opost: 0x0000_0001,
    csize: 0x0000_0300,
    cs8: 0x0000_0300,
    isig: 0x0000_0080,
    icanon: 0x0000_0100,
    echo: 0x0000_0008,
    iexten: 0x0000_0400,
    nccs: 20,
    vmin_idx: 16,
    vtime_idx: 17,
    tcsaflush: 2,
    eintr: 4,
    eagain: 35,
};

/// The table describing `os`.
pub const fn abi_for(os: Os) -> TermiosAbi {
    match os {
        Os::Linux => LINUX,
        Os::Darwin => DARWIN,
    }
}

/// The table describing the host.
#[cfg(unix)]
pub const HOST_ABI: TermiosAbi = abi_for(OS);

/// The raw-mode transformation, as a pure function.
///
/// This is `cfmakeraw(3)` minus the parts this reader does not want, plus the
/// `VMIN=0 / VTIME=1` poll behaviour the event loop depends on: a read comes
/// back after at most 100 ms with zero bytes, which is the tick that lets the
/// loop notice a resize without relying on `EINTR` semantics (they differ
/// across libcs).
pub fn apply_raw_mode(f: TermFlags, a: &TermiosAbi) -> TermFlags {
    TermFlags {
        iflag: f.iflag & !(a.brkint | a.icrnl | a.inpck | a.istrip | a.ixon),
        oflag: f.oflag & !a.opost,
        cflag: (f.cflag & !a.csize) | a.cs8,
        lflag: f.lflag & !(a.echo | a.icanon | a.iexten | a.isig),
        vmin: 0,
        vtime: 1,
    }
}

// ---------------------------------------------------------------------------
// ioctl request numbers
// ---------------------------------------------------------------------------

/// BSD `_IOC` direction bits (`<sys/ioccom.h>`).
pub const IOC_OUT: u64 = 0x4000_0000;
/// Kernel reads the argument. Unused today; kept so the encoder is complete.
#[allow(dead_code)]
pub const IOC_IN: u64 = 0x8000_0000;
/// No argument. Unused today; kept so the encoder is complete.
#[allow(dead_code)]
pub const IOC_VOID: u64 = 0x2000_0000;

/// `IOCPARM_MASK`: the payload size field is 13 bits wide.
const IOCPARM_MASK: u64 = 0x1fff;

/// The BSD/Darwin `_IOC(dir, group, num, len)` encoding.
///
/// `_IOR(g, n, T)` is `bsd_ioc(IOC_OUT, g, n, size_of::<T>())` and `_IOW` is the
/// same with [`IOC_IN`], which is why this takes the direction as a parameter
/// rather than existing twice.
pub const fn bsd_ioc(dir: u64, group: u8, num: u8, len: usize) -> u64 {
    dir | (((len as u64) & IOCPARM_MASK) << 16) | ((group as u64) << 8) | (num as u64)
}

/// `_IOR(group, num, T)` where `len == size_of::<T>()`.
pub const fn bsd_ior(group: u8, num: u8, len: usize) -> u64 {
    bsd_ioc(IOC_OUT, group, num, len)
}

/// `sizeof(struct winsize)` — four `unsigned short` on every platform here.
pub const WINSIZE_LEN: usize = 8;

/// `TIOCGWINSZ` for `os`.
///
/// Darwin's is *computed*: `_IOR('t', 104, struct winsize)`. Linux's is a
/// legacy magic number predating the `_IOC` scheme — the tty ioctls in
/// `<asm-generic/ioctls.h>` are a hand-numbered 0x54xx block, so there is
/// nothing to derive and the test below pins the literal instead.
pub const fn tiocgwinsz(os: Os) -> u64 {
    match os {
        Os::Linux => 0x5413,
        Os::Darwin => bsd_ior(b't', 104, WINSIZE_LEN),
    }
}

/// `TIOCGWINSZ` for the host.
#[cfg(unix)]
pub const TIOCGWINSZ: u64 = tiocgwinsz(OS);

/// Interpret an ioctl result plus the `winsize` it filled in.
///
/// A zero dimension is as useless as a failure (`term.rs` substitutes 80x24),
/// so both collapse to `None`.
pub fn winsize_result(rc: i32, cols: u16, rows: u16) -> Option<(u16, u16)> {
    if rc != 0 || cols == 0 || rows == 0 {
        None
    } else {
        Some((cols, rows))
    }
}

// ---------------------------------------------------------------------------
// The non-termios numbers a unix backend needs
// ---------------------------------------------------------------------------

/// `open(2)` flags and signal numbers for one OS.
///
/// Small, but exactly the place a port goes wrong quietly: `O_NOCTTY` is
/// `0o400` on Linux and `0x20000` on Darwin, and `0o400` on Darwin means
/// `O_NOFOLLOW`. Tabulating both here means the Darwin values are asserted by
/// the Linux host's test run instead of only by a machine nobody has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PosixAbi {
    /// `O_RDWR`.
    pub o_rdwr: i32,
    /// `O_NOCTTY` — open the tty without making it the controlling terminal.
    pub o_noctty: i32,
    pub sighup: i32,
    pub sigint: i32,
    pub sigquit: i32,
    pub sigterm: i32,
    pub sigwinch: i32,
}

/// Linux (`<asm-generic/fcntl.h>`, `<asm/signal.h>`).
pub const LINUX_POSIX: PosixAbi = PosixAbi {
    o_rdwr: 0o2,
    o_noctty: 0o400,
    sighup: 1,
    sigint: 2,
    sigquit: 3,
    sigterm: 15,
    sigwinch: 28,
};

/// Darwin (`<sys/fcntl.h>`, `<sys/signal.h>`). The five signal numbers happen
/// to agree with Linux — `SIGWINCH` is 28 in both the 4.3BSD numbering Darwin
/// kept and the x86/arm Linux numbering — but `O_NOCTTY` does not.
pub const DARWIN_POSIX: PosixAbi = PosixAbi {
    o_rdwr: 0x0002,
    o_noctty: 0x2_0000,
    sighup: 1,
    sigint: 2,
    sigquit: 3,
    sigterm: 15,
    sigwinch: 28,
};

/// The `open`/signal table describing `os`.
pub const fn posix_for(os: Os) -> PosixAbi {
    match os {
        Os::Linux => LINUX_POSIX,
        Os::Darwin => DARWIN_POSIX,
    }
}

/// The `open`/signal table describing the host.
#[cfg(unix)]
pub const HOST_POSIX: PosixAbi = posix_for(OS);

// ---------------------------------------------------------------------------
// read / write state machines
// ---------------------------------------------------------------------------

/// What a backend should do with the return value of one `read(2)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadStep {
    /// `n` bytes are in the buffer.
    Bytes(usize),
    /// Nothing arrived within `VTIME`; poll the signal flags and come back.
    Timeout,
    /// Interrupted before any data; call `read` again.
    Retry,
    /// Give up with this errno.
    Error(i32),
}

/// Classify one `read(2)` return value.
///
/// `errno` is only consulted when `n < 0`; pass anything when it is not.
/// A zero-length read is a *timeout*, not EOF: with `VMIN=0`/`VTIME>0` the two
/// are indistinguishable on a unix tty, and treating it as EOF would quit the
/// pager after 100 ms of idleness.
pub fn classify_read(n: isize, errno: i32, a: &TermiosAbi) -> ReadStep {
    if n > 0 {
        ReadStep::Bytes(n as usize)
    } else if n == 0 {
        ReadStep::Timeout
    } else if errno == a.eintr {
        ReadStep::Retry
    } else if errno == a.eagain {
        ReadStep::Timeout
    } else {
        ReadStep::Error(errno)
    }
}

/// What a backend should do with the return value of one `write(2)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteStep {
    /// `n` bytes went out; advance the cursor and keep going.
    Advance(usize),
    /// Interrupted or the pipe was momentarily full; call `write` again.
    Retry,
    /// Give up with this error code.
    Fail(i32),
}

/// Classify one `write(2)` return value. A zero-length write on a non-empty
/// buffer cannot make progress, so it fails rather than spinning forever.
pub fn classify_write(n: isize, errno: i32, a: &TermiosAbi) -> WriteStep {
    if n > 0 {
        WriteStep::Advance(n as usize)
    } else if n == 0 {
        WriteStep::Fail(0)
    } else if errno == a.eintr || errno == a.eagain {
        WriteStep::Retry
    } else {
        WriteStep::Fail(errno)
    }
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
