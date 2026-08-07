# tread

A terminal reader for markdown, CSV and JSON — `less`, but it understands markdown: collapsible
headings, banner H1s, real tables, colored links, and navigation across a
corpus of linked documents. Crate `tread`, binary `tread`.

`SPEC.md` is the binding contract. Read it before changing behaviour.

## Non-negotiables

- **Zero dependencies.** `[dependencies]` and `[dev-dependencies]` stay empty
  forever — no `libc`, no `crossterm`. Syscalls are hand-written `extern "C"`
  declarations. Tests use the built-in `#[test]` harness only.
- **All `unsafe` lives in the backend modules under `src/sys/`** — `unix.rs`
  with its two per-OS halves `unix_linux.rs` / `unix_darwin.rs`, and
  `windows/ffi.rs`. Every other module, including `src/sys/mod.rs`,
  `src/sys/windows.rs`, `src/sys/windows/io.rs` and the four pure
  `abi.rs`/`layout.rs` files, carries `#![deny(unsafe_code)]` or contains none.
- **Must build for `x86_64-unknown-linux-musl`** (static) **and check clean for
  `{x86_64,aarch64}-apple-darwin` and `x86_64-pc-windows-{msvc,gnu}`.** No
  glibc-only APIs, no Linux constant reused for Darwin without being verified
  against the `xnu` headers first, and no Win32 constant that is not re-derived
  from the SDK headers and pinned by a host test.
- **The mouse is never captured** (no `?1000h`/`?1002h`/`?1006h`), so
  terminal-native drag-select keeps working. Product requirement. On Windows
  that also means never setting `ENABLE_MOUSE_INPUT` and never clearing
  `ENABLE_QUICK_EDIT_MODE` — quick edit *is* console drag-select, and it is only
  honoured while `ENABLE_EXTENDED_FLAGS` is set.
- Files < 500 lines, functions < 50 lines — split modules instead of growing.
- No `println!`/`eprintln!` for UI; frames go through `Term`. `eprintln!` is
  allowed only for fatal startup errors, as `tread: <message>`.
- Exit codes: `0` ok, `1` runtime error, `2` usage error.

## Module layout

| Path | Role |
| --- | --- |
| `src/main.rs` | entry, wiring, panic guard, exit codes |
| `src/cli.rs` | hand-rolled arg parser, `--help`/`--version` text |
| `src/sys/mod.rs` | the platform seam: public surface, signal flags, backend dispatch, the contract |
| `src/sys/abi.rs` | pure ABI core: raw-mode flag arithmetic, `_IOR`/`_IOW` encoding, read/write classification, per-OS constant tables — no `unsafe`, host-tested |
| `src/sys/layout.rs` | pure `#[repr(C)]` struct layouts for every unix, asserted at compile time on all targets — no `unsafe`, host-tested |
| `src/sys/unix.rs` | the only `unsafe` on unix: termios, `TIOCGWINSZ`, read/write, signals, isatty; shared by Linux and Darwin |
| `src/sys/unix_linux.rs`, `src/sys/unix_darwin.rs` | per-OS halves: the `struct termios` alias and the errno accessor, nothing else |
| `src/sys/windows.rs` | the Windows contract: handles, raw mode, size, restore, `SetConsoleCtrlHandler` |
| `src/sys/windows/ffi.rs` | the only `unsafe` on Windows: hand-written `kernel32` declarations and the `Fd` → `HANDLE` table |
| `src/sys/windows/io.rs` | `read_input` (wait + peek + `ReadFile`), `write_all`, resize polling |
| `src/sys/windows/abi.rs` | pure console ABI: mode arithmetic, `srWindow` geometry, record/error classification, `CTRL_*` mapping — declared on *every* target, host-tested |
| `src/sys/windows/layout.rs` | pure `COORD` / `SMALL_RECT` / `CONSOLE_SCREEN_BUFFER_INFO` / `INPUT_RECORD`, compile-asserted on every target |
| `src/sys/browser.rs` | handing a URL to the system opener (SPEC.md §"Opening a link outside the reader"): the pure `argv` table — `xdg-open`, `open`, `rundll32 url.dll,FileProtocolHandler` — declared on *every* target and host-tested, plus the one `spawn` in the crate. One process, the URL as one argument, never a shell and never a command string; the child is never waited on or read. The only place in the crate that names an opener. `open` takes a `Vetted`, not a `&str`: the newtype's only constructor is handed an allowlist and refuses a scheme that is not on it, so the precondition is in the type and a second caller cannot spawn an unvetted URL. *Which* schemes are allowed stays above `sys`, in `nav::external` |
| `src/sys/stub.rs` | fallback backend for targets that are neither unix nor windows, same surface, no `unsafe` |
| `src/plat/` | pure platform *conventions* above `sys`: native path syntax (`path.rs`) and per-OS file locations (`dirs.rs`), both functions of an explicit `Platform`, host-tested for Linux, macOS and Windows at once |
| `src/url.rs` | URL *syntax*, and nothing else: `scheme_of`, `is_external`, `scheme_allowed`. A leaf module with no imports, because three layers need the same answer to "does this link leave the reader" — `render` (which colours it), `nav` (which resolves it) and `sys::browser` (which will not spawn without it) — and every one of them must read *down*. When the predicate lived in `nav`, `render` imported the navigator to paint a span. Policy is not here: `javascript:` is external *and* refused |
| `src/term/` | raw-mode guard, alt screen, ANSI, frame buffer, OSC 52 |
| `src/key/` | byte stream to `Key` decoder |
| `src/md/` | `mod.rs` (`parse`), `block.rs`, `inline.rs`, `ast.rs`, `sanitize.rs` |
| `src/csv/` | the CSV foundation: RFC 4180 `parse.rs`, lazy `index.rs` with `index_store.rs` (`RowStore`, re-exported so it stays `csv::index::RowStore`), windowed `read.rs`, `delim.rs`. `RowStore::row` is **bounded**: it spends at most one `MAX_ROW_BYTES` of scanning to find where a row ends and otherwise serves it clipped, because proving where the *last* known row ends is a scan to end-of-file on a file whose tail holds no terminator — which on the paint path is a first frame that never flushes and a `q` that is never read. The index is grammar-agnostic: `RowStore::lines` builds it over the quoting-free `Scanner::lines`, and is the **one** constructor every record-per-line format uses (`.jsonl` and plain text). There is exactly one line indexer in the crate; a second copy would drift, and running the CSV grammar over a log would let one `"` swallow the rest of the file |
| `src/json/` | the JSON foundation: RFC 8259 `parse.rs`, the source-faithful `value.rs` tree, `write.rs`, `error.rs`, and the lazy structural `index.rs` — all iterative, nothing recurses on nesting |
| `src/source/` | the format seam: `Source`, `markdown.rs`, `csv/`, `json/`, `jsonl/`, `jsonrow.rs`, `detect.rs`. Formats are compiled in, never loaded |
| `src/source/jsonrow.rs` | the **one** JSON tree-row grammar: indent, fold marker, key spelling, collapsed summary, scalar colours, path steps. Both JSON sources build every row through it, and `tests/json_differential.rs` reads the same content both ways and compares — two renderers would drift and the same object would look different depending on which file it came from |
| `src/source/json/` | JSON behind the seam: the lazily indexed `tree.rs` with `ident.rs` (a member's bytes, key and path — **derived** from the parent chain, never stored, or the paths alone cost O(depth²)), the fold state and flatten in `flat.rs`, `render.rs`, the `Source` impl in `view.rs`, and `export.rs` (`--to-jsonl`) |
| `src/source/jsonl/` | `.jsonl` / `.ndjson` behind the seam: a record per line over the *CSV* lazy line index, `rowmap.rs` (which rows an open record owns), `tree.rs` (one iterative walker: count, rows, path, over `jsonrow`'s grammar), `rows.rs`, `view.rs`, and the lens half — `plan.rs` (items: which records share a row, and the two-level row arithmetic) and `lensrow.rs` (the lens state on the source, and the rows it paints) |
| `src/source/text/` | plain text behind the seam (SPEC.md §Plain text): the file's lines, verbatim, over the *CSV* lazy line index — `mod.rs` (state, file access, the tab/control-character painting rule) and `view.rs` (the `Source` impl). No outline, no folds, no links: the honest empty answers. Anything whose extension names no parser lands here, and so does a corpus link to a `.sh` |
| `src/lens/` | the `--lens` seam (SPEC.md §Lenses): `mod.rs` holds `Lens`, `Summary` and the registry; one module per dialect (`agent.rs` — Claude Code session logs). A lens is a **transform over records**, never a `Source`: it says what one record *is* and nothing about rows, folding or search. A record it does not recognise falls back to the generic tree and is never hidden. Adding a dialect is a module plus one line in `LENSES` — `docs/lenses.md` is the contract |
| `src/open.rs` | resolving the input and building the `Box<dyn Source>` behind it; `open/lens.rs` is the one place a `--lens` meets a format |
| `src/render/` | AST + width to styled wrapped lines |
| `src/theme.rs` | palette, heading styles, banner glyphs |
| `src/pager/` | viewport, scrolling, collapse tree, search |
| `src/nav/` | document stack, index parsing, link resolution; `external.rs` is the *policy*: the scheme allowlist (`http`, `https`, `mailto`), the refusal that names the offending scheme, and `vetted()` — the one constructor in the crate of the `sys::browser::Vetted` token that `open` requires |
| `src/select/` | visual selection, yank text |

Keep platform FFI inside a backend under `src/sys/`; everything above `sys` is
platform-agnostic, and every `cfg(target_os)`-shaped *convention* (path syntax,
cache locations) is a pure function of `plat::Platform` rather than a `cfg`, so
the other OSes' rules are covered by `cargo test` on the Linux builder. Adding an OS is a new file next to `unix.rs`, its constants
added to `abi.rs` and its C struct layouts to `layout.rs` (both with host
tests), and one arm in the dispatch in `mod.rs`. Anything a backend could
compute without the OS belongs in `abi.rs` or `layout.rs`, where it is tested on
whatever host CI runs — including for OSes that host is not. A struct layout in
particular must carry `const _: () = assert!(size_of::<T>() == N);`: there is no
macOS or Windows machine in this loop, and a wrong layout otherwise compiles
cleanly and corrupts memory.

## Build, run, test

```sh
cargo build                 # native debug
cargo run -- README.md      # native run
cargo musl                  # static release (alias in .cargo/config.toml)
cargo test                  # unit + integration tests, must be green
cargo test --lib cli        # one module
cargo check --target aarch64-apple-darwin      # and x86_64-apple-darwin
cargo check --target x86_64-pc-windows-msvc    # and x86_64-pc-windows-gnu
cargo build --release --target aarch64-apple-darwin   # needs a Mac / Apple SDK

# against the target corpus (106 files, table-heavy, README.md index)
cargo run -- --index ~/notes
cargo run -- ~/notes/README.md --toc
cat ~/notes/README.md | cargo run -- --plain
```

Unit tests live beside the code in `#[cfg(test)]`; golden renders live in
`tests/`. Assert on ANSI-stripped text plus separate style spans so palette
changes do not break every test. Goldens are checked in under `tests/golden/`
from fixtures in `tests/fixtures/`; regenerate with
`UPDATE_GOLDEN=1 cargo test --test golden_files`.

Soak harnesses run outside `cargo test` and must stay green before shipping:

```sh
tools/soak.sh target/x86_64-unknown-linux-musl/release/tread ~/notes
tools/soak_pty.py target/x86_64-unknown-linux-musl/release/tread ~/notes

# scale + hostile input, per format. Both generate everything they need
# (csvgen.py / jsongen.py are deterministic) and check nothing in.
tools/soak_csv.sh  target/x86_64-unknown-linux-musl/release/tread
tools/soak_json.sh target/x86_64-unknown-linux-musl/release/tread
```

The scale claim these pin is that open time, quit time and resident memory do
not track file size. `tools/csvbench.py` and `tools/jsonbench.py` measure it
through a real pty, which is the only place the claim means anything.

`docs/windows.md` documents what the console backend does, and — importantly — what
about it is *not* verified: it type-checks for both Windows targets and its pure
logic is host-tested, but it has never been executed on Windows hardware. Never
claim it works; claim only what a command in this repo printed. If a change would
make that document wrong, the change is above `sys` and belongs somewhere else.
