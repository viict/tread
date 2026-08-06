//! Host tests for [`crate::sys::abi`].
//!
//! Split out of `abi.rs` only to keep that file under the 500-line limit
//! (CLAUDE.md §Non-negotiables). Every test here runs on whatever host CI uses
//! — that is the point: the Darwin tables are checked on Linux, because there
//! is no macOS machine in this project's loop.

use super::*;

#[test]
fn linux_flag_values_match_the_linux_headers() {
    assert_eq!(LINUX.brkint, 2);
    assert_eq!(LINUX.inpck, 0x10);
    assert_eq!(LINUX.istrip, 0x20);
    assert_eq!(LINUX.icrnl, 0x100);
    assert_eq!(LINUX.ixon, 0x400);
    assert_eq!(LINUX.opost, 1);
    assert_eq!(LINUX.csize, 0x30);
    assert_eq!(LINUX.cs8, 0x30);
    assert_eq!(LINUX.isig, 1);
    assert_eq!(LINUX.icanon, 2);
    assert_eq!(LINUX.echo, 8);
    assert_eq!(LINUX.iexten, 0x8000);
    assert_eq!((LINUX.nccs, LINUX.vmin_idx, LINUX.vtime_idx), (32, 6, 5));
    assert_eq!((LINUX.eintr, LINUX.eagain, LINUX.tcsaflush), (4, 11, 2));
}

#[test]
fn darwin_flag_values_match_the_xnu_headers() {
    assert_eq!(DARWIN.brkint, 2);
    assert_eq!(DARWIN.inpck, 0x10);
    assert_eq!(DARWIN.istrip, 0x20);
    assert_eq!(DARWIN.icrnl, 0x100);
    assert_eq!(DARWIN.ixon, 0x200);
    assert_eq!(DARWIN.opost, 1);
    assert_eq!(DARWIN.csize, 0x300);
    assert_eq!(DARWIN.cs8, 0x300);
    assert_eq!(DARWIN.isig, 0x80);
    assert_eq!(DARWIN.icanon, 0x100);
    assert_eq!(DARWIN.echo, 8);
    assert_eq!(DARWIN.iexten, 0x400);
    assert_eq!(
        (DARWIN.nccs, DARWIN.vmin_idx, DARWIN.vtime_idx),
        (20, 16, 17)
    );
    assert_eq!((DARWIN.eintr, DARWIN.eagain, DARWIN.tcsaflush), (4, 35, 2));
}

#[test]
fn the_two_tables_are_genuinely_different() {
    // If these ever compare equal, someone copy-pasted one over the other.
    assert_ne!(LINUX, DARWIN);
    assert_eq!(abi_for(Os::Linux), LINUX);
    assert_eq!(abi_for(Os::Darwin), DARWIN);
}

/// The bits raw mode must clear, spelled out independently of the masks so
/// this is a real check and not a restatement of `apply_raw_mode`.
#[test]
fn raw_mode_clears_exactly_the_intended_linux_bits() {
    let before = TermFlags {
        iflag: !0,
        oflag: !0,
        cflag: !0,
        lflag: !0,
        vmin: 7,
        vtime: 9,
    };
    let raw = apply_raw_mode(before, &LINUX);
    // BRKINT|ICRNL|INPCK|ISTRIP|IXON = 2|0x100|0x10|0x20|0x400 = 0x532
    assert_eq!(!raw.iflag & 0xffff_ffff, 0x532);
    assert_eq!(!raw.oflag & 0xffff_ffff, 0x1);
    // CSIZE cleared then CS8 set: on Linux CS8 == CSIZE, so nothing changes.
    assert_eq!(raw.cflag & 0xffff_ffff, 0xffff_ffff);
    // ECHO|ICANON|IEXTEN|ISIG = 0x800b
    assert_eq!(!raw.lflag & 0xffff_ffff, 0x800b);
    assert_eq!((raw.vmin, raw.vtime), (0, 1));
}

#[test]
fn raw_mode_sets_cs8_from_a_smaller_character_size() {
    // CS7 on Linux is 0o40 (CSIZE 0o60 with CS8 0o60): start from CS7.
    let f = TermFlags {
        cflag: 0o40,
        ..TermFlags::default()
    };
    assert_eq!(apply_raw_mode(f, &LINUX).cflag, LINUX.cs8);
    // Darwin CS7 == 0x200 inside CSIZE 0x300.
    let f = TermFlags {
        cflag: 0x200,
        ..TermFlags::default()
    };
    assert_eq!(apply_raw_mode(f, &DARWIN).cflag, DARWIN.cs8);
}

#[test]
fn raw_mode_preserves_bits_it_does_not_own() {
    // A bit outside every mask must survive untouched, on both tables.
    for a in [LINUX, DARWIN] {
        let f = TermFlags {
            iflag: 1 << 40,
            oflag: 1 << 41,
            cflag: 1 << 42,
            lflag: 1 << 43,
            vmin: 3,
            vtime: 3,
        };
        let r = apply_raw_mode(f, &a);
        assert_eq!(r.iflag, 1 << 40);
        assert_eq!(r.oflag, 1 << 41);
        assert_eq!(r.cflag, (1 << 42) | a.cs8);
        assert_eq!(r.lflag, 1 << 43);
    }
}

#[test]
fn raw_mode_is_idempotent() {
    for a in [LINUX, DARWIN] {
        let once = apply_raw_mode(
            TermFlags {
                iflag: !0,
                oflag: !0,
                cflag: !0,
                lflag: !0,
                vmin: 1,
                vtime: 0,
            },
            &a,
        );
        assert_eq!(apply_raw_mode(once, &a), once);
    }
}

#[test]
fn ioctl_encoding_reproduces_the_known_request_numbers() {
    // _IOR('t', 104, struct winsize) on Darwin/BSD.
    assert_eq!(bsd_ior(b't', 104, 8), 0x4008_7468);
    // _IOW('t', 103, struct winsize) is TIOCSWINSZ.
    assert_eq!(bsd_ioc(IOC_IN, b't', 103, 8), 0x8008_7467);
    // _IO('t', 20) style: no payload.
    assert_eq!(bsd_ioc(IOC_VOID, b't', 20, 0), 0x2000_7414);
    // Oversized payloads are masked to 13 bits, exactly as the C macro does.
    assert_eq!(bsd_ioc(IOC_OUT, b't', 1, 0x2000), 0x4000_7401);
}

#[test]
fn tiocgwinsz_is_right_for_each_os() {
    assert_eq!(tiocgwinsz(Os::Linux), 0x5413);
    assert_eq!(tiocgwinsz(Os::Darwin), 0x4008_7468);
}

#[test]
fn winsize_results_reject_failures_and_zero_dimensions() {
    assert_eq!(winsize_result(0, 80, 24), Some((80, 24)));
    assert_eq!(winsize_result(-1, 80, 24), None);
    assert_eq!(winsize_result(0, 0, 24), None);
    assert_eq!(winsize_result(0, 80, 0), None);
}

#[test]
fn read_classification_matches_the_vmin0_vtime1_contract() {
    let a = &LINUX;
    assert_eq!(classify_read(5, 0, a), ReadStep::Bytes(5));
    assert_eq!(classify_read(0, 0, a), ReadStep::Timeout);
    assert_eq!(classify_read(-1, a.eintr, a), ReadStep::Retry);
    assert_eq!(classify_read(-1, a.eagain, a), ReadStep::Timeout);
    assert_eq!(classify_read(-1, 9, a), ReadStep::Error(9));
}

#[test]
fn read_classification_uses_the_per_os_eagain() {
    // 11 is EAGAIN on Linux but EDEADLK on Darwin; 35 is the reverse.
    assert_eq!(classify_read(-1, 11, &LINUX), ReadStep::Timeout);
    assert_eq!(classify_read(-1, 11, &DARWIN), ReadStep::Error(11));
    assert_eq!(classify_read(-1, 35, &DARWIN), ReadStep::Timeout);
    assert_eq!(classify_read(-1, 35, &LINUX), ReadStep::Error(35));
}

#[test]
fn write_classification_loops_then_fails() {
    let a = &LINUX;
    assert_eq!(classify_write(3, 0, a), WriteStep::Advance(3));
    assert_eq!(classify_write(0, 0, a), WriteStep::Fail(0));
    assert_eq!(classify_write(-1, a.eintr, a), WriteStep::Retry);
    assert_eq!(classify_write(-1, a.eagain, a), WriteStep::Retry);
    assert_eq!(classify_write(-1, 9, a), WriteStep::Fail(9));
}

/// Driving the write loop purely through `classify_write` proves the
/// short-write handling without a file descriptor.
#[test]
fn a_short_write_loop_terminates() {
    let total = 10usize;
    let mut off = 0usize;
    let mut steps = 0;
    while off < total {
        steps += 1;
        assert!(steps < 100, "loop did not converge");
        let n = if steps == 2 { -1 } else { 4 };
        match classify_write(n, LINUX.eintr, &LINUX) {
            WriteStep::Advance(k) => off += k.min(total - off),
            WriteStep::Retry => continue,
            WriteStep::Fail(e) => panic!("unexpected failure {e}"),
        }
    }
    assert_eq!(off, total);
}

/// `O_NOCTTY` is the one `open` flag that differs, and getting it wrong is
/// quiet: `0o400` on Darwin is `O_NOFOLLOW`, which would *usually* still open
/// `/dev/tty` and would leave the tty able to become the controlling terminal.
#[test]
fn open_flags_differ_between_the_two_unixes() {
    assert_eq!(LINUX_POSIX.o_rdwr, 2);
    assert_eq!(DARWIN_POSIX.o_rdwr, 2);
    assert_eq!(LINUX_POSIX.o_noctty, 0o400);
    assert_eq!(DARWIN_POSIX.o_noctty, 0x2_0000);
    assert_ne!(LINUX_POSIX.o_noctty, DARWIN_POSIX.o_noctty);
    // Neither table may accidentally collide O_RDWR with O_NOCTTY.
    assert_eq!(LINUX_POSIX.o_rdwr & LINUX_POSIX.o_noctty, 0);
    assert_eq!(DARWIN_POSIX.o_rdwr & DARWIN_POSIX.o_noctty, 0);
}

/// The five signals this reader catches happen to have the same numbers on
/// Linux and Darwin — asserted rather than assumed, since `SIGWINCH` in
/// particular is 20 on some other unixes and 28 on these two.
#[test]
fn the_caught_signal_numbers_agree_across_the_two_unixes() {
    for p in [LINUX_POSIX, DARWIN_POSIX] {
        assert_eq!(p.sighup, 1);
        assert_eq!(p.sigint, 2);
        assert_eq!(p.sigquit, 3);
        assert_eq!(p.sigterm, 15);
        assert_eq!(p.sigwinch, 28);
    }
    assert_eq!(posix_for(Os::Linux), LINUX_POSIX);
    assert_eq!(posix_for(Os::Darwin), DARWIN_POSIX);
    // Distinct numbers: installing two handlers on one signal would lose one.
    let p = LINUX_POSIX;
    let mut v = [p.sighup, p.sigint, p.sigquit, p.sigterm, p.sigwinch];
    v.sort_unstable();
    v.windows(2).for_each(|w| assert_ne!(w[0], w[1]));
}

/// The Darwin table as a whole, cross-checked field by field against
/// `xnu/bsd/sys/{termios,errno}.h`, with an explicit note of which values
/// coincide with Linux and which do not. A copy-paste of the Linux block would
/// fail six of these.
#[test]
fn darwin_and_linux_agree_only_where_the_headers_do() {
    // Same on both.
    for f in [
        (LINUX.brkint, DARWIN.brkint),
        (LINUX.inpck, DARWIN.inpck),
        (LINUX.istrip, DARWIN.istrip),
        (LINUX.icrnl, DARWIN.icrnl),
        (LINUX.opost, DARWIN.opost),
        (LINUX.echo, DARWIN.echo),
    ] {
        assert_eq!(f.0, f.1, "these are equal in both headers");
    }
    // Different on each — the trap this table exists to avoid.
    for f in [
        (LINUX.ixon, DARWIN.ixon),
        (LINUX.isig, DARWIN.isig),
        (LINUX.icanon, DARWIN.icanon),
        (LINUX.iexten, DARWIN.iexten),
        (LINUX.csize, DARWIN.csize),
        (LINUX.cs8, DARWIN.cs8),
    ] {
        assert_ne!(f.0, f.1, "these differ between the headers");
    }
    // CS8 fills CSIZE on both, which is why raw mode can just OR it in.
    assert_eq!(LINUX.cs8 & LINUX.csize, LINUX.cs8);
    assert_eq!(DARWIN.cs8 & DARWIN.csize, DARWIN.cs8);
}
