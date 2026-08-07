#!/usr/bin/env python3
"""Deterministic JSON / JSONL generator for tread's scale and hostile-input soaks.

The sibling of tools/csvgen.py, and the same contract: every file this writes is
a pure function of its arguments -- same command, same bytes, on any machine.
Nothing it produces is checked in. Generate into a temp dir (tools/soak_json.sh
does) and delete it after; some of these are 50MB+.

Usage:
  tools/jsongen.py records --records N [--depth D] [--pad K] OUT.jsonl
  tools/jsongen.py doc --bytes N [--depth D] OUT.json
  tools/jsongen.py case NAME OUT
  tools/jsongen.py cases DIR         write every hostile case into DIR
  tools/jsongen.py list              name every hostile case

`records` writes a plausible log/trajectory shape: an ascending seq, a
timestamp, a type, a nested payload and a free-text message, one record per
line. --depth nests the payload D levels; --pad K pads the message to K bytes,
so "one 41KB line" is reachable without a special case.

`doc` writes a single top-level array of those same records, pretty-printed
across lines, grown until the file is about --bytes long. It is the shape the
lazy structural index has to stay flat on.
"""
import os
import sys

MB = 1024 * 1024

KINDS = ["user", "assistant", "tool_use", "tool_result", "system"]
TOOLS = ["Read", "Bash", "Edit", "Grep", "Write", "Glob"]
WORDS = ["index", "offset", "window", "record", "member", "container", "scan",
         "budget", "viewport", "fold", "row", "byte", "cursor", "path"]


def esc(s):
    """The subset of RFC 8259 string escaping these fixtures need."""
    return s.replace("\\", "\\\\").replace('"', '\\"')


def payload(i, depth):
    """A nested object `depth` levels deep, innermost holding the scalars."""
    inner = ('{"id":%d,"ok":%s,"ratio":%s,"note":"%s","tags":["%s","%s"]}'
             % (i, "true" if i % 3 else "false",
                "%d.%03d" % (i % 97, i % 1000),
                esc(WORDS[i % len(WORDS)] + " " + WORDS[(i * 7) % len(WORDS)]),
                WORDS[(i + 1) % len(WORDS)], WORDS[(i + 2) % len(WORDS)]))
    for d in range(depth):
        inner = '{"level%d":%s}' % (d, inner)
    return inner


def record(i, depth, pad):
    """One record as a single line of JSON, without the newline."""
    kind = KINDS[i % len(KINDS)]
    msg = "%s %s step %d" % (WORDS[i % len(WORDS)], WORDS[(i * 3) % len(WORDS)], i)
    if pad:
        # Pad deterministically to about `pad` bytes of message.
        filler = "".join(WORDS[(i + k) % len(WORDS)] + " " for k in range(64))
        while len(msg) < pad:
            msg += filler
        msg = msg[:pad]
    return ('{"seq":%d,"ts":"2026-0%d-%02dT%02d:%02d:%02dZ","type":"%s",'
            '"tool":"%s","message":"%s","payload":%s}'
            % (i, 1 + i % 9, 1 + i % 28, i % 24, i % 60, (i * 7) % 60,
               kind, TOOLS[i % len(TOOLS)], esc(msg), payload(i, depth)))


def gen_records(out, n, depth, pad):
    buf = []
    for i in range(n):
        buf.append(record(i, depth, pad))
        if len(buf) >= 2048:
            out.write(("\n".join(buf) + "\n").encode())
            buf = []
    if buf:
        out.write(("\n".join(buf) + "\n").encode())


def gen_doc(out, want_bytes, depth):
    """A top-level array of records, pretty-printed, to about `want_bytes`."""
    out.write(b"[\n")
    written = 2
    i = 0
    buf = []
    while written < want_bytes:
        line = ("  " if i == 0 else ",\n  ") + record(i, depth, 0)
        buf.append(line)
        written += len(line)
        i += 1
        if len(buf) >= 2048:
            blob = "".join(buf).encode()
            out.write(blob)
            buf = []
    if buf:
        out.write("".join(buf).encode())
    out.write(b"\n]\n")


# ------------------------------------------------------------ hostile cases
#
# Each case is (suffix, writer). The suffix matters: tread picks the format
# from the extension, so a `.jsonl` case and a `.json` case of the same bytes
# are two different tests.

def case_deep_10k(out):
    """Nesting 10,000 deep: the stack-overflow case, well-formed."""
    n = 10000
    out.write(b"[" * n + b"1" + b"]" * n + b"\n")


def case_deep_100k(out):
    """Ten times past the depth limit, still well-formed."""
    n = 100000
    out.write(b"[" * n + b"1" + b"]" * n + b"\n")


def case_deep_objects(out):
    """10,000 deep, but objects -- the key path is as long as the nesting."""
    n = 10000
    out.write(b'{"k":' * n + b"1" + b"}" * n + b"\n")


def case_deep_unclosed(out):
    """10,000 opening brackets and no closing one: deep *and* truncated."""
    out.write(b"[" * 10000 + b"\n")


def case_big_string(out):
    """One 50MB string value, past every per-member cap."""
    out.write(b'{"small":1,"blob":"')
    chunk = b"x" * MB
    for _ in range(50):
        out.write(chunk)
    out.write(b'","tail":2}\n')


def case_keys_100k(out):
    """One object with 100,000 keys."""
    out.write(b"{")
    for i in range(100000):
        out.write(b"," if i else b"")
        out.write(('"key_%06d":%d' % (i, i)).encode())
    out.write(b"}\n")


def case_array_1m(out):
    """One array with 1,000,000 elements."""
    out.write(b"[")
    buf = []
    for i in range(1000000):
        buf.append(("," if i else "") + str(i))
        if len(buf) >= 8192:
            out.write("".join(buf).encode())
            buf = []
    out.write("".join(buf).encode())
    out.write(b"]\n")


def case_dup_keys_100k(out):
    """100,000 members of one object, all spelled the same key."""
    out.write(b"{")
    for i in range(100000):
        out.write(b"," if i else b"")
        out.write(('"same":%d' % i).encode())
    out.write(b"}\n")


def case_truncated_string(out):
    out.write(b'{"a":1,"b":"never closed and the file just en')


def case_truncated_deep(out):
    out.write(b'{"a":[1,2,{"b":[3,4,')


def case_trailing_comma(out):
    out.write(b'{"a":[1,2,3,],"b":{"c":1,},}\n')


def case_unquoted_key(out):
    out.write(b'{a:1, b:2, c:[3]}\n')


def case_single_quotes(out):
    out.write(b"{'a':'b','c':['d']}\n")


def case_nan_infinity(out):
    out.write(b'{"a":NaN,"b":Infinity,"c":-Infinity,"d":1e999,"e":-0}\n')


def case_dup_keys(out):
    out.write(b'{"a":1,"a":2,"a":3,"b":{"x":1,"x":2}}\n')


def case_lone_surrogate(out):
    out.write(b'{"hi":"\\uD800","lo":"\\uDC00","pair":"\\uD83D\\uDE00",'
              b'"half":"a\\uD800b"}\n')


def case_invalid_utf8(out):
    out.write(b'{"a":"\xff\xfe\xfd","b":"caf\xe9","c":"ok"}\n')


def case_nuls(out):
    out.write(b'{"a":"x\x00y","b":"\x00\x00\x00","c":1}\n')


def case_bom(out):
    out.write(b"\xef\xbb\xbf" + b'{"a":1,"b":[2,3]}\n')


def case_bom_jsonl(out):
    out.write(b"\xef\xbb\xbf" + b'{"a":1}\n{"a":2}\n')


def case_utf16le(out):
    out.write(b"\xff\xfe" + '{"a":1}\n'.encode("utf-16-le"))


def case_empty(out):
    pass


def case_only_newlines(out):
    out.write(b"\n" * 500)


def case_only_whitespace(out):
    out.write(b" \t\r\n" * 200)


def case_bare_scalar(out):
    out.write(b"42\n")


def case_bare_string(out):
    out.write(b'"just a string"\n')


def case_two_values(out):
    """Two top-level values in a .json: only the first is a document."""
    out.write(b'{"a":1}{"b":2}\n')


def case_jsonl_bad_line(out):
    out.write(b'{"a":1}\n')
    out.write(b'{"a":2,,}\n')
    out.write(b'not json at all\n')
    out.write(b'{"a":3}\n')


def case_jsonl_empty_lines(out):
    out.write(b'{"a":1}\n\n\n{"a":2}\n\n')


def case_jsonl_no_trailing_newline(out):
    out.write(b'{"a":1}\n{"a":2}\n{"a":3}')


def case_jsonl_one_long_line(out):
    """A single 41KB line, the shape a real trajectory reaches."""
    out.write(record(0, 2, 41 * 1024).encode() + b"\n")


def case_jsonl_crlf(out):
    out.write(b'{"a":1}\r\n{"a":2}\r\n{"a":3}\r\n')


def case_jsonl_bare_cr(out):
    out.write(b'{"a":1}\r{"a":2}\r{"a":3}\r')


def case_jsonl_deep(out):
    """Every line 10,000 deep."""
    line = b"[" * 10000 + b"1" + b"]" * 10000
    for _ in range(5):
        out.write(line + b"\n")


def case_jsonl_nuls(out):
    out.write(b'{"a":"x\x00y"}\n{"a":"\x00"}\n{"a":1}\n')


def case_jsonl_big_line(out):
    """One 8MB line among small ones: past the per-record cap."""
    out.write(b'{"a":1}\n')
    out.write(b'{"blob":"')
    for _ in range(8):
        out.write(b"y" * MB)
    out.write(b'"}\n')
    out.write(b'{"a":2}\n')


def case_not_a_trajectory(out):
    """Valid records that no lens recognises: --lens must fall back, not hide."""
    for i in range(50):
        out.write(('{"id":%d,"colour":"blue","dims":[%d,%d]}\n'
                   % (i, i, i * 2)).encode())


def case_lens_shaped_but_wrong(out):
    """Trajectory-ish keys carrying the wrong types."""
    out.write(b'{"type":42,"message":[1,2,3]}\n')
    out.write(b'{"type":"assistant","message":{"content":"a bare string"}}\n')
    out.write(b'{"type":"assistant","message":{"content":[{"type":"tool_use"}]}}\n')
    out.write(b'{"message":null}\n')
    out.write(b'{}\n')


CASES = {
    "deep_10k": (".json", case_deep_10k),
    "deep_100k": (".json", case_deep_100k),
    "deep_objects": (".json", case_deep_objects),
    "deep_unclosed": (".json", case_deep_unclosed),
    "big_string_50mb": (".json", case_big_string),
    "keys_100k": (".json", case_keys_100k),
    "array_1m": (".json", case_array_1m),
    "dup_keys_100k": (".json", case_dup_keys_100k),
    "truncated_string": (".json", case_truncated_string),
    "truncated_deep": (".json", case_truncated_deep),
    "trailing_comma": (".json", case_trailing_comma),
    "unquoted_key": (".json", case_unquoted_key),
    "single_quotes": (".json", case_single_quotes),
    "nan_infinity": (".json", case_nan_infinity),
    "dup_keys": (".json", case_dup_keys),
    "lone_surrogate": (".json", case_lone_surrogate),
    "invalid_utf8": (".json", case_invalid_utf8),
    "nuls": (".json", case_nuls),
    "bom": (".json", case_bom),
    "empty": (".json", case_empty),
    "only_newlines": (".json", case_only_newlines),
    "only_whitespace": (".json", case_only_whitespace),
    "bare_scalar": (".json", case_bare_scalar),
    "bare_string": (".json", case_bare_string),
    "two_values": (".json", case_two_values),
    "jsonl_bom": (".jsonl", case_bom_jsonl),
    "jsonl_bad_line": (".jsonl", case_jsonl_bad_line),
    "jsonl_empty_lines": (".jsonl", case_jsonl_empty_lines),
    "jsonl_no_trailing_newline": (".jsonl", case_jsonl_no_trailing_newline),
    "jsonl_one_long_line": (".jsonl", case_jsonl_one_long_line),
    "jsonl_crlf": (".jsonl", case_jsonl_crlf),
    "jsonl_bare_cr": (".jsonl", case_jsonl_bare_cr),
    "jsonl_deep": (".jsonl", case_jsonl_deep),
    "jsonl_nuls": (".jsonl", case_jsonl_nuls),
    "jsonl_big_line": (".jsonl", case_jsonl_big_line),
    "jsonl_empty": (".jsonl", case_empty),
    "jsonl_only_newlines": (".jsonl", case_only_newlines),
    "not_a_trajectory": (".jsonl", case_not_a_trajectory),
    "lens_shaped_but_wrong": (".jsonl", case_lens_shaped_but_wrong),
}

# Cases tread must *refuse* rather than render, for the same reason the CSV
# side keeps them apart: the render soaks fail on a non-zero exit.
REFUSALS = {
    "utf16le": (".json", case_utf16le),
}

# The huge ones. Kept out of the default `cases` set so the soak can generate
# a directory it can walk in seconds; soak_json.sh asks for them by name.
HUGE = {"deep_100k", "big_string_50mb", "keys_100k", "array_1m",
        "dup_keys_100k", "jsonl_big_line"}


# ------------------------------------------------------------------- driver

def write_case(which, name, d):
    suffix, make = which[name]
    path = os.path.join(d, name + suffix)
    with open(path, "wb", buffering=1 << 20) as f:
        make(f)
    print("%s %d" % (path, os.path.getsize(path)))


def main(argv):
    if len(argv) < 2:
        sys.stderr.write(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "list":
        for name in sorted(CASES):
            print("%s%s%s" % (name, CASES[name][0],
                              " (huge)" if name in HUGE else ""))
        for name in sorted(REFUSALS):
            print("%s%s (refused)" % (name, REFUSALS[name][0]))
        return 0
    if cmd in ("cases", "huge", "refusals"):
        d = argv[2]
        os.makedirs(d, exist_ok=True)
        if cmd == "refusals":
            which, names = REFUSALS, sorted(REFUSALS)
        elif cmd == "huge":
            which, names = CASES, sorted(HUGE)
        else:
            which, names = CASES, sorted(set(CASES) - HUGE)
        for name in names:
            write_case(which, name, d)
        return 0
    if cmd == "case":
        name, path = argv[2], argv[3]
        entry = CASES.get(name) or REFUSALS.get(name)
        if entry is None:
            sys.stderr.write("unknown case: %s\n" % name)
            return 2
        with open(path, "wb", buffering=1 << 20) as f:
            entry[1](f)
        print("%s %d" % (path, os.path.getsize(path)))
        return 0
    if cmd in ("records", "doc"):
        n, depth, pad, want, path = 1000, 1, 0, MB, None
        i = 2
        while i < len(argv):
            a = argv[i]
            if a == "--records":
                n = int(argv[i + 1]); i += 2
            elif a == "--depth":
                depth = int(argv[i + 1]); i += 2
            elif a == "--pad":
                pad = int(argv[i + 1]); i += 2
            elif a == "--bytes":
                want = int(argv[i + 1]); i += 2
            else:
                path = a; i += 1
        if path is None:
            sys.stderr.write("%s: no output path\n" % cmd)
            return 2
        with open(path, "wb", buffering=1 << 20) as f:
            if cmd == "records":
                gen_records(f, n, depth, pad)
            else:
                gen_doc(f, want, depth)
        print("%s %d" % (path, os.path.getsize(path)))
        return 0
    sys.stderr.write("unknown command: %s\n" % cmd)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
