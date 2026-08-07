#!/bin/sh
# JSON / JSONL scale + hostile-input soak for tread.
#
# The sibling of tools/soak_csv.sh. Generates every file it needs
# (tools/jsongen.py, deterministic) into a work directory, measures the reader
# on them (tools/jsonbench.py, real pty), and then tries to break it. Nothing
# here is checked in: the huge files are generated on demand and deleted with
# the work directory.
#
# Usage: tools/soak_json.sh <tread-binary> [work-dir]
#
#   RECORDS="10000 1000000"  record counts to measure   (default 10k/100k/1M)
#   DOCS="1048576 16777216"  .json sizes in bytes       (default 1MB/16MB/256MB)
#   TRAJECTORY=<path>        also measure a real .jsonl, with and without --lens
#   KEEP=1                   keep the work directory
#   SKIP_HOSTILE=1           only measure, do not break
#   SKIP_SCALE=1             only break, do not measure
#   SKIP_HUGE=1              skip the 50MB-string / 1M-element cases
#
# The default run needs about 800MB of disk and a few minutes.
set -u

BIN=${1:?usage: soak_json.sh <tread-binary> [work-dir]}
case $BIN in /*) ;; *) BIN=$(pwd)/$BIN ;; esac
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORK=${2:-$(mktemp -d)}
RECORDS=${RECORDS:-"10000 100000 1000000"}
DOCS=${DOCS:-"1048576 16777216 268435456"}
mkdir -p "$WORK"
[ "${KEEP:-0}" = 1 ] || trap 'rm -rf "$WORK"' EXIT

fail=0
note() { echo "$@"; }
bad() { echo "FAIL $*"; fail=$((fail + 1)); }

note "soak_json: binary=$BIN work=$WORK"

# ---------------------------------------------------------------- scale ----
if [ "${SKIP_SCALE:-0}" != 1 ]; then
    files=""
    for n in $RECORDS; do
        f="$WORK/rec_$n.jsonl"
        [ -f "$f" ] || python3 "$HERE/jsongen.py" records --records "$n" --depth 2 \
            "$f" >/dev/null || bad "generate $f"
        files="$files $f"
    done
    for b in $DOCS; do
        f="$WORK/doc_$b.json"
        [ -f "$f" ] || python3 "$HERE/jsongen.py" doc --bytes "$b" "$f" >/dev/null \
            || bad "generate $f"
        files="$files $f"
    done
    note "-- open / scroll / zR / G / q, by size"
    # shellcheck disable=SC2086
    python3 "$HERE/jsonbench.py" "$BIN" $files || bad "jsonbench"

    # --to-jsonl streams: the document is never held (SPEC.md §--to-jsonl), so
    # its peak RSS must not track the size of the document it is writing out.
    note "-- --to-jsonl, streaming"
    for b in $DOCS; do
        f="$WORK/doc_$b.json"
        /usr/bin/time -f "   %C: %es %MkB" timeout 300 "$BIN" --to-jsonl "$f" \
            >"$WORK/out.jsonl" 2>"$WORK/time" || bad "--to-jsonl $f: exit $?"
        sed "s|$WORK/||" "$WORK/time"
        [ -s "$WORK/out.jsonl" ] || bad "--to-jsonl $f wrote nothing"
    done

    if [ -n "${TRAJECTORY:-}" ]; then
        note "-- a real trajectory, generic and through the lens"
        python3 "$HERE/jsonbench.py" "$BIN" "$TRAJECTORY" || bad "trajectory"
        python3 "$HERE/jsonbench.py" --lens agent "$BIN" "$TRAJECTORY" \
            || bad "trajectory --lens"
    fi
fi

# -------------------------------------------------------------- hostile ----
if [ "${SKIP_HOSTILE:-0}" != 1 ]; then
    CASES="$WORK/cases"
    python3 "$HERE/jsongen.py" cases "$CASES" >/dev/null || bad "generate cases"
    if [ "${SKIP_HUGE:-0}" != 1 ]; then
        python3 "$HERE/jsongen.py" huge "$WORK/huge" >/dev/null || bad "generate huge"
    fi

    note "-- hostile input, non-interactive render"
    sh "$HERE/soak.sh" "$BIN" "$CASES" || bad "soak.sh over hostile cases"

    note "-- hostile input, interactive pty"
    python3 "$HERE/soak_pty.py" "$BIN" "$CASES" || bad "soak_pty.py over hostile cases"

    note "-- every hostile case through the lens, which must fall back not hide"
    for f in "$CASES"/*.jsonl; do
        timeout 60 "$BIN" --no-alt --width 100 --lens agent "$f" >/dev/null 2>&1 \
            || bad "--lens agent $f: exit $?"
    done

    if [ "${SKIP_HUGE:-0}" != 1 ]; then
        note "-- the huge cases: 50MB string, 100k keys, 1M elements, 100k deep"
        sh "$HERE/soak.sh" "$BIN" "$WORK/huge" || bad "soak.sh over huge cases"
        python3 "$HERE/soak_pty.py" "$BIN" "$WORK/huge" || bad "soak_pty.py over huge"

        # The per-member cap: a member too big to show says how big it is and
        # what the limit is, by name, rather than being loaded (SPEC.md §JSON).
        out=$(timeout 120 "$BIN" --no-alt --plain --width 120 \
            "$WORK/huge/big_string_50mb.json")
        echo "$out" | grep -q "over the" \
            || bad "50MB member: no cap message: $out"
        echo "$out" | grep -q '"tail": 2' \
            || bad "50MB member: the rows after it were dropped"

        # Nesting past the limit is refused by name rather than opened.
        timeout 300 "$BIN" --no-alt --plain --width 160 "$WORK/huge/deep_100k.json" \
            2>/dev/null | grep -q "nested deeper than" \
            || bad "100k deep: no refusal shown"
    fi

    note "-- input tread must refuse rather than render"
    REF="$WORK/refused"
    python3 "$HERE/jsongen.py" refusals "$REF" >/dev/null || bad "generate refusals"
    for f in "$REF"/*.json; do
        timeout 20 "$BIN" --no-alt --width 80 "$f" >"$WORK/out" 2>"$WORK/err"
        rc=$?
        [ $rc -eq 1 ] || bad "$f: exit $rc, expected 1"
        [ -s "$WORK/out" ] && bad "$f: refused but still painted something"
        grep -q "UTF-16" "$WORK/err" || bad "$f: no encoding named"
    done

    note "-- --lens on files that are not records"
    for f in "$CASES/deep_10k.json" "$HERE/../README.md"; do
        timeout 20 "$BIN" --no-alt --width 80 --lens agent "$f" >"$WORK/out" 2>"$WORK/err"
        [ $? -eq 2 ] || bad "--lens $f: expected a usage error"
        grep -q -- "--format jsonl" "$WORK/err" || bad "--lens $f: no way forward"
    done
    timeout 20 "$BIN" --no-alt --width 80 --lens nosuch "$CASES/not_a_trajectory.jsonl" \
        >/dev/null 2>"$WORK/err"
    [ $? -eq 2 ] || bad "--lens nosuch: expected a usage error"

    note "-- --to-jsonl refusals"
    for f in "$CASES/bare_scalar.json" "$CASES/empty.json" "$CASES/dup_keys.json"; do
        timeout 60 "$BIN" --to-jsonl "$f" >/dev/null 2>"$WORK/err"
        [ $? -eq 1 ] || bad "--to-jsonl $f: expected exit 1"
        grep -q "to-jsonl" "$WORK/err" || bad "--to-jsonl $f: unhelpful error"
    done

    note "-- special files"
    for extra in "" "--format json" "--format jsonl"; do
        # shellcheck disable=SC2086
        timeout 20 "$BIN" --no-alt --width 80 $extra /dev/null >/dev/null 2>"$WORK/err" \
            || bad "/dev/null $extra: exit $?"
    done
    timeout 20 "$BIN" --no-alt --width 80 --format json "$WORK" >/dev/null 2>&1
    [ $? -eq 1 ] || bad "a directory is not an error"

    fifo="$WORK/pipe.jsonl"
    rm -f "$fifo"; mkfifo "$fifo"
    (printf '{"a":1}\n{"a":2}\n' >"$fifo") &
    timeout 20 "$BIN" --no-alt --width 80 - <"$fifo" >"$WORK/fifo.out" 2>&1 \
        || bad "named pipe on stdin: exit $?"
    grep -q '"a"' "$WORK/fifo.out" || bad "named pipe on stdin: no data rendered"
    wait

    # A JSON document by path down a fifo cannot be seeked; it must not hang.
    rm -f "$fifo"; mkfifo "$fifo"
    (printf '{"a":1,"b":[2,3]}\n' >"$fifo") &
    timeout 20 "$BIN" --no-alt --width 80 --format json "$fifo" >"$WORK/fifo2.out" 2>&1
    rc=$?
    [ $rc -eq 124 ] && bad "named pipe by path: hung"
    wait

    # An endless device must stop somewhere instead of eating the machine.
    timeout 120 "$BIN" --no-alt --width 80 --format json /dev/zero >/dev/null 2>"$WORK/err"
    rc=$?
    [ $rc -eq 1 ] || bad "/dev/zero: exit $rc, expected 1"

    note "-- truncated while open"
    shrink="$WORK/shrinking.json"
    python3 "$HERE/jsongen.py" doc --bytes 8388608 "$shrink" >/dev/null
    ( sleep 0.5; : >"$shrink" ) &
    timeout 60 "$BIN" --no-alt --width 80 "$shrink" >/dev/null 2>"$WORK/err" \
        || bad "truncated while open: exit $?"
    grep -q "panicked at" "$WORK/err" && bad "truncated while open: panic"
    wait

    note "-- appended to while open"
    grow="$WORK/growing.jsonl"
    printf '{"a":0}\n' >"$grow"
    ( for i in $(seq 1 200); do printf '{"a":%d}\n' "$i" >>"$grow"; sleep 0.01; done ) &
    writer=$!
    python3 "$HERE/jsonbench.py" "$BIN" "$grow" >"$WORK/grow.out" 2>&1 \
        || bad "growing file: jsonbench"
    grep -q "panic *True" "$WORK/grow.out" && bad "growing file: panic"
    grep -E "open_ms|quit_ms|panic" "$WORK/grow.out"
    kill $writer 2>/dev/null
    wait 2>/dev/null
fi

echo "soak_json: $fail failures"
[ "$fail" -eq 0 ]
