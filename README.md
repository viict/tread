# tread — `tread`

`less`, but it understands markdown.

A terminal markdown reader with collapsible headings, banner H1s, real
box-drawn tables, colored links, and navigation across a corpus of linked
documents. One static binary, no runtime, no configuration, **no dependencies
at all** — not even `libc`.

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
[`WINDOWS.md`](WINDOWS.md) records what the console backend does and what is
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
  --toc            Print the heading outline and exit.
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
| `h / ←` | scroll left (code, wide tables) |
| `l / →` | scroll right (code, wide tables) |
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

## Layout

| Path | Role |
| --- | --- |
| [`src/main.rs`](src/main.rs) | entry, wiring, panic guard, exit codes |
| [`src/cli.rs`](src/cli.rs) | argument parser, `--help`/`--version` |
| [`src/sys/`](src/sys/) | the platform seam and its backends — the only `unsafe` in the tree |
| [`src/plat/`](src/plat/) | per-OS conventions above `sys`: path syntax, file locations |
| [`src/term/`](src/term/) | raw-mode guard, alt screen, ANSI, frame buffer, OSC 52 |
| [`src/key/`](src/key/) | byte stream to `Key` decoder |
| [`src/md/`](src/md/) | the markdown parser: block, inline, table, list, AST |
| [`src/render/`](src/render/) | AST + width to styled wrapped lines |
| [`src/theme.rs`](src/theme.rs) | palette, heading styles, banner glyphs |
| [`src/pager/`](src/pager/) | viewport, scrolling, collapse tree, search, keymap |
| [`src/nav/`](src/nav/) | document stack, index parsing, link resolution |
| [`src/select/`](src/select/) | visual selection, yank text, clipboard |
| [`src/dump.rs`](src/dump.rs) | non-interactive render for pipes and `--no-alt` |

Every file is under 500 lines and every function under 50; modules split rather
than grow. Adding an OS means a new backend beside the others and one arm in the
dispatch — nothing above `src/sys/` is platform-specific.

## Testing

```sh
cargo test
cargo clippy --all-targets
```

Unit tests live beside the code; [`tests/`](tests/) drives the real binary with
golden renders, adversarial input (invalid UTF-8, NUL, CRLF, 5000-char words,
500-deep quotes, unclosed fences) and a check that this file's key table matches
the code. Goldens hold ANSI-stripped text only, so a palette change cannot break
a layout assertion — regenerate them with
`UPDATE_GOLDEN=1 cargo test --test golden_files`.

Two soak harnesses go beyond `cargo test`, rendering a whole corpus at four
widths and driving the pager through a real pty:

```sh
tools/soak.sh    target/x86_64-unknown-linux-musl/release/tread ~/notes
tools/soak_pty.py target/x86_64-unknown-linux-musl/release/tread ~/notes
```
