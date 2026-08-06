#!/usr/bin/env python3
"""Measure how tread behaves on a CSV, under a real pty.

Called by tools/soak_csv.sh; usable on its own:

  tools/csvbench.py <binary> <file.csv> [--rows-hint N]

Reports, in milliseconds and KiB, all measured the way a reader experiences
them -- from a pty, from process start, with the terminal on the other end:

  open_ms      exec to the first byte of the first frame
  paint_ms     exec to the frame going quiet (the whole first screen)
  rss_open     peak RSS after the first screen
  scroll_ms    response to 40 half-page downs
  G_ms         response to `G`, which forces the row-index scan
  G_done_ms    `G` to the screen going quiet again
  G_progress   whether the status bar counted while the scan ran
  rss_peak     peak RSS over the whole session
  quit_ms      `q` to process exit, measured on a freshly opened file
  Gquit_ms     `q` 200ms into a `G` scan, to process exit (interruptibility)

The claim under test is that none of these track file size.
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time

COLS, ROWS = 120, 40
QUIET = 0.35          # seconds of silence that end a frame burst
OPEN_BUDGET = 30.0
SCAN_BUDGET = 600.0


def vmhwm(pid):
    """Peak RSS of `pid` in KiB, or 0 when it cannot be read (not Linux, or
    the process already exited)."""
    try:
        with open("/proc/%d/status" % pid) as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


class Session:
    """One tread process on the other end of a pty."""

    def __init__(self, binary, path, extra=()):
        self.peak = 0
        self.out = bytearray()
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["LINES"], os.environ["COLUMNS"] = str(ROWS), str(COLS)
            os.execv(binary, [binary, path, *extra])
            os._exit(127)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                    struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.t0 = time.time()

    def sample(self):
        self.peak = max(self.peak, vmhwm(self.pid))

    def drain(self, budget, quiet=QUIET):
        """Read until `quiet` seconds of silence or `budget` runs out.
        Returns (seconds to first byte, total bytes, text)."""
        first = None
        start = time.time()
        got = bytearray()
        while time.time() - start < budget:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            self.sample()
            if not r:
                if first is not None and time.time() - last > quiet:
                    break
                continue
            try:
                chunk = os.read(self.fd, 1 << 16)
            except OSError:
                break
            if not chunk:
                break
            if first is None:
                first = time.time() - start
            last = time.time()
            got += chunk
        self.out += got
        return first, len(got), bytes(got)

    def send(self, keys):
        os.write(self.fd, keys)

    def wait(self, budget=20.0):
        """Wait for exit; returns (seconds, status or None on timeout)."""
        start = time.time()
        while time.time() - start < budget:
            self.sample()
            try:
                r, _, _ = select.select([self.fd], [], [], 0.02)
                if r:
                    chunk = os.read(self.fd, 1 << 16)
                    if chunk:
                        self.out += chunk
            except OSError:
                pass
            done, st = os.waitpid(self.pid, os.WNOHANG)
            if done == self.pid:
                return time.time() - start, st
        return time.time() - start, None

    def kill(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError, OSError):
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass


PROGRESS = re.compile(rb"indexing\s+(\d+)%")
POSITION = re.compile(rb"row ([0-9]+)/([^ \x1b]*)")


def ms(x):
    return -1.0 if x is None else x * 1000.0


def bench(binary, path):
    r = {"file": path, "bytes": os.path.getsize(path)}

    # -- open, scroll, G, on one session ------------------------------------
    s = Session(binary, path)
    first, _, _ = s.drain(OPEN_BUDGET)
    r["open_ms"] = ms(first)
    r["paint_ms"] = (time.time() - s.t0 - QUIET) * 1000.0
    s.sample()
    r["rss_open"] = s.peak

    t = time.time()
    s.send(b"d" * 40)
    f, _, _ = s.drain(30.0)
    r["scroll_ms"] = ms(f)
    r["scroll_done_ms"] = (time.time() - t - QUIET) * 1000.0

    t = time.time()
    s.send(b"G")
    f, _, text = s.drain(SCAN_BUDGET, quiet=1.0)
    r["G_ms"] = ms(f)
    r["G_done_ms"] = (time.time() - t - 1.0) * 1000.0
    pcts = sorted(set(int(m) for m in PROGRESS.findall(text)))
    r["G_progress"] = "%d ticks %s" % (
        len(pcts), ("%d..%d%%" % (pcts[0], pcts[-1])) if pcts else "-")
    seen = POSITION.findall(text)
    r["G_landed"] = ("row %s/%s" % (seen[-1][0].decode(), seen[-1][1].decode())
                     if seen else "-")
    s.sample()
    r["rss_peak"] = s.peak
    t = time.time()
    s.send(b"q")
    took, st = s.wait()
    r["quit_after_G_ms"] = took * 1000.0
    r["exit"] = "timeout" if st is None else (
        os.WEXITSTATUS(st) if os.WIFEXITED(st) else "signal %r" % st)
    body = bytes(s.out)
    r["panic"] = b"panicked at" in body
    s.kill()

    # -- q straight after the first screen -----------------------------------
    s2 = Session(binary, path)
    s2.drain(OPEN_BUDGET)
    t = time.time()
    s2.send(b"q")
    took, st2 = s2.wait()
    r["quit_ms"] = took * 1000.0
    r["quit_exit"] = "timeout" if st2 is None else (
        os.WEXITSTATUS(st2) if os.WIFEXITED(st2) else "signal %r" % st2)
    r["panic"] = r["panic"] or b"panicked at" in bytes(s2.out)
    s2.kill()

    # -- G pressed on the first screen, before the index has run -------------
    s4 = Session(binary, path)
    s4.drain(0.3, quiet=0.1)
    # Key latency *while* the background row index is running: this is what
    # "file size does not change how it feels" has to mean on the first screen
    # of a file that is still being indexed.
    busy = []
    for _ in range(5):
        t = time.time()
        s4.send(b"j")
        f, _, _ = s4.drain(5.0, quiet=0.12)
        busy.append((time.time() - t - 0.12) * 1000.0)
    r["busy_key_ms"] = "%.1f max of %s" % (max(busy), " ".join("%.0f" % b for b in busy))
    t = time.time()
    s4.send(b"G")
    f, _, text = s4.drain(SCAN_BUDGET, quiet=1.0)
    r["Gearly_ms"] = ms(f)
    r["Gearly_done_ms"] = (time.time() - t - 1.0) * 1000.0
    pcts = sorted(set(int(m) for m in PROGRESS.findall(text)))
    scan = sorted(set(int(m) for m in re.findall(rb"end of file\D+(\d+)%", text)))
    r["Gearly_progress"] = "%d index ticks, %d scan ticks %s" % (
        len(pcts), len(scan), ("%d..%d%%" % (scan[0], scan[-1])) if scan else "-")
    seen = POSITION.findall(text)
    r["Gearly_landed"] = ("row %s/%s" % (seen[-1][0].decode(), seen[-1][1].decode())
                          if seen else "-")
    r["rss_scan"] = s4.peak
    r["panic"] = r["panic"] or b"panicked at" in bytes(s4.out)
    s4.send(b"q")
    took, st4 = s4.wait()
    r["quit_after_scan_ms"] = took * 1000.0
    s4.kill()

    # -- q 200ms into a G scan: is the scan interruptible? -------------------
    s3 = Session(binary, path)
    s3.drain(0.3, quiet=0.1)
    s3.send(b"G")
    time.sleep(0.2)
    t = time.time()
    s3.send(b"q")
    took, st3 = s3.wait(budget=30.0)
    r["Gquit_ms"] = took * 1000.0
    r["Gquit_exit"] = "timeout" if st3 is None else (
        os.WEXITSTATUS(st3) if os.WIFEXITED(st3) else "signal %r" % st3)
    r["panic"] = r["panic"] or b"panicked at" in bytes(s3.out)
    s3.kill()
    return r


ORDER = ["bytes", "open_ms", "paint_ms", "rss_open", "scroll_ms",
         "scroll_done_ms", "G_ms", "G_done_ms", "G_progress", "G_landed",
         "rss_peak", "busy_key_ms", "Gearly_ms", "Gearly_done_ms", "Gearly_progress",
         "Gearly_landed", "rss_scan", "quit_ms", "quit_after_G_ms",
         "quit_after_scan_ms", "Gquit_ms", "exit", "quit_exit",
         "Gquit_exit", "panic"]


def main(argv):
    binary = argv[1]
    for path in argv[2:]:
        r = bench(binary, path)
        print("== %s" % r["file"])
        for k in ORDER:
            v = r[k]
            print("   %-16s %s" % (k, ("%.1f" % v) if isinstance(v, float) else v))
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
