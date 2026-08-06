#!/usr/bin/env python3
"""Measure interactive latency of mdr under a real pty.

Usage: tools/latency.py <mdr-binary> <document.md>

Prints time-to-first-frame and the response time of each key command. Use it
to confirm a very large document (10 MB+) still paints and scrolls promptly.
"""
import os, pty, select, sys, time, termios, struct, fcntl, signal

binary, path = sys.argv[1], sys.argv[2]
pid, fd = pty.fork()
if pid == 0:
    os.execv(binary, [binary, path])
    os._exit(127)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 110, 0, 0))
t0 = time.time()

def drain(timeout):
    """Read until quiet for `timeout` seconds; return (first_byte_delay, total)."""
    first = None
    total = 0
    start = time.time()
    while True:
        r, _, _ = select.select([fd], [], [], timeout)
        if not r:
            return first, total
        try:
            c = os.read(fd, 1 << 16)
        except OSError:
            return first, total
        if not c:
            return first, total
        if first is None:
            first = time.time() - start
        total += len(c)

first, n = drain(8.0)
print("first frame:   %.3fs  (%d bytes)" % (first, n))

for label, keys in [("G (bottom)", b"G"), ("g (top)", b"g"),
                    ("d x20", b"d" * 20), ("/alpha<CR>", b"/alpha\r"),
                    ("n x10", b"n" * 10), ("zM", b"zM"), ("zR", b"zR"),
                    ("o (outline)", b"o"), ("Esc", b"\x1b")]:
    t = time.time()
    os.write(fd, keys)
    f, n = drain(6.0)
    print("%-12s resp %.3fs  (%d bytes)" % (label, (f if f is not None else -1), n))

os.write(fd, b"q")
time.sleep(0.4)
try:
    _, status = os.waitpid(pid, os.WNOHANG)
    print("exit:", os.WEXITSTATUS(status) if os.WIFEXITED(status) else status)
except ChildProcessError:
    print("exit: reaped")
else:
    pass
try:
    os.kill(pid, 0)
    os.kill(pid, signal.SIGKILL)
    print("STILL RUNNING after q -- HANG")
except (ProcessLookupError, OSError):
    pass
print("total wall: %.3fs" % (time.time() - t0))
