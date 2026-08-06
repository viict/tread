//! Host tests for [`crate::sys::layout`].
//!
//! These run on Linux and check **both** OS layouts, which is the whole point
//! of declaring them unconditionally: the Darwin `struct termios` offsets are
//! verified by a machine that cannot run Darwin.

use super::*;
use crate::sys::abi::{apply_raw_mode, DARWIN, LINUX};

/// Byte offset of a field, computed from two addresses. `offset_of!` would be
/// tidier but is newer than this crate's stated MSRV (Cargo.toml `rust-version`).
fn off(base: usize, field: usize) -> usize {
    field - base
}

#[test]
fn linux_termios_layout_matches_glibc_and_musl() {
    assert_eq!(size_of::<LinuxTermios>(), 76); // 60 real + 16 reserve
    assert_eq!(align_of::<LinuxTermios>(), 4);
    let t = LinuxTermios::zeroed();
    let b = &t as *const LinuxTermios as usize;
    assert_eq!(off(b, &t.c_iflag as *const _ as usize), 0);
    assert_eq!(off(b, &t.c_oflag as *const _ as usize), 4);
    assert_eq!(off(b, &t.c_cflag as *const _ as usize), 8);
    assert_eq!(off(b, &t.c_lflag as *const _ as usize), 12);
    assert_eq!(off(b, &t.c_line as *const _ as usize), 16);
    assert_eq!(off(b, &t.c_cc as *const _ as usize), 17);
    assert_eq!(off(b, &t.c_ispeed as *const _ as usize), 52);
    assert_eq!(off(b, &t.c_ospeed as *const _ as usize), 56);
}

/// `xnu/bsd/sys/termios.h`: four 8-byte flags, then `c_cc[20]`, then the two
/// 8-byte speeds after four bytes of tail padding. No `c_line`.
#[test]
fn darwin_termios_layout_matches_the_xnu_header() {
    assert_eq!(size_of::<DarwinTermios>(), 72);
    assert_eq!(align_of::<DarwinTermios>(), 8);
    let t = DarwinTermios::zeroed();
    let b = &t as *const DarwinTermios as usize;
    assert_eq!(off(b, &t.c_iflag as *const _ as usize), 0);
    assert_eq!(off(b, &t.c_oflag as *const _ as usize), 8);
    assert_eq!(off(b, &t.c_cflag as *const _ as usize), 16);
    assert_eq!(off(b, &t.c_lflag as *const _ as usize), 24);
    assert_eq!(off(b, &t.c_cc as *const _ as usize), 32);
    assert_eq!(off(b, &t.c_ispeed as *const _ as usize), 56);
    assert_eq!(off(b, &t.c_ospeed as *const _ as usize), 64);
    assert_eq!(t.c_cc.len(), 20);
}

/// The two layouts must not be interchangeable, which is the mistake this
/// whole module exists to prevent.
#[test]
fn the_two_termios_layouts_are_genuinely_different() {
    assert_ne!(size_of::<LinuxTermios>(), size_of::<DarwinTermios>());
    assert_ne!(align_of::<LinuxTermios>(), align_of::<DarwinTermios>());
    assert_ne!(LINUX_NCCS, DARWIN_NCCS);
}

#[test]
fn winsize_is_four_shorts_row_first() {
    assert_eq!(size_of::<Winsize>(), 8);
    let mut ws = Winsize::zeroed();
    ws.ws_row = 24;
    ws.ws_col = 80;
    assert_eq!(ws.cols_rows(), (80, 24));
}

#[test]
fn linux_flags_round_trip_losslessly() {
    let mut t = LinuxTermios::zeroed();
    t.c_iflag = 0xdead_beef;
    t.c_oflag = 0x0123_4567;
    t.c_cflag = 0x89ab_cdef;
    t.c_lflag = 0xfeed_face;
    t.c_cc[LINUX.vmin_idx] = 7;
    t.c_cc[LINUX.vtime_idx] = 9;
    let f = t.flags(&LINUX);
    assert_eq!(
        (f.iflag, f.oflag, f.cflag, f.lflag, f.vmin, f.vtime),
        (0xdead_beef, 0x0123_4567, 0x89ab_cdef, 0xfeed_face, 7, 9)
    );
    let mut back = LinuxTermios::zeroed();
    back.set_flags(f, &LINUX);
    assert_eq!(back.c_iflag, t.c_iflag);
    assert_eq!(back.c_oflag, t.c_oflag);
    assert_eq!(back.c_cflag, t.c_cflag);
    assert_eq!(back.c_lflag, t.c_lflag);
    assert_eq!(back.c_cc, t.c_cc);
}

/// Darwin's `tcflag_t` is 64-bit, so a flag word with bits above 32 must
/// survive; on Linux the same value would be truncated by design.
#[test]
fn darwin_flags_round_trip_losslessly_including_the_high_word() {
    let mut t = DarwinTermios::zeroed();
    t.c_iflag = 0x1234_5678_9abc_def0;
    t.c_oflag = 0xffff_ffff_0000_0001;
    t.c_cflag = 0x0000_0001_ffff_ffff;
    t.c_lflag = 0x8000_0000_0000_0008;
    t.c_cc[DARWIN.vmin_idx] = 7;
    t.c_cc[DARWIN.vtime_idx] = 9;
    let f = t.flags(&DARWIN);
    assert_eq!(f.iflag, 0x1234_5678_9abc_def0);
    assert_eq!(f.oflag, 0xffff_ffff_0000_0001);
    assert_eq!(f.cflag, 0x0000_0001_ffff_ffff);
    assert_eq!(f.lflag, 0x8000_0000_0000_0008);
    assert_eq!((f.vmin, f.vtime), (7, 9));
    let mut back = DarwinTermios::zeroed();
    back.set_flags(f, &DARWIN);
    assert_eq!(back.c_iflag, t.c_iflag);
    assert_eq!(back.c_oflag, t.c_oflag);
    assert_eq!(back.c_cflag, t.c_cflag);
    assert_eq!(back.c_lflag, t.c_lflag);
    assert_eq!(back.c_cc, t.c_cc);
}

/// VMIN/VTIME must land in slots 16/17 on Darwin. Writing Linux's 6/5 would
/// leave VMIN at its inherited value (typically 1) and the pager would block
/// forever instead of ticking every 100 ms.
#[test]
fn darwin_vmin_and_vtime_land_in_slots_16_and_17() {
    let mut t = DarwinTermios::zeroed();
    t.set_flags(apply_raw_mode(t.flags(&DARWIN), &DARWIN), &DARWIN);
    assert_eq!(t.c_cc[16], 0, "VMIN");
    assert_eq!(t.c_cc[17], 1, "VTIME");
    for (i, b) in t.c_cc.iter().enumerate() {
        if i != 17 {
            assert_eq!(*b, 0, "c_cc[{i}] must be untouched");
        }
    }
}

#[test]
fn linux_vmin_and_vtime_land_in_slots_6_and_5() {
    let mut t = LinuxTermios::zeroed();
    t.set_flags(apply_raw_mode(t.flags(&LINUX), &LINUX), &LINUX);
    assert_eq!(t.c_cc[6], 0, "VMIN");
    assert_eq!(t.c_cc[5], 1, "VTIME");
    for (i, b) in t.c_cc.iter().enumerate() {
        if i != 5 {
            assert_eq!(*b, 0, "c_cc[{i}] must be untouched");
        }
    }
}

/// The full Darwin raw-mode transformation, spelled out against literals from
/// `xnu/bsd/sys/termios.h` rather than against the mask names, so a wrong
/// constant in the table fails here.
#[test]
fn darwin_raw_mode_clears_exactly_the_documented_bits() {
    let mut t = DarwinTermios::zeroed();
    t.c_iflag = !0;
    t.c_oflag = !0;
    t.c_cflag = !0;
    t.c_lflag = !0;
    let mut raw = t;
    raw.set_flags(apply_raw_mode(t.flags(&DARWIN), &DARWIN), &DARWIN);
    // BRKINT|ICRNL|INPCK|ISTRIP|IXON = 0x2|0x100|0x10|0x20|0x200
    assert_eq!(!raw.c_iflag, 0x332);
    // OPOST
    assert_eq!(!raw.c_oflag, 0x1);
    // CSIZE (0x300) cleared, CS8 (0x300) set: on Darwin too these coincide.
    assert_eq!(raw.c_cflag, !0);
    // ECHO|ICANON|IEXTEN|ISIG = 0x8|0x100|0x400|0x80
    assert_eq!(!raw.c_lflag, 0x588);
    // Nothing above bit 31 may be disturbed: Darwin has real flags up there
    // (e.g. NOFLSH 0x80000000 is in range, but the 64-bit word must survive).
    assert_eq!(raw.c_lflag >> 32, 0xffff_ffff);
}
