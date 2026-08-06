#!/bin/sh
# Corpus + adversarial soak for tread.
#
# Renders every document at several widths, both plain (piped) and styled
# (forced through the pty path), and fails on: non-zero exit, timeout, panic
# text on stderr, or an unbalanced / stray escape sequence in the output.
#
# Usage: tools/soak.sh <tread-binary> <corpus-dir> [extra-dir ...]
set -u

BIN=${1:?usage: soak.sh <binary> <dir> [dir...]}
shift
[ $# -gt 0 ] || { echo "soak: no directories given" >&2; exit 2; }

WIDTHS="40 80 120 200"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fail=0
count=0

# A rendered frame may only contain SGR (CSI ... m), erase-in-line (CSI K),
# cursor position (CSI ... H), the screen/cursor toggles, and OSC 8 / OSC 52.
# Anything else -- a bare ESC, a truncated CSI, a mouse-tracking mode, or a raw
# control byte that leaked out of the document -- is a bug.
check_escapes() {
    LC_ALL=C awk -v file="$1" '
      { line = $0
        if (line ~ /[\001-\010\013\014\016-\032\034-\037\177]/) {
            print file ": raw control byte in output"; bad = 1
        }
        while ((i = index(line, "\033")) > 0) {
            rest = substr(line, i + 1)
            if (rest !~ /^(\[[0-9;?]*[a-zA-Z]|\][0-9]|[()][B0])/) {
                print file ": stray ESC: " substr(rest, 1, 20); bad = 1
            }
            if (rest ~ /^\[\?(1000|1002|1003|1006|1015)[hl]/) {
                print file ": MOUSE CAPTURE emitted"; bad = 1
            }
            line = rest
        }
      }
      END { exit bad ? 1 : 0 }' "$2"
}

run_one() {
    doc=$1; width=$2; shift 2
    out="$TMP/out"; err="$TMP/err"
    timeout 20 "$BIN" --no-alt --width "$width" "$@" "$doc" >"$out" 2>"$err"
    rc=$?
    count=$((count + 1))
    if [ $rc -eq 124 ]; then
        echo "FAIL(timeout ${width}) $doc"; fail=$((fail + 1)); return
    fi
    if [ $rc -ne 0 ]; then
        echo "FAIL(exit $rc @${width}) $doc: $(head -1 "$err")"; fail=$((fail + 1)); return
    fi
    if grep -qE "panicked at|RUST_BACKTRACE|internal error" "$err"; then
        echo "FAIL(panic @${width}) $doc: $(head -2 "$err" | tr '\n' ' ')"
        fail=$((fail + 1)); return
    fi
    if ! check_escapes "$doc@$width" "$out"; then
        fail=$((fail + 1))
    fi
}

for dir in "$@"; do
    # shellcheck disable=SC2044
    for doc in $(find "$dir" -name '*.md' | sort); do
        for w in $WIDTHS; do
            run_one "$doc" "$w"
        done
        run_one "$doc" 80 --toc
        timeout 20 "$BIN" --no-alt --width 80 - <"$doc" >/dev/null 2>&1 || {
            echo "FAIL(stdin) $doc"; fail=$((fail + 1)); }
        count=$((count + 1))
    done
done

echo "soak: $count renders, $fail failures"
[ "$fail" -eq 0 ]
