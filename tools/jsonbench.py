#!/usr/bin/env python3
"""Measure how tread behaves on a .json / .jsonl file, under a real pty.

The JSON sibling of tools/csvbench.py, and it reuses that file's `Session` so
both measure the same way: from a pty, from process start, with a terminal on
the other end. Called by tools/soak_json.sh; usable on its own:

  tools/jsonbench.py <binary> <file> [file ...]
  tools/jsonbench.py --lens agent <binary> <file>

Reports, in milliseconds and KiB:

  open_ms        exec to the first byte of the first frame
  paint_ms       exec to the frame going quiet (the whole first screen)
  rss_open       peak RSS after the first screen
  scroll_ms      response to 40 half-page downs
  expand_ms      response to zR -- expand every container
  rss_expand     peak RSS after that
  G_ms           response to `G`, which drives the structural walk to the end
  G_done_ms      `G` to the screen going quiet again
  rss_peak       peak RSS over the whole session
  quit_ms        `q` to process exit, measured on a freshly opened file
  Gquit_ms       `q` 200ms into a `G` walk, to process exit (interruptibility)

The claim under test is that open_ms, quit_ms and rss_open do not track file
size.
"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from csvbench import Session, ms, OPEN_BUDGET, SCAN_BUDGET, QUIET  # noqa: E402


def bench(binary, path, extra=()):
    r = {"file": path, "bytes": os.path.getsize(path)}

    # -- open, scroll, expand, G, on one session ----------------------------
    s = Session(binary, path, extra)
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

    # zR: every container open. On a lazily indexed tree this is the keystroke
    # that can turn a cheap document into an expensive one.
    t = time.time()
    s.send(b"zR")
    f, _, _ = s.drain(60.0, quiet=1.0)
    r["expand_ms"] = ms(f)
    r["expand_done_ms"] = (time.time() - t - 1.0) * 1000.0
    s.sample()
    r["rss_expand"] = s.peak

    t = time.time()
    s.send(b"G")
    f, _, text = s.drain(SCAN_BUDGET, quiet=1.0)
    r["G_ms"] = ms(f)
    r["G_done_ms"] = (time.time() - t - 1.0) * 1000.0
    s.sample()
    r["rss_peak"] = s.peak
    t = time.time()
    s.send(b"q")
    took, st = s.wait()
    r["quit_after_G_ms"] = took * 1000.0
    r["exit"] = "timeout" if st is None else (
        os.WEXITSTATUS(st) if os.WIFEXITED(st) else "signal %r" % st)
    r["panic"] = b"panicked at" in bytes(s.out)
    s.kill()

    # -- q straight after the first screen -----------------------------------
    s2 = Session(binary, path, extra)
    s2.drain(OPEN_BUDGET)
    t = time.time()
    s2.send(b"q")
    took, st2 = s2.wait()
    r["quit_ms"] = took * 1000.0
    r["quit_exit"] = "timeout" if st2 is None else (
        os.WEXITSTATUS(st2) if os.WIFEXITED(st2) else "signal %r" % st2)
    r["panic"] = r["panic"] or b"panicked at" in bytes(s2.out)
    s2.kill()

    # -- q 200ms into a G walk: is the walk interruptible? -------------------
    s3 = Session(binary, path, extra)
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
         "scroll_done_ms", "expand_ms", "expand_done_ms", "rss_expand",
         "G_ms", "G_done_ms", "rss_peak", "quit_ms", "quit_after_G_ms",
         "Gquit_ms", "exit", "quit_exit", "Gquit_exit", "panic"]


def main(argv):
    argv = list(argv[1:])
    extra = []
    while argv and argv[0].startswith("--"):
        extra.append(argv.pop(0))
        if extra[-1] in ("--lens", "--format", "--width"):
            extra.append(argv.pop(0))
    binary = argv[0]
    for path in argv[1:]:
        r = bench(binary, path, tuple(extra))
        print("== %s%s" % (r["file"], (" " + " ".join(extra)) if extra else ""))
        for k in ORDER:
            v = r[k]
            print("   %-16s %s" % (k, ("%.1f" % v) if isinstance(v, float) else v))
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
