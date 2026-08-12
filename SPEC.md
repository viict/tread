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
src/url.rs         URL syntax: scheme_of / is_external (a leaf; no imports)
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

### Frontmatter

A leading `---` … `---` block is **rendered, not skipped**: in a documentation
corpus it holds the status, the owner and the cross-references, which is most
of what a reader wants before reading anything. It **starts folded**, to one summary
line: the status, the short scalars, and a count for each list
(`Active · viict · 5 related`). Open, it is an aligned key/value column closed
by a rule — dim labels, list values stacked under their key without repeating
it. The summary row is the fold handle and counts its own contents, so the
painter's usual `(N lines)` is suppressed there. `y` on a field row copies that
value, as it copies a cell in a CSV.

A dump is not a viewport: `--plain`, `--no-alt` and a piped render unfold
everything first, since a folded block in a pipe is missing output rather than
something the reader can open.

Two values mean more than text. `status` is coloured by its first word (live /
in flight / historical), so a trailing explanation does not stop it reading as
superseded. A value that is a path ending `.md`, with no whitespace, becomes a
real link and so is reachable with `n` and `Enter`; prose that merely mentions
a filename is not. Parsing covers what such corpora use — scalars, `-` lists,
and wrapped continuation lines — and nothing more: guessing at anchors or
nested maps would be a parser pretending to a generality it does not have. An
unterminated `---` is a thematic break, not a block that swallows the file.

A link whose first segment is the index root's own folder name
(`codex/models/X.md`, written inside `codex/`) resolves from the root when
that finds a real file. Only as a fallback, and only when the target exists.

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
h/l            horizontal scroll (code blocks, wide tables)
←/→            select a link on the row; scrolls where the row scrolls
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
  --no-browser       never hand an external link to the system opener
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
- `i` jumps to the index; the index view lists every linked *document*, grouped
  by the H2 section it appeared under, and is navigable with j/k/Enter. A link
  to something that is not a document — a script, an image, a data file — still
  resolves and still opens from the body of the document that wrote it, since
  the reader can show any file; it is simply not an entry in the corpus's table
  of contents, which is what this listing, `]`/`[` and the outline overlay all
  read.
- Anchor links (`#some-heading`) scroll to the matching heading, GitHub-style
  slug matching.
- External (`http`, `https`, `mailto`) links open in the system browser on
  `Enter`, and are coloured apart from links that stay inside the reader, so it
  is visible before pressing which links leave. `--no-browser` restores the
  old behaviour of showing the URL and refusing to open it.
- Links to files the reader can show — any file, since unknown types render as
  plain text — open in the reader. The URL is always yankable.

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
- **`sep=<char>` on the first line** is Excel's delimiter directive: it names
  the delimiter and is consumed, never shown as a row. `--delim` overrides it.
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
  In the form, `y` copies the field under the cursor — the value verbatim, not
  the sanitised form on screen and not re-quoted, because there you are reading
  a value rather than exporting a record.
- **Yank**: `y` the cell, `Y` the row as valid CSV, `c` the column. Always
  source-faithful — re-quoted correctly, never the padded display form.

Malformed input never panics: an unterminated quote at EOF, a 50MB single cell,
10k columns and embedded NULs all degrade to something readable.

## JSON

Hand-written, RFC 8259, no dependencies. The reader must open a large JSON file
as fast as it opens a large CSV, which means the same discipline: **nothing
reads the whole file on the open path.**

### Structural indexing

A container is indexed by *byte range*, not parsed:

- Finding the boundaries of a container's immediate members is a linear byte
  walk with a depth counter, an in-string flag and an escape flag. It builds no
  values and allocates nothing per member beyond an offset.
- The scan is **lazy and incremental**, exactly as the CSV row index is: enough
  to paint the first screen, extended as the viewport moves and on idle ticks.
- **Expanding a node indexes that node's members the same way.** Laziness is
  therefore not limited to the top level: a document that is one object holding
  one enormous array stays instant, because each level is indexed only when it
  is opened.
- A member is parsed into a value only when it is shown. The size cap applies
  to *one member*, not to the document, and a member too large to parse says so
  by name and number rather than being loaded until the machine suffers.

There is deliberately **no cache**: no derived file, no invalidation rule, no
second copy of the reader's data on disk. Making the first open cheap removes
the reason to have one.

### `.jsonl` / `.ndjson`

A record per line — logs, exports, agent trajectories. Indexed lazily by line
offset, records parsed only when shown. A single line may itself be tens of
kilobytes, so per-record parsing is lazy too.

A line that is not valid JSON renders as an error row carrying the reason and
the line number, and does not stop the file. Half a log is still worth reading.

### `--to-jsonl`

Writes a top-level array to stdout as one element per line, so a document can be
turned into the record form deliberately:

```sh
tread --to-jsonl big.json > big.jsonl
```

An export, not a cache: the reader never writes it on its own, and there is no
staleness question because it exists only when asked for. Applies to a top-level
array; anything else is refused with the reason. It streams — it must not hold
the document in memory to write it.

### Values

- Numbers keep their **source text**. `1e999`, `0.1` and a 40-digit integer all
  display exactly as written: a reader that round-trips through `f64` shows
  something the document does not say.
- Duplicate keys are kept, in order, for the same reason.
- Parsing must not recurse on nesting depth — an explicit stack, so ten thousand
  levels of `[[[[` is a refusal or a flat render, never a blown stack. The
  renderer, the serialiser and the fold-range computation must not recurse
  either: an iterative parser behind a recursive walker is still a crash.

### The tree

- Root open, everything under it folded. A collapsed node summarises itself:
  `{…5 keys}`, `[…120 items]`. Counts come from the index, so summarising a node
  does not require parsing it.
- Keys, strings, numbers, booleans and `null` are coloured distinctly. Strings
  are shown quoted, because `"1"` and `1` are different values.
- The status bar names the path of the row under the cursor: `.users[3].name`.
- `y` copies the value under the cursor, `Y` the subtree as valid JSON.
- Control characters inside strings go through the shared sanitiser.

### Lenses

`--lens <name>` selects a semantic view over a record file, for records whose
shape is known. Without it, records render as the generic tree; the flag only
ever *adds* interpretation, and an unrecognised record falls back to the generic
rendering rather than being hidden.

The first is agent trajectories, where the generic tree is close to useless: a
run is a conversation, and what a reader wants is the conversation with the
mechanics folded away. Messages stay visible; consecutive tool calls and their
results collapse into one summary row (`⟨6 steps · 4 tool calls⟩`) that opens.
Two dialects read one today — `agent` for Claude Code session logs, `atif` for
ATIF trajectories — and a third is a module and a line.

**A row is a headline; the message is under it.** A summary row is one line —
who, when, what — and for a message that `what` **is the message's own first
line**, at the current width; the rest of what was said is wrapped under it,
indented to the same column. One wrap, split between the row and the rows below
it, so the opening words of a message are on the screen once and a message that
fits on its row has nothing under it. A message that opens with blank lines
starts on the first line that says something: a headline is what was said, and
the blank rows would otherwise be the only thing the reader got. It has two
states and no third:
**clipped** to a few lines,
whose last row says what it is not showing (`⋯ +37 lines`), and **whole**, which
`Enter` / `za` on the row toggles. A message the clip already shows in full has
only one of those states, and the key then means what it means on any other row
— the record's own fold — rather than repainting the same screen. A clip may
never be silent about what it
left out, and the record itself is never further than `zt`, which opens the raw
tree of the record under the cursor whatever its message is doing. `zR` shows
every message the viewport has reached in full — which is what a batch
(`--plain`, `--toc`) is — and `zM` puts them all back to their clip.

A message therefore makes an item as tall as the width allows, and a resize
re-wraps it. That is the one place in a record document where rows move without
a fold changing, and it is why a mark into a file read *through a lens* is the
**record**: the cursor comes back to what it was reading, on that record's own
row. With no lens nothing wraps, and a mark there is the row it always was.

**Records inside a document.** A record file is usually one record per line, but
a trajectory in the ATIF interchange format is a single JSON document whose
records are the elements of a named array, alongside top-level keys describing
the run. A dialect therefore *declares* where its records live, and `--lens`
reads whichever file that names: a `.jsonl` for a record-per-line dialect, one
`.json` document for a records-in-a-document one. Pointing either at the other
is refused by name rather than rendered wrongly — a document read as records is
one enormous record, which is not an error a reader can see.

Two rules follow from a document holding more than its records:

- **The keys that are not the records are record 0.** `schema_version`,
  `session_id`, `agent` — whatever the envelope holds — get a summary row above
  the first record, which opens into their generic tree. A lens adds
  interpretation and never hides data, and that applies to the document around
  the records as much as to the records. The cost is that record numbering is
  shifted by one against the array's own indices, which the status bar and `#n`
  say plainly.
- **Finding the records costs one structural scan, and no parse.** The
  structural index knows a member only once it has walked past that member's
  last byte, and the array is one member — so the first row of a document lens
  waits on a byte walk of the file, reported as `≥N (indexing P%)` like every
  other scan, with nothing parsed and nothing held in memory. After that it is
  the ordinary contract: a record is parsed when it is painted and not before,
  at any file size. A record per line has no such wait, which is the reason to
  prefer that envelope when a format is being designed rather than read.
  A **batch** — `--toc`, `--plain` — waits for that scan rather than printing
  what one slice happened to reach: a short list and a zero exit status is the
  one answer a script cannot tell from "this file has no records".

## Plain text

A file whose extension names no parser renders **verbatim**: no headings, no
wrapping, no inline markup, nothing invented. A shell script is not markdown,
and parsing it as markdown turns `# comment` into a banner heading — which is
worse than doing nothing at all.

Everything that does not depend on structure still works: scrolling, search,
selection, yank, horizontal scrolling of long lines. What a text file has no
answer for — an outline, folds, links — says so rather than pretending.

`--toc` on a format with no outline prints **nothing** and exits 0. That is the
honest empty answer, and it is the one a script can use: prose on stdout saying
"this has no headings" would corrupt the pipeline that asked.

`--format text` forces it for a file whose extension would otherwise claim a
parser. This is also what makes a corpus link to a `.sh` or a `.conf`
followable, instead of refusing it as "not markdown".

## Selecting links on a line

`←`/`→` move the link focus along the current row, so a line holding several
links can be walked without `n` carrying the cursor off it.

The two cases **do** apply to the same row: a markdown table wider than the
terminal marks every one of its rows horizontally scrollable, links and all, and
so does any row under a `--width` greater than the terminal's. The rule is
therefore "links win where there is a choice to make":

- A row carrying **more than one link** gets the link walk. That is the motion
  nothing else can make, and a table row holding four links is the case the
  binding exists for.
- Every other row **scrolls if it can** — a code block, a CSV row, a text line,
  a table row with one link or none.
- `h`/`l` scroll everywhere regardless, so a wide linked table is still
  scrollable with one keypress.

The walk stops at the row's ends rather than carrying onto the next row, and is
silent at both ends and on a row with nothing to do: these are held-down keys.

## Opening a link outside the reader

`Enter` on an external link hands the URL to the system's opener. The rules
exist because a document is untrusted input:

- **Scheme allowlist**: `http`, `https`, `mailto`. Nothing else is ever handed
  over — a `file:`, `javascript:` or `vbscript:` URL in a document is refused
  by name.
- **Never through a shell.** The URL is one argument to one process
  (`xdg-open`, `open`, or `rundll32 url.dll,FileProtocolHandler`), never a
  string a shell will re-interpret. On Windows in particular, `cmd /c start`
  is not used: its quoting rules make a hostile URL a command-injection.
- The reader does not wait for the browser, does not read its output, and a
  missing opener is a status-bar message rather than an error.
- `--no-browser` disables it entirely.

## Installing on Windows

`install.ps1`, invoked the way Windows users expect:

```powershell
irm https://raw.githubusercontent.com/viict/tread/master/install.ps1 | iex
```

Same contract as `install.sh`: pick the build for the machine's architecture,
verify it against the release's `SHA256SUMS`, refuse to install anything that
does not match, and install to a per-user location on `PATH`
(`%LOCALAPPDATA%\Programs\tread` by default, `$env:INSTALL_PATH` to change it).
It reports how to add the directory to `PATH` when it is not already there.

## Directories

A directory is something to read, not an error. `tread some/dir` lists it, and a
corpus link to a directory opens the listing when there is no `README.md` to
prefer — the current refusal, "directory has no README.md", tells the reader
what is missing rather than showing them what is there.

- Directories first, each with a trailing `/`, then files with a human size and
  the format `tread` would read them as. Sorted case-insensitively within each
  group, so a listing reads the way a person would write it.
- **Dotfiles are hidden but counted.** The header says how many, and `a` toggles
  them: hiding what exists without saying so would be lying about the
  directory's contents.
- Every entry is a link, so `n` walks them, `←`/`→` select along a row and
  `Enter` opens one — the corpus navigation that already exists, not a second
  mechanism. A directory entry opens as another listing, so a tree can be walked
  down and `Backspace` walks back up.
- `README.md` still wins when it exists: a directory that documents itself
  should show its documentation.
- Relative links resolve against the directory itself rather than its parent,
  because for a listing the directory *is* the document's location.
- A directory that cannot be read (permissions) says so and stays a listing with
  no entries, rather than becoming a fatal error.

## Code

A source file is read as its comments and declarations, not as 400 lines of
text. `tread src/csv/delim.rs` shows the file's comments, every declaration's
doc comment and signature, and nothing else; each body is folded shut behind its
signature with a count of what it hides.

- **Any block folds, not only a declaration.** `za` on an `if`, a `for`, a
  `match` arm or a bare `{` folds that block and nothing else: a region ends
  where it closes, so the statement after the closing brace stays on screen.
  Blocks are foldable but never folded on open — a reader opens a function *to*
  read it — and a block under three lines is not foldable at all, since the
  marker would be no shorter than what it hid.
  In **Python** a block is a line ending in `:` and the lines indented under
  it — there are no braces to count — and `def`/`class` are excluded, because a
  declaration already owns its body and a second region over the same lines
  would be two folds for one thing.
- Blocks are not outline entries. Listing every branch under `o` would bury the
  declarations the outline exists for, so the fold key reaches them directly
  (`Source::fold_here`).
- **A declaration is a heading and its body is what that heading hides.** This
  is the whole design: folding, the `o` outline, `zo`/`zc`/`zR`/`zM`, `]`/`[`,
  `#anchor` jumps and the fold counts are the machinery markdown already used,
  so a code file needs no view of its own.
- The doc comment belongs to the heading, not to the line above it — it is what
  the collapsed view exists to show, and a comment left outside the heading
  falls inside the *previous* symbol's fold.
- **`a` toggles the summary and the source.** Every line of the file is
  rendered; unfolding everything *is* the raw file, so the two views cannot
  disagree.
- A method is nested one level under its `impl` or `trait`, so folding the
  container folds its members. An item declared inside a function is not listed.
- **A file that does not lex cleanly has no outline at all** and is shown as
  plain source, with the status bar saying so. This is deliberate: a mis-read
  brace swallows the rest of the file into one body and *hides* it, so a wrong
  outline is worse than none.
- Code is never reflowed. A row wider than the viewport scrolls sideways, the
  way a fenced code block already does.
- **Colouring is the lexer's, and there are four colours**: keyword, string,
  number, comment. The same tokens that find the declarations say which bytes
  are a comment or a literal, so a block comment spanning twenty lines is a
  comment on all twenty and a `fn` inside a string is never a keyword — neither
  of which a line-by-line highlighter can know. Four hues is a limit, not a
  starting point: a reader is scanning for shape, and every extra colour
  competes with the search highlight and the cursor row.
- The whole file is laid out at open, like markdown and unlike CSV. A source
  file is small; a generated bundle of 200k lines would cost memory in
  proportion, and that is a stated trade rather than an oversight.
- **An import that names a file in this tree is a link, one per imported
  name.** `use super::parse::{Records, QUOTE}` is two links, and `Enter` on
  either lands on *that declaration* in the target rather than at the top of the
  file — the url carries the name as an anchor, and the target's fold ids are
  its symbol paths, so nothing new is needed on the other side. `n` walks them
  and `Backspace` comes back.
- Rust resolves `crate::`, `super::` and `self::` against the nearest `src/`,
  and **`mod foo;` is a link like any import**. TypeScript resolves `./` and
  `../` through the extensions it knows and a directory's `index`, and reads
  `X as Y` as `X` — the anchor must match what the *target* declares. A bare
  specifier (`react`, `std::fs`) is left as text: its source is not here.
- **`tsconfig.json` path aliases are read**, including `baseUrl`, exact and
  wildcard patterns, and `extends`. Without them the reader is useless on real
  application code, which is written `@/components/…` rather than `../../`. The
  file is JSON with comments and trailing commas, so it is sanitised by the
  JavaScript lexer — the one that knows a `//` inside a string is not a comment
  — before being parsed. The search stops at a `node_modules` boundary: a
  dependency's config says nothing about this file.
- **A link is a relative path, never an absolute one.** A leading `/` means
  *relative to the corpus root*, so an absolute path would be looked for beneath
  the root and reported missing.
- **A code file's corpus is its project** — the nearest ancestor holding `.git`,
  `Cargo.toml`, `package.json`, `tsconfig.json`, `go.mod` or `pyproject.toml`.
  Markdown discovers its corpus from a `README.md` that links to the document;
  nothing links to `page.tsx`, so without this the root would be the folder the
  file happens to sit in and every import of a sibling directory would be
  refused for escaping it.
- **A workspace package is followed like any other import.** In a monorepo a
  package's own code is imported by *name*, and nothing in `tsconfig.json` says
  where that name lives. Members come from `pnpm-workspace.yaml` or from
  `workspaces` in `package.json` (npm, yarn, bun; turbo declares none of its own
  and rides on whichever is there), and the subpath is resolved through the
  package's `exports` map — exact keys before wildcards, and `types` before a
  build output, because the source is what a reader wants. A package naming no
  entry point at all opens as its directory listing.
- An extension is **appended** when probing, never substituted: `payload.config`
  is `payload.config.ts` and `storyblok` may be `storyblok.d.ts`. Substituting
  is tried only for a real module extension, which is the `./x.js` that means
  `x.ts` case.
- In **Rust**, a name equal to the target file's own stem carries no anchor:
  `use crate::theme` names the module, so it opens at the top rather than
  chasing a symbol called `theme`. The rule is Rust's alone — TypeScript names
  a file after the thing it exports, so `Widget.tsx` exporting `Widget` is the
  commonest import in the language and must keep its anchor.
- **An identifier is never a link.** Resolving one to its definition needs
  types, and a jump to the wrong `new` is worse than no jump. Within a file the
  outline already goes anywhere, since every fold id is a symbol path.
- **Nothing here is semantic.** The reader never compiles anything: items
  produced by a macro do not exist, `cfg`-ed out code is still listed, and two
  declarations of one name are two entries.
- Languages are compiled in, never loaded: Rust, JavaScript/TypeScript, Python
  and Java. An extension that names no known language stays plain text
  (§Plain text).
- **Python is measured in columns, not braces.** Blocks are indentation, so
  every `def` sits at brace depth zero and depth has to come from the indent.
  Its docstring is the first statement of the *body* — the part that folds — so
  it is pulled up into the signature rows; folding a Python function leaves
  `def f(x):` with its documentation under it, which is the point of the view.
  Indentation itself is never judged: a file indented oddly still outlines,
  because refusing it would invent a rule Python does not have.
- **Imports fold as a run, never one at a time.** A block of them is a wall a
  reader scrolls past, so consecutive imports (and Rust's `mod`) collapse behind
  one fold that is shut on open, the way frontmatter is. A single import is left
  alone: folding it would hide nothing worth hiding.
- A folded run says **what** it hides rather than how much — `38 symbols from 12
  modules` — because the useful figure for imports is how many names came from
  how many places, not how many lines they took. Any source may answer this;
  the line count is the default.
- An import names the module it pulls from rather than the bindings it
  introduces — that is what a reader follows.
