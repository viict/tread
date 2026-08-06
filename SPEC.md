# tread — spec

A terminal markdown reader. `less`, but it understands markdown: collapsible
headings, visual heading hierarchy, colored links, and navigation across a
linked document tree (an "index" doc whose relative links form a corpus).

Binary: `tread`. Crate: `tread`.

## Hard constraints

1. **Zero dependencies.** `[dependencies]` in Cargo.toml stays empty. No
   `libc`, no `crossterm`, no `pulldown-cmark`, no dev-dependencies. All
   syscalls go through hand-written `extern "C"` declarations under `src/sys/`.
   Tests use the built-in `#[test]` harness only.
2. **Static musl target.** Must build clean under
   `cargo build --release --target x86_64-unknown-linux-musl`.
   No glibc-only syscalls, no `std::os::unix` APIs unavailable on musl.
3. **Portability.** Keep all platform-specific FFI isolated in a backend under
   `src/sys/`, behind the surface documented in `src/sys/mod.rs`, so a future
   `sys/windows.rs` can be dropped in. Everything above `sys` must be
   platform-agnostic pure Rust. Linux (glibc and musl) and macOS
   (`x86_64`/`aarch64`) are supported; every ABI fact that is arithmetic or
   struct layout lives in `abi.rs`/`layout.rs`, is host-tested for *all* of
   them, and is additionally pinned by `const` assertions so a wrong layout is
   a build error rather than runtime memory corruption.
4. **No `unsafe` outside the `src/sys/` backends.** Add `#![forbid(unsafe_code)]`-level
   discipline elsewhere (module-level `#![deny(unsafe_code)]` where possible).
5. **The mouse is never captured.** Do not emit `?1000h`/`?1002h`/`?1006h`.
   Terminal-native click-drag selection must keep working at all times. This
   is a product requirement, not a detail.
6. Files < 500 lines, functions < 50 lines. Split modules when they grow.
7. No `println!`/`eprintln!` for UI output. All terminal writes go through the
   `Term` writer with a single buffered flush per frame. `eprintln!` is
   allowed only for fatal startup errors before raw mode is entered.

## Module layout

```
src/main.rs        entry, wiring, top-level error handling, panic guard
src/cli.rs         hand-rolled arg parser + --help/--version text
src/sys/mod.rs     public surface + backend dispatch (no unsafe)
src/sys/abi.rs     pure ABI core: raw-mode flags, ioctl encoding, per-OS
                   constant tables (no unsafe, host-tested for every OS)
src/sys/layout.rs  pure #[repr(C)] struct layouts for every OS, with
                   compile-time size/alignment assertions (no unsafe)
src/sys/unix.rs    ALL unsafe FFI on unix, shared by Linux and Darwin:
                   termios, ioctl(TIOCGWINSZ), read(2), write(2),
                   signal(SIGWINCH/SIGINT/SIGTERM/SIGHUP/SIGQUIT), isatty
src/sys/unix_linux.rs   Linux half:  struct termios + __errno_location()
src/sys/unix_darwin.rs  Darwin half: struct termios + __error()
src/sys/stub.rs    non-unix placeholder backend
src/term.rs        safe layer over sys: raw mode RAII guard, alt screen,
                   ANSI style codes, frame buffer, OSC 52 clipboard write
src/key.rs         input byte-stream -> Key enum decoder (escape sequences,
                   UTF-8, bracketed paste guard)
src/md/mod.rs      public parse() entry
src/md/block.rs    block-level parser -> Vec<Block>
src/md/inline.rs   inline parser -> Vec<Inline>
src/md/ast.rs      Block / Inline / ListKind / TableAlign types
src/render.rs      AST + width -> Vec<Line> (styled, wrapped) layout engine
src/theme.rs       color palette, heading styles, banner glyph font
src/pager.rs       viewport state, scrolling, collapse tree, search
src/nav.rs         document stack, index parsing, relative link resolution
src/select.rs      visual/line selection state + yank text extraction
```

## Rendering rules

### Headings
- **H1**: rendered as a banner using a block-glyph font (`theme.rs`), tinted
  with the accent color, preceded and followed by a blank line. Fall back to
  plain bold-uppercase-with-rule when the H1 text contains characters absent
  from the glyph font, or when terminal width < banner width.
- **H2**: bold bright, followed by a full-width `─` rule.
- **H3**: bold cyan, indent 2. **H4**: cyan, indent 4.
  **H5**: dim cyan, indent 6. **H6**: dim italic, indent 8.
- Every heading carries a collapse marker in the left gutter:
  `▾` open / `▸` collapsed. A collapsed heading hides all content until the
  next heading of equal-or-shallower depth.

### Inline
- Links: blue (`38;5;39`), underlined; the URL is hidden in the body and shown
  in the status bar when the link is the current cursor target. Reference and
  autolinks both supported. Emit OSC 8 hyperlinks too, guarded so terminals
  that don't support it degrade cleanly.
- `code`: distinct background, no color bleed across wraps.
- Bold, italic, bold-italic, strikethrough. Nesting must work.
- Escapes (`\*`) and entity-ish literals handled.

### Blocks
- Paragraphs: word-wrapped to terminal width minus gutter, never mid-word
  unless a single word exceeds the width.
- Lists: bullets `•`/`◦`/`▪` by depth; ordered lists keep source numbering;
  task lists render `☐`/`☑`. Nested lists indent by 2 and wrap with hanging
  indent aligned to the text, not the bullet.
- Code blocks: bordered or background-tinted, language label in the corner,
  NOT wrapped — horizontally scrollable. No syntax highlighting in v1
  (leave a seam for it).
- Block quotes: left bar `▏` in dim, recursive.
- **Tables**: required, first-class. The codex corpus is table-heavy.
  Compute per-column widths from content, honor `:---`/`:---:`/`---:`
  alignment, draw with box-drawing characters, and degrade to horizontal
  scrolling when the table exceeds terminal width.
- Thematic breaks, HTML blocks (rendered dim-literal), footnotes-as-text.

### Width & unicode
Wrapping must use display width, not byte or char count: handle wide CJK
(width 2), zero-width combining marks (width 0), and not panic on emoji.
Implement a compact `char_width()` in `render.rs` from Unicode ranges — no
crate. Never slice a String on a non-char boundary.

## Keybindings

```
j/k ↓/↑        line down/up            d/u        half page
space/f, b     page down/up            g/G        top/bottom
h/l ←/→        horizontal scroll (code blocks, wide tables)
za / Enter     toggle collapse at cursor heading
zM / zR        collapse all / expand all
zo / zc        open / close current section
Tab / S-Tab    next / previous heading
n              next link;  Enter on a link follows it
Backspace / -  back in document history
o              document outline / table of contents overlay
i              open the corpus index
/  ?           search forward / backward   n/N  next / prev match
v              visual line-select mode; y yanks (OSC 52)
Y              yank the section (or code block) under the cursor
c              yank the code block under cursor, verbatim
?  or F1       help overlay
q              quit (pops nav stack first if deep)
```

## CLI

```
tread [OPTIONS] [FILE]

  --index <PATH>     treat PATH (file or dir containing README.md) as the
                     corpus index; opening with no FILE starts here
  --no-alt           render into scrollback instead of the alternate screen
  --plain            no color (also honor NO_COLOR and non-tty stdout)
  --width <N>        force wrap width
  --toc              print the outline and exit
  -h, --help / -V, --version
```

Reading from stdin (`cat x.md | tread`) must work: open `/dev/tty` for input
when stdin is a pipe.

## Navigation / the corpus

Primary target corpus: `~/notes` — 106 markdown files, a
`README.md` index whose tables link out with relative paths
(`models/SAMPLE_MODEL.md`), heavy table usage, ATX headings.

- Relative links resolve against the current document's directory.
- Following a link pushes onto a history stack; Backspace pops.
- `i` jumps to the index; the index view lists every linked doc, grouped by
  the H2 section it appeared under, and is navigable with j/k/Enter.
- Anchor links (`#some-heading`) scroll to the matching heading, GitHub-style
  slug matching.
- Non-markdown / external (`http`) links are not opened; show the URL in the
  status bar and allow yanking it.

## Status bar

`file.md  ·  42%  ·  line 120/840  ·  [3 back]  ·  <current link url>`
Left: document path relative to the index root. Right: position. Transient
messages (yanked, search wrapped, no match) replace the bar for ~2s.

## Testing

`cargo test` must pass. Unit tests live next to the code (`#[cfg(test)]`);
integration/golden tests in `tests/`. Cover: block parsing, inline parsing
including nesting and escapes, wrapping with wide/zero-width chars, table
column sizing, collapse range computation, link resolution, ANSI-stripped
golden renders. Rendering tests must assert on the ANSI-stripped text plus a
separate style-span assertion, so palette tweaks don't break every test.

---

# Multi-format reading

`tread` reads more than markdown. Formats are **compiled in**, never loaded at
runtime: no `dlopen`, no plugin files, no shared objects. One static binary
stays the shipping property, and `[dependencies]` stays empty — every parser is
hand-written in this repo.

## The `Source` seam

A document is anything that can produce rendered lines on demand:

```rust
pub trait Source {
    fn len(&self) -> usize;                                   // total rendered lines
    fn lines(&mut self, rows: Range<usize>) -> Vec<Line>;     // only what is on screen
    fn set_width(&mut self, cols: usize);
    fn outline(&self) -> &[Entry];                            // `o`, and the collapse tree
    fn search(&mut self, needle: &str, from: usize, fwd: bool) -> Option<usize>;
    fn yank(&self, rows: Range<usize>) -> String;             // source-faithful text
}
```

`pager`, `nav`, `select` and `term` hold a `Box<dyn Source>` and must never
learn which format they are showing — the same discipline that let the Windows
port change nothing above `sys`. Adding a format is one module plus one arm in
the detector.

`MarkdownSource` keeps today's eager `Vec<Line>`: markdown documents are small,
and nothing about their behaviour may change when they move behind the trait.

Format detection: file extension first; content sniff when there is no name,
which is the stdin case (BOM, a leading `{`/`[`, a plausible delimiter row).
`--format <md|csv>` overrides both.

## CSV

The point of CSV support is **files too big to load**. A multi-GB file must
open instantly and quit instantly; nothing may read the whole file on the open
path, and `q` must never wait on a scan.

- **Row index.** One lazy pass recording each row's byte offset, respecting
  quoting: a newline inside a quoted field is *not* a row boundary, and getting
  that wrong corrupts every offset after it. Rows render from their offsets on
  demand.
- **Column widths are sampled** from the first ~1000 rows, so open time does not
  depend on file size. A later value that exceeds its column is truncated with a
  visible marker rather than breaking the layout; `w` widens the column under
  the cursor on demand. Layout may shift as sampling proves wrong — that is the
  accepted trade for instant open.
- **Parsing** is RFC 4180: quoted fields, embedded newlines and delimiters,
  doubled quotes as escapes, BOM, CRLF, ragged rows padded or truncated to the
  header's arity. Delimiter sniffed among `,` `\t` `;` `|`, overridable.
- **Reading affordances**: the header row stays pinned while scrolling
  vertically; `h`/`l` scroll by column, not by character; the status bar names
  the current row, the total, and the column under the cursor.
- **`Enter` opens a row as a form** — one field per line, label beside value —
  because a record wider than the terminal is unreadable as a row. Rows
  carrying more fields than the header named keep them: the grid marks such a
  row by standing a `+` in for its left border (never an extra column, which
  would misalign it), and the form lists the surplus labelled by position.
  Padding a short row is display; dropping a long one would be data loss.
- **Yank**: `y` the cell, `Y` the row as valid CSV, `c` the column. Always
  source-faithful — re-quoted correctly, never the padded display form.

Malformed input never panics: an unterminated quote at EOF, a 50MB single cell,
10k columns and embedded NULs all degrade to something readable.
