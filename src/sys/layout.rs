//! The C struct layouts the unix backends pass to the kernel — every OS's, on
//! every target.
//!
//! Nothing here is `unsafe` and nothing here calls the OS. The structs are
//! declared unconditionally, not behind `cfg(target_os)`, for one reason: a
//! wrong `#[repr(C)]` layout compiles cleanly and then corrupts memory on a
//! machine this project cannot run. Declaring both means the Darwin layout is
//! size-asserted at compile time on *every* target (including the Linux CI
//! host) and field-offset-tested by `cargo test` on Linux, rather than only by
//! a Mac nobody has. `unix_linux.rs` / `unix_darwin.rs` each alias one of them.
//!
//! Both layouts are pure data + `repr(C)` arithmetic, and `repr(C)` field
//! placement for these types is identical under the System V AMD64 psABI and
//! AAPCS64 (`u64` aligns to 8, `u32` to 4, `u8` arrays to 1), so what the host
//! computes for `DarwinTermios` is what an Apple target computes.
//!
//! Authorities:
//!   * Linux — glibc `<bits/termios.h>` and musl `arch/x86_64/bits/termios.h`:
//!     `tcflag_t`/`speed_t` = `unsigned int`, `cc_t` = `unsigned char`,
//!     `NCCS` = 32, members `c_iflag, c_oflag, c_cflag, c_lflag, c_line,
//!     c_cc[NCCS], c_ispeed, c_ospeed`; `sizeof` = 60.
//!   * Darwin — `xnu/bsd/sys/termios.h`: `tcflag_t`/`speed_t` = `unsigned
//!     long` (8 bytes on both `x86_64` and `arm64`), `cc_t` = `unsigned char`,
//!     `NCCS` = 20, members `c_iflag, c_oflag, c_cflag, c_lflag, c_cc[NCCS],
//!     c_ispeed, c_ospeed` — **no `c_line`**, that is a Linux/SysV member;
//!     `sizeof` = 72 (32 flags + 20 c_cc + 4 tail pad + 16 speeds).
//!   * `struct winsize` — `<bits/ioctl-types.h>` and `xnu/bsd/sys/ttycom.h`:
//!     four `unsigned short`, in the order row, col, xpixel, ypixel, on both.
#![deny(unsafe_code)]
// The backend for a given target uses exactly one of these; the other is here
// to be asserted against, so it is "dead" by construction.
#![allow(dead_code)]

use super::abi::{TermFlags, TermiosAbi};

/// Linux `NCCS`.
pub const LINUX_NCCS: usize = 32;
/// Darwin `NCCS`.
pub const DARWIN_NCCS: usize = 20;

/// Linux `struct termios` (identical under glibc and musl).
///
/// `_tail_pad` is reserve only; no libc reads or writes it. It exists so that a
/// libc writing a *shorter* struct is still safe and a hypothetical longer one
/// cannot overflow our storage.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LinuxTermios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; LINUX_NCCS],
    c_ispeed: u32,
    c_ospeed: u32,
    _tail_pad: [u8; 16],
}

/// Darwin `struct termios`. Exact size — 72 bytes, no reserve tail — because
/// the layout is pinned by the assertions below and by `xnu`'s header, and an
/// exact size is a stronger statement than a padded one.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DarwinTermios {
    c_iflag: u64,
    c_oflag: u64,
    c_cflag: u64,
    c_lflag: u64,
    c_cc: [u8; DARWIN_NCCS],
    c_ispeed: u64,
    c_ospeed: u64,
}

/// `struct winsize`, shared verbatim by Linux and Darwin.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

impl Winsize {
    pub const fn zeroed() -> Self {
        Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
    /// `(cols, rows)`, in the order the rest of the crate uses.
    pub fn cols_rows(&self) -> (u16, u16) {
        (self.ws_col, self.ws_row)
    }
}

impl LinuxTermios {
    pub const fn zeroed() -> Self {
        LinuxTermios {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: 0,
            c_line: 0,
            c_cc: [0; LINUX_NCCS],
            c_ispeed: 0,
            c_ospeed: 0,
            _tail_pad: [0; 16],
        }
    }

    /// The portable view of this struct, for `abi::apply_raw_mode`.
    pub fn flags(&self, a: &TermiosAbi) -> TermFlags {
        TermFlags {
            iflag: self.c_iflag as u64,
            oflag: self.c_oflag as u64,
            cflag: self.c_cflag as u64,
            lflag: self.c_lflag as u64,
            vmin: self.c_cc[a.vmin_idx],
            vtime: self.c_cc[a.vtime_idx],
        }
    }

    /// Write a portable view back into the C layout, narrowing to `tcflag_t`.
    pub fn set_flags(&mut self, f: TermFlags, a: &TermiosAbi) {
        self.c_iflag = f.iflag as u32;
        self.c_oflag = f.oflag as u32;
        self.c_cflag = f.cflag as u32;
        self.c_lflag = f.lflag as u32;
        self.c_cc[a.vmin_idx] = f.vmin;
        self.c_cc[a.vtime_idx] = f.vtime;
    }
}

impl DarwinTermios {
    pub const fn zeroed() -> Self {
        DarwinTermios {
            c_iflag: 0,
            c_oflag: 0,
            c_cflag: 0,
            c_lflag: 0,
            c_cc: [0; DARWIN_NCCS],
            c_ispeed: 0,
            c_ospeed: 0,
        }
    }

    /// The portable view of this struct. `tcflag_t` is already 64-bit here, so
    /// this widens nothing.
    pub fn flags(&self, a: &TermiosAbi) -> TermFlags {
        TermFlags {
            iflag: self.c_iflag,
            oflag: self.c_oflag,
            cflag: self.c_cflag,
            lflag: self.c_lflag,
            vmin: self.c_cc[a.vmin_idx],
            vtime: self.c_cc[a.vtime_idx],
        }
    }

    /// Write a portable view back. No narrowing: `TermFlags` is `u64` precisely
    /// so Darwin's `unsigned long` flags survive the round trip intact.
    pub fn set_flags(&mut self, f: TermFlags, a: &TermiosAbi) {
        self.c_iflag = f.iflag;
        self.c_oflag = f.oflag;
        self.c_cflag = f.cflag;
        self.c_lflag = f.lflag;
        self.c_cc[a.vmin_idx] = f.vmin;
        self.c_cc[a.vtime_idx] = f.vtime;
    }
}

// ---------------------------------------------------------------------------
// Compile-time ABI assertions — enforced on every target, not just the one
// whose backend is being built.
// ---------------------------------------------------------------------------

use core::mem::{align_of, size_of};

use super::abi::{DARWIN, LINUX, WINSIZE_LEN};

// Linux: the real struct is 60 bytes; ours carries a 16-byte reserve tail.
const _: () = assert!(size_of::<LinuxTermios>() == 76);
const _: () = assert!(size_of::<LinuxTermios>() >= 60);
const _: () = assert!(align_of::<LinuxTermios>() == 4);
const _: () = assert!(LINUX.nccs == LINUX_NCCS);
const _: () = assert!(LINUX.vmin_idx < LINUX_NCCS && LINUX.vtime_idx < LINUX_NCCS);
// `tcflag_t` is 4 bytes on Linux, so no Linux mask may overflow one.
const _: () = assert!(LINUX.iexten <= u32::MAX as u64);
const _: () = assert!(LINUX.csize <= u32::MAX as u64);

// Darwin: sizeof(struct termios) == 72, alignment 8 (`unsigned long` members).
const _: () = assert!(size_of::<DarwinTermios>() == 72);
const _: () = assert!(align_of::<DarwinTermios>() == 8);
const _: () = assert!(DARWIN.nccs == DARWIN_NCCS);
const _: () = assert!(DARWIN.vmin_idx < DARWIN_NCCS && DARWIN.vtime_idx < DARWIN_NCCS);
// VMIN/VTIME are c_cc[16]/c_cc[17] on Darwin (VSTATUS is 18, 19 is spare);
// using Linux's 6/5 would set VREPRINT/VKILL and the reader would block or spin.
const _: () = assert!(DARWIN.vmin_idx == 16 && DARWIN.vtime_idx == 17);

const _: () = assert!(size_of::<Winsize>() == WINSIZE_LEN);
const _: () = assert!(align_of::<Winsize>() == 2);

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
