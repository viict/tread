#!/usr/bin/env python3
"""Deterministic CSV generator for tread's scale and hostile-input soaks.

Every file this writes is a pure function of its arguments: same command, same
bytes, on any machine. Nothing it produces is checked in -- generate into a
temp dir (tools/soak_csv.sh does) and delete it after.

Usage:
  tools/csvgen.py rows --rows N [--cols M] [--delim ,] [--quote-every K] OUT
  tools/csvgen.py case NAME OUT
  tools/csvgen.py cases DIR          write every hostile case into DIR
  tools/csvgen.py list               name every hostile case

The `rows` shape is a plausible business table: an ascending id, a name, an
email, an amount, a date, a flag and a free-text note, cycled out to --cols.
--quote-every K quotes (and puts a comma and a newline inside) every Kth row's
note, so the row index's quoting rules are exercised at scale rather than only
in unit tests.
"""
import os
import sys

# ---------------------------------------------------------------- row shapes

NAMES = ["ada", "grace", "alan", "edsger", "barbara", "donald", "ken", "bjarne",
         "niklaus", "john", "tony", "leslie", "fran", "jean", "maurice", "kathleen"]
DOMAINS = ["example.com", "test.org", "sample.net", "corp.internal"]
CITIES = ["reykjavik", "sao paulo", "kyoto", "nairobi", "lisbon", "quito"]
NOTES = ["ok", "needs review", "escalated", "closed by owner", "duplicate",
         "waiting on customer", "auto-resolved", "reopened after audit"]
HEADERS = ["id", "name", "email", "amount", "date", "flag", "note", "city"]


def header(cols, delim):
    out = []
    for i in range(cols):
        base = HEADERS[i % len(HEADERS)]
        out.append(base if i < len(HEADERS) else "%s_%d" % (base, i // len(HEADERS)))
    return delim.join(out) + "\n"


def cell(col, i):
    """One deterministic cell value. `i` is the row number."""
    which = col % len(HEADERS)
    if which == 0:
        return str(i)
    if which == 1:
        return NAMES[i % len(NAMES)]
    if which == 2:
        return "%s%d@%s" % (NAMES[(i + col) % len(NAMES)], i % 1000,
                            DOMAINS[(i + col) % len(DOMAINS)])
    if which == 3:
        return "%d.%02d" % ((i * 37) % 100000, (i * 7) % 100)
    if which == 4:
        return "20%02d-%02d-%02d" % (10 + i % 15, 1 + i % 12, 1 + i % 28)
    if which == 5:
        return "true" if (i + col) % 3 else "false"
    if which == 6:
        return NOTES[(i + col) % len(NOTES)]
    return CITIES[(i + col) % len(CITIES)]


def gen_rows(out, rows, cols, delim, quote_every):
    """Stream `rows` data rows to the open binary file `out`."""
    out.write(header(cols, delim).encode())
    buf = []
    size = 0
    for i in range(rows):
        fields = [cell(c, i) for c in range(cols)]
        if quote_every and i % quote_every == 0:
            fields[-1] = '"%s%s embedded\nnewline"' % (fields[-1], delim)
        buf.append(delim.join(fields))
        if len(buf) >= 4096:
            blob = ("\n".join(buf) + "\n").encode()
            out.write(blob)
            size += len(blob)
            buf = []
    if buf:
        blob = ("\n".join(buf) + "\n").encode()
        out.write(blob)
        size += len(blob)
    return size


# ------------------------------------------------------------ hostile cases

MB = 1024 * 1024


def case_big_cell(out):
    """A single 50MB cell: one row whose second field is 50MB of text."""
    out.write(b"id,blob,tail\n")
    out.write(b"1,")
    chunk = b"x" * (1 * MB)
    for _ in range(50):
        out.write(chunk)
    out.write(b",end\n")
    out.write(b"2,small,end\n")


def case_wide(out):
    """10,000 columns."""
    n = 10000
    out.write((",".join("c%d" % i for i in range(n)) + "\n").encode())
    for r in range(20):
        out.write((",".join("%d-%d" % (r, i) for i in range(n)) + "\n").encode())


def case_big_header(out):
    """A 10MB header row, then ordinary data."""
    # ~10MB spread over many columns rather than one, so the grid has to size
    # a header it can never show.
    names = []
    total = 0
    i = 0
    while total < 10 * MB:
        name = "column_%06d_%s" % (i, "n" * 90)
        names.append(name)
        total += len(name) + 1
        i += 1
    out.write((",".join(names) + "\n").encode())
    for r in range(5):
        out.write((",".join(str(r) for _ in range(min(50, len(names)))) + "\n").encode())


def case_unterminated_quote(out):
    out.write(b'id,name\n1,"open quote never closed\n2,still inside\n3,eof here')


def case_stray_quote(out):
    out.write(b'id,name\n1,ab"cd\n2,"quoted"tail,x\n3,plain\n')


def case_nuls(out):
    out.write(b"id,name\n1,a\x00b\n2,\x00\x00\x00\n3,ok\n")


def case_invalid_utf8(out):
    out.write(b"id,name\n1,\xff\xfe\xfd\n2,caf\xe9\n3,ok\n")


def case_utf16le(out):
    text = "id,name\n1,alice\n2,bob\n3,carol\n"
    out.write(b"\xff\xfe" + text.encode("utf-16-le"))


def case_utf16be(out):
    text = "id,name\n1,alice\n2,bob\n3,carol\n"
    out.write(b"\xfe\xff" + text.encode("utf-16-be"))


def case_crlf(out):
    out.write(b"id,name\r\n1,alice\r\n2,bob\r\n")


def case_bare_cr(out):
    out.write(b"id,name\r1,alice\r2,bob\r")


def case_no_trailing_newline(out):
    out.write(b"id,name\n1,alice\n2,bob")


def case_empty(out):
    pass


def case_header_only(out):
    out.write(b"id,name,amount\n")


def case_delims_only(out):
    out.write(b",,,,\n" * 200)


def case_one_long_line(out):
    """No newline at all: a 20MB single row."""
    out.write(b"a" * (20 * MB))


def case_bom_crlf_quoted(out):
    out.write("﻿".encode() + b'id,name\r\n1,"multi\r\nline"\r\n2,plain\r\n')


CASES = {
    "big_cell": case_big_cell,
    "wide_10k_cols": case_wide,
    "big_header": case_big_header,
    "unterminated_quote": case_unterminated_quote,
    "stray_quote": case_stray_quote,
    "nuls": case_nuls,
    "invalid_utf8": case_invalid_utf8,
    "crlf": case_crlf,
    "bare_cr": case_bare_cr,
    "no_trailing_newline": case_no_trailing_newline,
    "empty": case_empty,
    "header_only": case_header_only,
    "delims_only": case_delims_only,
    "one_long_line": case_one_long_line,
    "bom_crlf_quoted": case_bom_crlf_quoted,
}

# Cases tread must *refuse*, not render: an encoding it cannot read is the one
# input where opening the file is worse than saying why it did not. They are
# kept out of CASES so the render soaks (which fail on a non-zero exit) do not
# have to know about them; soak_csv.sh checks them by hand.
REFUSALS = {
    "utf16le": case_utf16le,
    "utf16be": case_utf16be,
}


# ------------------------------------------------------------------- driver

def main(argv):
    if len(argv) < 2:
        sys.stderr.write(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "list":
        for name in sorted(CASES):
            print(name)
        for name in sorted(REFUSALS):
            print("%s (refused)" % name)
        return 0
    if cmd in ("cases", "refusals"):
        d = argv[2]
        which = CASES if cmd == "cases" else REFUSALS
        os.makedirs(d, exist_ok=True)
        for name in sorted(which):
            path = os.path.join(d, name + ".csv")
            with open(path, "wb") as f:
                which[name](f)
            print("%s %d" % (path, os.path.getsize(path)))
        return 0
    if cmd == "case":
        name, path = argv[2], argv[3]
        make = CASES.get(name) or REFUSALS.get(name)
        if make is None:
            sys.stderr.write("unknown case: %s\n" % name)
            return 2
        with open(path, "wb") as f:
            make(f)
        print("%s %d" % (path, os.path.getsize(path)))
        return 0
    if cmd == "rows":
        rows, cols, delim, quote_every, path = 1000, 8, ",", 0, None
        i = 2
        while i < len(argv):
            a = argv[i]
            if a == "--rows":
                rows = int(argv[i + 1]); i += 2
            elif a == "--cols":
                cols = int(argv[i + 1]); i += 2
            elif a == "--delim":
                delim = argv[i + 1]; i += 2
            elif a == "--quote-every":
                quote_every = int(argv[i + 1]); i += 2
            else:
                path = a; i += 1
        if path is None:
            sys.stderr.write("rows: no output path\n")
            return 2
        with open(path, "wb", buffering=1 << 20) as f:
            gen_rows(f, rows, cols, delim, quote_every)
        print("%s %d" % (path, os.path.getsize(path)))
        return 0
    sys.stderr.write("unknown command: %s\n" % cmd)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
