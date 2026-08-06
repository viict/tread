#!/usr/bin/env python3
"""Interactive soak: drive tread through a real pty over a corpus.

Unlike tools/soak.sh (which exercises the non-interactive dump path) this runs
the pager itself, so it covers raw mode, the frame buffer, scrolling, collapse,
search, selection and teardown. It fails on a non-zero exit, a hang, a panic, a
terminal left un-restored, or any mouse-tracking sequence.

Usage: tools/soak_pty.py <binary> <dir> [max-files]
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

KEYS = (
    b"jjjjkk"          # line scroll
    b"dddduu"          # half pages
    b"  b"             # page down / up
    b"G g"             # bottom / top
    b"lllhhh"          # horizontal scroll (whole columns in a CSV)
    b"wl w h"          # widen the column under the cursor
    b"\t\t\t"          # next heading
    b"za za"           # toggle collapse
    b"zMzR"            # collapse all / expand all
    b"nn"              # next link
    b"o\x1b"           # outline overlay, then escape
    b"/the\r" b"nnN"   # search
    b"vjjy"            # visual select + yank
    b"Yc"              # yank section / code block
    b"i\x1b"           # index overlay
    b"?\x1b"           # help overlay
    b"\x7f"            # backspace / history pop
    b"q" b"q" b"q"     # quit (pops nav stack first if deep)
)

MOUSE = re.compile(rb"\x1b\[\?(1000|1002|1003|1006|1015)[hl]")
BAD_CTRL = re.compile(rb"[\x01-\x08\x0b\x0c\x0e-\x1a\x1c-\x1f\x7f]")
# OSC first (it may legitimately carry a BEL or ESC-backslash terminator),
# then CSI and charset selection. Whatever is left must contain no ESC.
OSC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", re.S)
CSI = re.compile(rb"\x1b(?:\[[0-9;?]*[ -/]*[@-~]|[()][B0]|[=>78MD])")


def drive(binary, path, cols=100, rows=40, budget=15.0):
    """Run one document. Returns a list of problem strings (empty == pass)."""
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execv(binary, [binary, path])
        os._exit(127)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out = bytearray()
    problems = []
    deadline = time.time() + budget
    sent = False
    status = None
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                chunk = os.read(fd, 1 << 16)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
            if not sent:
                os.write(fd, KEYS)
                sent = True
        done, st = os.waitpid(pid, os.WNOHANG)
        if done == pid:
            status = st
            break
    if status is None:
        # The pty master reports EIO as soon as the child closes it, so give the
        # child a moment to be reaped before calling it a hang.
        for _ in range(20):
            done, st = os.waitpid(pid, os.WNOHANG)
            if done == pid:
                status = st
                break
            time.sleep(0.05)
    if status is None:
        problems.append("HANG")
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    elif not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        problems.append("exit=%r" % (status,))
    # Drain whatever the child wrote between the last read and its exit --
    # that tail is exactly where the teardown sequences live.
    while True:
        r, _, _ = select.select([fd], [], [], 0.1)
        if not r:
            break
        try:
            chunk = os.read(fd, 1 << 16)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
    os.close(fd)

    text = bytes(out)
    if b"panicked at" in text:
        problems.append("PANIC")
    if MOUSE.search(text):
        problems.append("MOUSE CAPTURE")
    if b"\x1b[?1049h" in text and b"\x1b[?1049l" not in text:
        problems.append("alt screen not exited")
    if b"\x1b[?25l" in text and b"\x1b[?25h" not in text:
        problems.append("cursor left hidden")
    stripped = CSI.sub(b"", OSC.sub(b"", text))
    if b"\x1b" in stripped:
        i = stripped.index(b"\x1b")
        problems.append("stray ESC: %r" % stripped[i:i + 16])
    # \r is legitimate on a pty (the line discipline adds it); other C0 are not.
    hit = BAD_CTRL.search(stripped)
    if hit:
        problems.append("raw control byte: %r" % hit.group())
    return problems


def drive_signal(binary, path, sig, cols=100, rows=40):
    """Start tread, deliver `sig`, and check the terminal came back.

    The default disposition of SIGTERM/SIGHUP/SIGQUIT kills the process without
    running Drop or the panic hook (the release profile is panic=abort), which
    would strand the tty in raw mode on the alternate screen. tread catches them
    and quits through the normal teardown instead.
    """
    problems = []
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execv(binary, [binary, path])
        os._exit(127)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    before = termios.tcgetattr(fd)
    out = bytearray()
    deadline = time.time() + 5.0
    # Wait for the first frame, so raw mode and the alt screen are definitely up.
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            out += os.read(fd, 1 << 16)
            if b"\x1b[?1049h" in bytes(out):
                break
    raw = termios.tcgetattr(fd)
    if raw[3] & termios.ICANON:
        problems.append("%s: raw mode never entered" % sig)
    os.kill(pid, sig)
    status = None
    while time.time() < deadline:
        try:
            r, _, _ = select.select([fd], [], [], 0.2)
            if r:
                chunk = os.read(fd, 1 << 16)
                if chunk:
                    out += chunk
        except OSError:
            pass
        done, st = os.waitpid(pid, os.WNOHANG)
        if done == pid:
            status = st
            break
    after = None
    try:
        after = termios.tcgetattr(fd)
    except termios.error:
        pass
    os.close(fd)
    if status is None:
        problems.append("%s: HANG" % sig)
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    elif os.WIFSIGNALED(status):
        problems.append("%s: killed by signal %d, teardown never ran"
                        % (sig, os.WTERMSIG(status)))
    text = bytes(out)
    if b"\x1b[?1049h" in text and b"\x1b[?1049l" not in text:
        problems.append("%s: alt screen not exited" % sig)
    if b"\x1b[?25l" in text and b"\x1b[?25h" not in text:
        problems.append("%s: cursor left hidden" % sig)
    if after is not None and after[3] != before[3]:
        problems.append("%s: termios lflag not restored (%s -> %s)"
                        % (sig, before[3], after[3]))
    return problems


def main():
    binary, root = sys.argv[1], sys.argv[2]
    limit = int(sys.argv[3]) if len(sys.argv) > 3 else 0
    docs = sorted(
        os.path.join(d, f)
        for d, _, fs in os.walk(root)
        for f in fs
        if f.endswith((".md", ".csv", ".tsv"))
    )
    if limit:
        docs = docs[:limit]
    failed = 0
    for doc in docs:
        problems = drive(binary, doc)
        if problems:
            failed += 1
            print("FAIL %s: %s" % (doc, "; ".join(problems)))
    print("pty soak: %d documents, %d failures" % (len(docs), failed))
    sig_failed = 0
    if docs:
        for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT, signal.SIGINT):
            problems = drive_signal(binary, docs[0], sig)
            if problems:
                sig_failed += 1
                print("FAIL signal %s: %s" % (sig, "; ".join(problems)))
        print("signal teardown: 4 signals, %d failures" % sig_failed)
    return 1 if failed or sig_failed else 0


if __name__ == "__main__":
    sys.exit(main())
