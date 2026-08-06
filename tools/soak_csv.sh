#!/bin/sh
# CSV scale + hostile-input soak for tread.
#
# Generates every file it needs (tools/csvgen.py, deterministic) into a work
# directory, measures the reader on them (tools/csvbench.py, real pty), and
# then tries to break it. Nothing here is checked in: the multi-GB files are
# generated on demand and deleted with the work directory.
#
# Usage: tools/soak_csv.sh <tread-binary> [work-dir]
#
#   ROWS="10000 1000000"   row counts to measure    (default 10k / 1M / 10M)
#   KEEP=1                 keep the work directory
#   SKIP_HOSTILE=1         only measure, do not break
#   SKIP_SCALE=1           only break, do not measure
#
# 10M rows of the default shape is about 1GB, so the default run needs ~1.2GB
# of disk and a few minutes to generate.
set -u

BIN=${1:?usage: soak_csv.sh <tread-binary> [work-dir]}
case $BIN in /*) ;; *) BIN=$(pwd)/$BIN ;; esac
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORK=${2:-$(mktemp -d)}
ROWS=${ROWS:-"10000 1000000 10000000"}
mkdir -p "$WORK"
[ "${KEEP:-0}" = 1 ] || trap 'rm -rf "$WORK"' EXIT

fail=0
note() { echo "$@"; }
bad() { echo "FAIL $*"; fail=$((fail + 1)); }

note "soak_csv: binary=$BIN work=$WORK"

# ---------------------------------------------------------------- scale ----
if [ "${SKIP_SCALE:-0}" != 1 ]; then
    files=""
    for n in $ROWS; do
        f="$WORK/rows_$n.csv"
        [ -f "$f" ] || python3 "$HERE/csvgen.py" rows --rows "$n" --cols 8 \
            --quote-every 997 "$f" >/dev/null || bad "generate $f"
        files="$files $f"
    done
    note "-- open / scroll / G / q, by size"
    # shellcheck disable=SC2086
    python3 "$HERE/csvbench.py" "$BIN" $files || bad "csvbench"
fi

# -------------------------------------------------------------- hostile ----
if [ "${SKIP_HOSTILE:-0}" != 1 ]; then
    CASES="$WORK/cases"
    python3 "$HERE/csvgen.py" cases "$CASES" >/dev/null || bad "generate cases"

    note "-- hostile input, non-interactive render"
    sh "$HERE/soak.sh" "$BIN" "$CASES" || bad "soak.sh over hostile cases"

    note "-- hostile input, interactive pty"
    python3 "$HERE/soak_pty.py" "$BIN" "$CASES" || bad "soak_pty.py over hostile cases"

    note "-- input tread must refuse rather than render"
    REF="$WORK/refused"
    python3 "$HERE/csvgen.py" refusals "$REF" >/dev/null || bad "generate refusals"
    for f in "$REF"/*.csv; do
        out=$(timeout 20 "$BIN" --no-alt --width 80 "$f" 2>"$WORK/err")
        rc=$?
        [ $rc -eq 1 ] || bad "$f: exit $rc, expected 1"
        [ -z "$out" ] || bad "$f: refused but still painted something"
        grep -q "UTF-16" "$WORK/err" || bad "$f: no encoding named: $(head -1 "$WORK/err")"
        grep -q "iconv" "$WORK/err" || bad "$f: no conversion suggested"
        # ... and the same on stdin, which has no file name to go on.
        timeout 20 "$BIN" --no-alt --width 80 - <"$f" >/dev/null 2>&1
        [ $? -eq 1 ] || bad "$f on stdin: not refused"
    done
    head -1 "$WORK/err"

    note "-- special files"
    timeout 20 "$BIN" --no-alt --width 80 /dev/null >/dev/null 2>"$WORK/err" \
        || bad "/dev/null: exit $? $(head -1 "$WORK/err")"
    timeout 20 "$BIN" --no-alt --width 80 --format csv /dev/null \
        >/dev/null 2>"$WORK/err" || bad "/dev/null --format csv: exit $?"

    fifo="$WORK/pipe.csv"
    rm -f "$fifo"; mkfifo "$fifo"
    (printf 'id,name\n1,alice\n2,bob\n3,carol\n' >"$fifo") &
    timeout 20 "$BIN" --no-alt --width 80 - <"$fifo" >"$WORK/fifo.out" 2>&1 \
        || bad "named pipe on stdin: exit $?"
    grep -q alice "$WORK/fifo.out" || bad "named pipe on stdin: no data rendered"
    wait

    # A named pipe passed by *path* cannot be seeked; it must not hang.
    rm -f "$fifo"; mkfifo "$fifo"
    (printf 'id,name\n1,alice\n2,bob\n3,carol\n' >"$fifo") &
    timeout 20 "$BIN" --no-alt --width 80 "$fifo" >"$WORK/fifo2.out" 2>&1
    rc=$?
    [ $rc -eq 124 ] && bad "named pipe by path: hung"
    [ $rc -eq 0 ] || bad "named pipe by path: exit $rc"
    grep -q alice "$WORK/fifo2.out" || bad "named pipe by path: no data rendered"
    wait

    # An endless device must stop somewhere instead of eating the machine.
    timeout 120 "$BIN" --no-alt --width 80 /dev/zero >/dev/null 2>"$WORK/err"
    rc=$?
    [ $rc -eq 1 ] || bad "/dev/zero: exit $rc, expected 1"
    grep -q "regular file" "$WORK/err" || bad "/dev/zero: unhelpful error: $(head -1 "$WORK/err")"

    timeout 20 "$BIN" --no-alt --width 80 "$WORK" >/dev/null 2>"$WORK/err"
    [ $? -eq 1 ] || bad "a directory is not an error"

    note "-- file appended to while open"
    grow="$WORK/growing.csv"
    printf 'id,name\n1,alice\n' >"$grow"
    ( for i in $(seq 1 200); do
          printf '%d,name%d\n' "$i" "$i" >>"$grow"; sleep 0.01
      done ) &
    writer=$!
    python3 "$HERE/csvbench.py" "$BIN" "$grow" >"$WORK/grow.out" 2>&1 \
        || bad "growing file: csvbench"
    grep -q "panic *True" "$WORK/grow.out" && bad "growing file: panic"
    grep -E "open_ms|quit_ms|panic" "$WORK/grow.out"
    kill $writer 2>/dev/null
    wait 2>/dev/null

    note "-- truncated while open"
    shrink="$WORK/shrinking.csv"
    python3 "$HERE/csvgen.py" rows --rows 20000 "$shrink" >/dev/null
    ( sleep 0.5; : >"$shrink" ) &
    timeout 20 "$BIN" --no-alt --width 80 "$shrink" >/dev/null 2>"$WORK/err" \
        || bad "truncated while open: exit $?"
    wait
fi

echo "soak_csv: $fail failures"
[ "$fail" -eq 0 ]
