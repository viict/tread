# tread — `tread`

`less`, but it understands markdown — and CSV.

A terminal reader with collapsible headings, banner H1s, real box-drawn tables,
colored links, and navigation across a corpus of linked documents. It also
reads CSV: multi-GB files open instantly, with a pinned header and column-wise
scrolling. One static binary, no runtime, no configuration, **no dependencies
at all** — not even `libc`. Every format is compiled in; nothing is ever loaded
at runtime.

```
$ tread README.md
```

## What it does

- **Collapsible sections.** Every heading carries a `▾`/`▸` marker in the
  gutter. `za` folds the section under the cursor, `zM` folds everything, `zR`
  opens it back up.
- **Real tables.** Column widths are computed from content, `:---`/`:---:`/
  `---:` alignment is honored, and a table wider than the terminal scrolls
  horizontally with `h`/`l` instead of being mangled.
- **A document tree.** Relative links resolve against the current file.
  `Enter` follows one, `Backspace` goes back, `i` opens the corpus index
  grouped by the section each link appeared under.
- **Search, outline, yank.** `/` searches, `o` shows the outline, `v` starts a
  visual line selection, `y` puts it on the system clipboard over OSC 52 (with
  a file fallback so a terminal that refuses the escape never loses the copy).
- **Correct widths.** Wrapping uses display columns, so CJK is width 2,
  combining marks are width 0, and emoji do not desynchronise the layout.
- **The mouse is never captured.** No `?1000h`, no `?1002h`, no `?1006h`. Your
  terminal's own click-drag selection keeps working, always. This is a product
  requirement, not an oversight.

## Install

A Rust toolchain (1.75+) is all you need — no C compiler, no linker, no system
libraries.

```sh
cargo build --release
./target/release/tread --help
```

For the shipping artifact, one file that runs on any x86-64 Linux from a
scratch container upward:

```sh
rustup target add x86_64-unknown-linux-musl
cargo musl                      # alias for --release --target …-musl
```

```
$ ldd target/x86_64-unknown-linux-musl/release/tread
        statically linked                             # 649 KiB
```

| Platform | Target | Linkage |
| --- | --- | --- |
| Linux | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | fully static |
| Linux | `x86_64-unknown-linux-gnu` | dynamic (glibc) |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | dynamic, `libSystem` only |
| Windows 10 1703+ | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-pc-windows-gnu` | dynamic, `kernel32` only |

Build for a platform on that platform. The musl binary is a Linux ELF and will
not run on macOS; macOS cannot be statically linked at all, because Apple does
not ship a static libc. Any unix that is neither Linux nor Darwin is refused at
compile time with a message naming the work, rather than served a `termios`
layout that is probably wrong.

Every target in that table is built *and* tested on its own architecture by CI
on each release, so the macOS and Windows backends run for real rather than
merely type-checking. What no CI run covers is interactive behaviour — that a
console host restores its mode on exit, that drag-select still works while the
pager is up — because none of it happens without a terminal attached.
[`docs/windows.md`](docs/windows.md) records what the console backend does and what is
still only inferred.

## Usage

```
tread [OPTIONS] [FILE]

  --index <PATH>   Treat PATH as the corpus index. PATH may be a markdown file
                   or a directory containing README.md. Relative links in the
                   corpus resolve against it.
  --no-alt         Render into the scrollback instead of the alternate screen,
                   so the output stays visible after quitting.
  --plain          Disable color. Implied by NO_COLOR or a non-terminal stdout.
  --width <N>      Force the wrap width instead of detecting the terminal size.
  --format <FMT>   Force the format: `md` or `csv`. By default the extension
                   decides, and unnamed input (a pipe) is sniffed.
  --delim <D>      CSV field delimiter: one character, or `tab`, `comma`,
                   `semicolon`, `pipe`. Sniffed among `,` TAB `;` `|` otherwise.
  --toc            Print the heading outline (CSV: the column names) and exit.
  -h, --help       Show help.
  -V, --version    Show the version.
```

With no `FILE`, `tread` reads piped stdin, or opens the corpus index when stdin
is a terminal. `-` forces reading stdin. Piping works because keys are read
from `/dev/tty` when stdin is busy:

```sh
cat notes.md | tread
tread --toc notes.md
tread --plain --width 100 notes.md > notes.txt
```

Exit codes: `0` ok, `1` runtime error, `2` usage error.

## Keys

| Key | Action |
| --- | --- |
| `j / ↓` | line down |
| `k / ↑` | line up |
| `d` | half page down |
| `u` | half page up |
| `space / f` | page down |
| `b` | page up |
| `g` | top of document |
| `G` | bottom of document |
| `h / ←` | scroll left (code, wide tables; one CSV column) |
| `l / →` | scroll right (code, wide tables; one CSV column) |
| `w` | widen the CSV column under the cursor to fit the screen |
| `za` | toggle the section at the cursor |
| `Enter` | follow the focused link, else toggle the section |
| `zo` | open the section at the cursor |
| `zc` | close the section at the cursor |
| `zM` | collapse every section |
| `zR` | expand every section |
| `Tab` | next heading |
| `S-Tab` | previous heading |
| `o` | outline overlay |
| `/` | search forward |
| `?` | search backward |
| `n` | next link (next search match while searching) |
| `N` | previous link (previous match while searching) |
| `Backspace / -` | back in document history |
| `+` | forward in document history |
| `i` | corpus index (j/k move, Enter open, / filter, Esc close) |
| `]` | next document in index order |
| `[` | previous document in index order |
| `v` | visual line select (j/k/d/u/g/G extend, Esc cancels) |
| `y` | yank the selection, or the focused link's target |
| `Y` | yank the section under the cursor |
| `c` | yank the code block under the cursor, verbatim |
| `F1 / H` | this help |
| `q` | quit (steps back first when the history is deep) |
| `Ctrl-C` | quit immediately, whatever the history depth |

[`src/pager/keys.rs`](src/pager/keys.rs) is the single source of truth for this
table: the dispatcher, the in-app help overlay and the rows above all come from
the same `BINDINGS` array.

## Reading a CSV

```sh
tread events.csv
tread --format csv --delim tab dump.txt
psql -c 'copy ... to stdout csv header' | tread
```

The point of CSV support is **files too big to load**. Nothing reads the whole
file: opening stats it, sniffs the delimiter and samples the first ~1000 rows to
size the columns, then each frame renders only the rows on screen by seeking to
their byte offsets. The row index grows a bounded amount per frame and per idle
tick, so `q` returns immediately whatever the file size, and the status bar says
`≥N (indexing 12%)` while the total is still unknown rather than inventing one.

- **The header is pinned.** It stays on screen while the body scrolls, and
  scrolls sideways with it — the two are drawn from one column layout, so they
  cannot drift apart.
- **`h`/`l` move a whole column**, not four characters. The column they land on
  is the one the status bar names, the one `w` widens and the one `y` copies.
- **Widths are sampled, so a later value can overflow.** It is truncated with a
  visible `…` rather than being allowed to break the grid. `w` fits the column
  under the cursor to the widest value *currently on screen* — instant on any
  file size, and pressing it twice on the same screen changes nothing.
- **Yanks are source-faithful**, never the padded display form: `y` copies the
  cell, `Y` the row, `c` the column and `y` in visual mode the selected rows —
  always re-quoted, so a value holding a comma or a quote comes back as
  something a CSV parser accepts.
- **`G` scans, and says so.** The end of a file that has not been indexed yet is
  not known, so `G` does not jump to the end of the indexed prefix and pretend:
  it runs the scan a slice at a time, counts up in the status bar
  (`scanning to end of file… 62%`), and stops on any key press. Whatever was
  scanned is kept, so pressing `G` again resumes. `q` still exits at once.
- Parsing is RFC 4180: quoted fields, embedded newlines and delimiters, `""`
  escapes, BOM, CRLF, ragged rows padded to the header's arity. Malformed input
  degrades to something readable and never panics; a control character in a
  cell is shown as `·` rather than sent to the terminal.
- A file that announces itself as UTF-16 or UTF-32 with a byte-order mark is
  **refused by name** — `tread reads UTF-8 — convert it first, e.g. iconv -f
  UTF-16 -t UTF-8` — because a lossy render of it is mojibake that says nothing.
  Invalid UTF-8 inside an otherwise UTF-8 file is still just `�`.
- A named pipe or device given by path (`tread /dev/fd/3`) is read as a stream,
  the way piped stdin is: there is no size to stat and no offset to seek to.

A CSV has no sections and no links, so `o`, `za`, `Tab` and `n` say so instead
of pretending.

## Working a corpus

A corpus is any directory of markdown whose `README.md` links out with relative
paths — notes, a docs tree, a wiki.

```sh
tread --index ~/notes            # start at the index
tread ~/notes/guides/setup.md    # start anywhere; `i` finds the index
```

- Relative links resolve against the current document's directory. A link that
  climbs out of the index root is refused, not followed.
- Following a link pushes onto a history stack; `Backspace` pops, `+` goes
  forward again. The status bar shows `[3 back]` when the stack is deep.
- `i` opens the index view: every linked document, grouped by the H2 section it
  appeared under, filterable with `/`, navigable with `j`/`k`/`Enter`.
- `]` and `[` walk the corpus in index order, without returning to the index.
- `#anchor` links scroll to the matching heading using GitHub-style slugs.
- `http(s)` and other external links are never opened. The URL shows in the
  status bar and can be yanked.

The status bar reads:

```
file.md  ·  42%  ·  line 120/840  ·  [3 back]  ·  <link under cursor>
```

Transient messages (yanked, search wrapped, no match) replace it for ~2s.

## Zero dependencies

`[dependencies]` is empty, `[dev-dependencies]` does not exist, and there is no
build script. The markdown parser, the wrapper, the Unicode width table, the
ANSI writer, the key decoder and the argument parser are all in this repo, and
every syscall is a hand-written `extern "C"` declaration.

```
$ cargo tree
tread v0.1.0
```

All `unsafe` lives in the platform backends under [`src/sys/`](src/sys/); every
other module carries `#![deny(unsafe_code)]` or contains none. Frames go
through the `Term` buffer as one write per frame, so there is no `println!` in
any UI path.

## Docs

How it is built and how it is proven lives in [`docs/`](docs/) — the module
map, the test layers, and what is and is not verified about the Windows
console backend. [`SPEC.md`](SPEC.md) is the binding contract for behaviour.
