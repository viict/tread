#!/usr/bin/env python3
"""Render a real tread session to an SVG, for the README.

Drives the binary through a pty, replays the escape sequences it paints into a
cell grid, and writes that grid as SVG text. The picture is therefore the
program's actual output rather than a mock-up, and regenerating it after a
change to the theme or the layout is one command.

Only the small part of the terminal protocol tread uses is implemented: absolute
cursor positioning, erase-to-end-of-line, and SGR colour/attribute changes.

Usage:
  tools/screenshot.py <binary> <file> <out.svg> [--keys jjj] [--cols 92] [--rows 26]
"""
import argparse
import os
import pty
import re
import select
import sys
import time

CSI = re.compile(rb"\x1b\[([0-9;?]*)([a-zA-Z])")
OSC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

# A dark palette in the same family as a default terminal.
BG = "#12141a"
FG = "#d0d4dc"


def xterm256(i):
    """xterm-256 index -> #rrggbb."""
    base = [
        (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16),
        (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
        (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67),
        (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
    ]
    if i < 16:
        r, g, b = base[i]
    elif i < 232:
        i -= 16
        levels = [0, 95, 135, 175, 215, 255]
        r, g, b = levels[i // 36], levels[(i // 6) % 6], levels[i % 6]
    else:
        v = 8 + (i - 232) * 10
        r = g = b = v
    return f"#{r:02x}{g:02x}{b:02x}"


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "italic", "underline")

    def __init__(self):
        self.ch = " "
        self.fg = None
        self.bg = None
        self.bold = self.dim = self.italic = self.underline = False


class Screen:
    """Just enough terminal to replay what tread paints."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.grid = [[Cell() for _ in range(cols)] for _ in range(rows)]
        self.r = self.c = 0
        self.reset_style()

    def reset_style(self):
        self.fg = self.bg = None
        self.bold = self.dim = self.italic = self.underline = self.reverse = False

    def sgr(self, params):
        vals = [int(p) for p in params.split(b";") if p != b""] or [0]
        i = 0
        while i < len(vals):
            v = vals[i]
            if v == 0:
                self.reset_style()
            elif v == 1:
                self.bold = True
            elif v == 2:
                self.dim = True
            elif v == 3:
                self.italic = True
            elif v == 4:
                self.underline = True
            elif v == 7:
                self.reverse = True
            elif v == 22:
                self.bold = self.dim = False
            elif v == 23:
                self.italic = False
            elif v == 24:
                self.underline = False
            elif v == 27:
                self.reverse = False
            elif v == 39:
                self.fg = None
            elif v == 49:
                self.bg = None
            elif v in (38, 48) and i + 2 < len(vals) and vals[i + 1] == 5:
                colour = xterm256(vals[i + 2])
                if v == 38:
                    self.fg = colour
                else:
                    self.bg = colour
                i += 2
            elif 30 <= v <= 37:
                self.fg = xterm256(v - 30)
            elif 90 <= v <= 97:
                self.fg = xterm256(v - 90 + 8)
            i += 1

    def put(self, ch):
        if self.r >= self.rows or self.c >= self.cols:
            return
        cell = self.grid[self.r][self.c]
        cell.ch = ch
        fg, bg = self.fg, self.bg
        if self.reverse:
            fg, bg = bg or BG, fg or FG
        cell.fg, cell.bg = fg, bg
        cell.bold, cell.dim = self.bold, self.dim
        cell.italic, cell.underline = self.italic, self.underline
        self.c += 1

    def feed(self, data):
        data = OSC.sub(b"", data)
        i = 0
        while i < len(data):
            m = CSI.match(data, i)
            if m:
                self.csi(m.group(1), m.group(2))
                i = m.end()
                continue
            b = data[i : i + 1]
            if b == b"\x1b":
                i += 2
                continue
            if b == b"\r":
                self.c = 0
                i += 1
                continue
            if b == b"\n":
                self.r, self.c = self.r + 1, 0
                i += 1
                continue
            # Decode one UTF-8 character.
            n = 1
            while i + n < len(data) and (data[i + n] & 0xC0) == 0x80:
                n += 1
            self.put(data[i : i + n].decode("utf8", "replace"))
            i += n

    def csi(self, params, verb):
        if verb == b"m":
            self.sgr(params)
        elif verb == b"H":
            parts = params.split(b";")
            self.r = max(0, int(parts[0] or 1) - 1)
            self.c = max(0, int(parts[1] or 1) - 1) if len(parts) > 1 else 0
        elif verb == b"K":
            for c in range(self.c, self.cols):
                cell = self.grid[self.r][c]
                cell.ch, cell.fg = " ", None
                cell.bg = self.bg


def capture(binary, path, keys, cols, rows, settle=1.6):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(COLUMNS=str(cols), LINES=str(rows), TERM="xterm-256color")
        os.execv(binary, [binary, path])
    import fcntl
    import struct
    import termios

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out = bytearray()
    deadline = time.monotonic() + settle
    sent = False
    while time.monotonic() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        elif not sent and keys:
            os.write(fd, keys.encode())
            sent = True
            deadline = time.monotonic() + settle
    try:
        os.write(fd, b"q")
        os.waitpid(pid, 0)
    except OSError:
        pass
    return bytes(out)


def to_svg(screen, cw=8.4, ch=18.0, pad=14):
    w = screen.cols * cw + pad * 2
    h = screen.rows * ch + pad * 2
    esc = lambda s: s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0f}" height="{h:.0f}" '
        f'viewBox="0 0 {w:.0f} {h:.0f}" font-family="ui-monospace,SFMono-Regular,'
        f'Menlo,Consolas,monospace" font-size="13">',
        f'<rect width="{w:.0f}" height="{h:.0f}" rx="8" fill="{BG}"/>',
    ]
    # Backgrounds first, merged into runs so the file stays small.
    for r, row in enumerate(screen.grid):
        c = 0
        while c < screen.cols:
            bg = row[c].bg
            if bg is None:
                c += 1
                continue
            start = c
            while c < screen.cols and row[c].bg == bg:
                c += 1
            x = pad + start * cw
            y = pad + r * ch
            parts.append(
                f'<rect x="{x:.1f}" y="{y:.1f}" width="{(c - start) * cw:.1f}" '
                f'height="{ch:.1f}" fill="{bg}"/>'
            )
    # Then text, one run per contiguous identical style.
    for r, row in enumerate(screen.grid):
        c = 0
        while c < screen.cols:
            cell = row[c]
            if cell.ch == " ":
                c += 1
                continue
            key = (cell.fg, cell.bold, cell.dim, cell.italic, cell.underline)
            start, text = c, []
            while c < screen.cols:
                cur = row[c]
                if (cur.fg, cur.bold, cur.dim, cur.italic, cur.underline) != key:
                    break
                text.append(cur.ch)
                c += 1
            run = "".join(text).rstrip()
            if not run:
                continue
            x = pad + start * cw
            y = pad + r * ch + ch * 0.75
            attrs = [f'x="{x:.1f}"', f'y="{y:.1f}"']
            attrs.append(f'fill="{cell.fg or FG}"')
            if cell.bold:
                attrs.append('font-weight="bold"')
            if cell.italic:
                attrs.append('font-style="italic"')
            if cell.underline:
                attrs.append('text-decoration="underline"')
            if cell.dim:
                attrs.append('opacity="0.65"')
            attrs.append('xml:space="preserve"')
            parts.append(f'<text {" ".join(attrs)}>{esc(run)}</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("file")
    ap.add_argument("out")
    ap.add_argument("--keys", default="")
    ap.add_argument("--cols", type=int, default=92)
    ap.add_argument("--rows", type=int, default=26)
    a = ap.parse_args()

    data = capture(a.binary, a.file, a.keys, a.cols, a.rows)
    screen = Screen(a.cols, a.rows)
    # Only what was painted after entering the alternate screen.
    if b"\x1b[?1049h" in data:
        data = data.split(b"\x1b[?1049h", 1)[1]
    screen.feed(data)
    with open(a.out, "w", encoding="utf8") as fh:
        fh.write(to_svg(screen))
    print(f"wrote {a.out} ({a.cols}x{a.rows})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
