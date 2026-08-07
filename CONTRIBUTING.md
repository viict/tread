# Contributing to tread

Thanks for looking. This document exists mostly to save you effort: `tread` has
a few constraints that are unusual enough that a well-written patch can still be
unmergeable, and it is fairer to say so before you write it than after.

`SPEC.md` is the binding contract for behaviour, and `CLAUDE.md` describes the
module layout. Read whichever covers what you are touching.

## What will be declined

These are not preferences to be argued out of. They are the reason the project
exists in this shape.

- **Adding a dependency.** `[dependencies]` and `[dev-dependencies]` are empty
  and stay empty — no `libc`, no `crossterm`, no `windows-sys`, nothing for
  tests. The markdown, CSV and JSON parsers, the Unicode width table, the ANSI
  writer, the key decoder, base64 and every syscall are hand-written here on
  purpose. A patch that adds a crate will be declined even if the crate is
  better than what it replaces.
- **Capturing the mouse.** No `?1000h`, `?1002h`, `?1006h`; on Windows, never
  setting `ENABLE_MOUSE_INPUT` and never clearing `ENABLE_QUICK_EDIT_MODE`.
  Terminal-native drag-select must keep working, because being able to select
  text with the mouse is a product requirement — not a missing feature.
- **`unsafe` outside the backends.** All of it lives in `src/sys/unix.rs` and
  `src/sys/windows/ffi.rs`. Everything else carries `#![deny(unsafe_code)]` or
  contains none, including the pure `abi.rs` / `layout.rs` files.
- **A platform `#[cfg]` above `src/sys/`.** Path syntax, cache locations and
  similar *conventions* are pure functions of `plat::Platform`, so the other
  OSes' rules are covered by `cargo test` on Linux. A new `cfg(target_os)` in
  `render`, `pager` or `nav` is a design problem, not a fix.
- **`println!` / `eprintln!` for UI.** Frames go through `Term`. `eprintln!` is
  only for fatal startup errors, as `tread: <message>`.
- **Files over 500 lines or functions over 50.** Split the module instead.

## What is welcome

Bug reports with a file that reproduces them, especially from a terminal or OS
combination the project cannot test. New `--lens` dialects (see
`docs/lenses.md` — a module plus one line in `LENSES`). New formats behind the
`Source` seam. Rendering fixes for scripts and widths that are wrong today.
And anything on Windows: see below.

## Before opening a pull request

```sh
cargo test                                   # must be green
cargo clippy --all-targets -- -D warnings    # CI gates on this
cargo check --target x86_64-pc-windows-msvc   # and aarch64-pc-windows-msvc,
                                              # and x86_64-pc-windows-gnu
cargo check --target aarch64-apple-darwin     # and x86_64-apple-darwin
```

CI runs the full suite natively on Linux, macOS and Windows for both
architectures, so you do not need those machines yourself — but a cross `check`
locally catches the common mistakes in seconds rather than minutes.

`cargo fmt` is **not** run on this tree. It follows a hand style and rustfmt
would rewrite thousands of lines; please match the surrounding code instead of
reformatting it.

Tests live beside the code in `#[cfg(test)]`; golden renders live in `tests/`.
Assert on ANSI-stripped text plus separate style spans, so a palette change does
not break every test. Regenerate goldens with
`UPDATE_GOLDEN=1 cargo test --test golden_files`. `CLAUDE.md` § *Build, run,
test* has the rest — the corpus checks and the soak harnesses — and is where a
change to any of this belongs.

## Windows especially

The console backend is hand-written against `kernel32` and is the least-proven
part of the project: the suite runs on real Windows runners in CI, but there is
no ConPTY soak harness, so the interactive pager on Windows has only been
exercised by hand. `docs/windows.md` is precise about what is and is not
verified. If you run `tread` on Windows and something is wrong — rendering,
resize, drag-select, exit behaviour — that report is more valuable than most
patches, and please say which terminal (Windows Terminal, conhost, VS Code) and
which PowerShell.

Do not claim a Windows behaviour works unless a command printed it or you
watched a session do it, and keep that document's verified / not-verified lists
honest when you change the backend.

## Commits

`scope: imperative description`, lowercase — `code: read python and java`, not
`add DirSource`. The scope is *where* the change lives, not what kind of change
it is; there is no `feat:` / `fix:` vocabulary, because nothing is generated
from the log. Say what changed in behaviour and let the diff show the files.

One logical change per commit, each building and passing on its own, and please
fold a fix-up into the commit it repairs before asking for a merge. A feature
branch is not squashed to a single commit — a large feature is exactly what
someone will `git bisect` through later.

`CLAUDE.md` § *Commits and history* is the full version of this rule and the
one that wins if these two ever disagree.
