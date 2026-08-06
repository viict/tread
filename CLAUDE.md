# rmarktui

A terminal markdown reader — `less`, but it understands markdown: collapsible
headings, banner H1s, real tables, colored links, and navigation across a
corpus of linked documents. Crate `rmarktui`, binary `mdr`.

`SPEC.md` is the binding contract. Read it before changing behaviour.

## Non-negotiables

- **Zero dependencies.** `[dependencies]` and `[dev-dependencies]` stay empty
  forever — no `libc`, no `crossterm`. Syscalls are hand-written `extern "C"`
  declarations. Tests use the built-in `#[test]` harness only.
- **All `unsafe` lives in the backend modules under `src/sys/`** — today
  `unix.rs` and its two per-OS halves `unix_linux.rs` / `unix_darwin.rs`. Every
  other module, including `src/sys/mod.rs`, `src/sys/abi.rs` and
  `src/sys/layout.rs`, carries `#![deny(unsafe_code)]` or contains none.
- **Must build for `x86_64-unknown-linux-musl`** (static) **and check clean for
  `{x86_64,aarch64}-apple-darwin`.** No glibc-only APIs, and no Linux constant
  reused for Darwin without being verified against the `xnu` headers first.
- **The mouse is never captured** (no `?1000h`/`?1002h`/`?1006h`), so
  terminal-native drag-select keeps working. Product requirement.
- Files < 500 lines, functions < 50 lines — split modules instead of growing.
- No `println!`/`eprintln!` for UI; frames go through `Term`. `eprintln!` is
  allowed only for fatal startup errors, as `mdr: <message>`.
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
| `src/sys/stub.rs` | non-unix placeholder backend, same surface, no `unsafe` |
| `src/term/` | raw-mode guard, alt screen, ANSI, frame buffer, OSC 52 |
| `src/key/` | byte stream to `Key` decoder |
| `src/md/` | `mod.rs` (`parse`), `block.rs`, `inline.rs`, `ast.rs`, `sanitize.rs` |
| `src/render/` | AST + width to styled wrapped lines |
| `src/theme.rs` | palette, heading styles, banner glyphs |
| `src/pager/` | viewport, scrolling, collapse tree, search |
| `src/nav/` | document stack, index parsing, link resolution |
| `src/select/` | visual selection, yank text |

Keep platform FFI inside a backend under `src/sys/`; everything above `sys` is
platform-agnostic. Adding an OS is a new file next to `unix.rs`, its constants
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
cargo check --target aarch64-apple-darwin   # and x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin   # needs a Mac / Apple SDK

# against the target corpus (106 files, table-heavy, README.md index)
cargo run -- --index ~/rmarktui/codex
cargo run -- ~/rmarktui/codex/README.md --toc
cat ~/rmarktui/codex/README.md | cargo run -- --plain
```

Unit tests live beside the code in `#[cfg(test)]`; golden renders live in
`tests/`. Assert on ANSI-stripped text plus separate style spans so palette
changes do not break every test. Goldens are checked in under `tests/golden/`
from fixtures in `tests/fixtures/`; regenerate with
`UPDATE_GOLDEN=1 cargo test --test golden_files`.

Two soak harnesses run outside `cargo test` and must stay green before shipping:

```sh
tools/soak.sh target/x86_64-unknown-linux-musl/release/mdr ~/rmarktui/codex
tools/soak_pty.py target/x86_64-unknown-linux-musl/release/mdr ~/rmarktui/codex
```

`WINDOWS.md` is the contract for a second `sys` backend. If a change would make
that document wrong, the change is above `sys` and belongs somewhere else.
