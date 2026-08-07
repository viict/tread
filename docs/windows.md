# `tread` on Windows

This described a backend that did not exist. It now describes one that does:
`src/sys/windows.rs` plus the four files under `src/sys/windows/`, hand-written
`extern "system"` bindings to `kernel32` with no `windows-sys`, no `winapi` and
no `libc`.

**Read this caveat first.** The backend has never run on Windows. There is no
Windows machine, no Wine and no mingw linker in this project's loop. Everything
below is *implemented and type-checked* for `x86_64-pc-windows-msvc` and
`x86_64-pc-windows-gnu`, and every part of it that is arithmetic rather than a
syscall is unit-tested on the Linux builder — but "compiles and its pure logic
passes tests" is not "works". The first execution on real hardware is the first
real test. §"What is and is not verified" is precise about the line.

## The seam

Every platform call lives under `src/sys/`, and the backend modules there are
the only ones containing `unsafe`. Everything above — `term`, `key`, the parser,
the renderer, the pager, `nav`, `select` — is portable safe Rust that reaches the
platform only through the surface in `src/sys/mod.rs`.

```
main.rs / term/ / key/ / plat/    safe, portable, no #[cfg(windows)] anywhere
──────────────────────────────────────────────────────────────────────────────
sys/mod.rs        public surface + backend dispatch       (no unsafe)
sys/abi.rs        pure unix ABI arithmetic, host-tested   (no unsafe)
sys/layout.rs     pure unix C struct layouts, host-tested (no unsafe)
sys/windows/abi.rs     pure console constants + arithmetic, host-tested
sys/windows/layout.rs  pure console C struct layouts, host-tested
──────────────────────────────────────────────────────────────────────────────
sys/unix.rs (termios + ioctl)      │ sys/windows.rs (Console API)
  + unix_linux.rs / unix_darwin.rs │   + windows/ffi.rs, windows/io.rs
                                   │ sys/stub.rs (neither: no console at all)
```

The dispatch in `src/sys/mod.rs` is exhaustive by construction — `cfg(unix)`,
`cfg(windows)`, `cfg(not(any(unix, windows)))` — so the stub can never be
selected on a platform that has a real backend. `stub.rs` is kept, not deleted:
`wasm32`, `redox` and friends are neither unix nor windows, and it is what keeps
`cargo build` honest for them (no console, straight to the dump path).

Verify the seam holds at any time:

```sh
grep -rn 'unsafe' src --include='*.rs' | grep -v '^src/sys/'   # must be empty
grep -rn 'target_os\|cfg(windows)' src --include='*.rs' | grep -v '^src/sys/'
# ^ only src/plat/mod.rs, which turns the cfg into a Platform value once
```

## The contract

`src/sys/mod.rs` is the authoritative copy. The Windows backend provides exactly
those names with those signatures:

| Item | On Windows |
| --- | --- |
| `Fd` (`i32`) | 0/1/2 keep their unix meaning; 3.. index a four-slot table of `(CONIN$, CONOUT$)` `HANDLE` pairs in `windows/ffi.rs`. A `HANDLE` does not fit in an `i32`, so it is never cast into one. |
| `SavedTermios` | both console modes, both code pages, and the two handles they belong to — `restore` must work after the handle table has been torn down. |
| `install_signal_handlers()` | `SetConsoleCtrlHandler`, idempotent. |
| `is_tty(Fd)` | `GetConsoleMode` succeeding. Stronger than `GetFileType == FILE_TYPE_CHAR`, which is also true of `NUL` and of a serial port. |
| `open_tty()` | `CreateFileW("CONIN$")` + `CreateFileW("CONOUT$")`, read+write, shared. |
| `tty_fd()` | stdin when it is a console, else `open_tty()` with "you own this". |
| `close_fd(Fd)` | closes both handles of a slot; ignores 0/1/2. |
| `winsize()` / `winsize_of(Fd)` | `GetConsoleScreenBufferInfo`, `srWindow`. |
| `set_raw(Fd)` / `restore(..)` | `SetConsoleMode` on both handles + `SetConsoleCP`/`SetConsoleOutputCP`. |
| `read_input(..)` | `WaitForSingleObject(100ms)` + peek + `ReadFile`. |
| `write_all(..)` | `WriteFile`, looping over short writes. |
| `vt_output_supported()` | false until `set_raw` has enabled VT output. Every other platform gets a constant `true` from `sys/mod.rs`. |

`read_byte` appeared in an earlier draft of this table and does not exist: the
decoder reads into a buffer.

## What the backend does

### 1. Console mode — `src/sys/windows/abi.rs`

Raw mode is `SetConsoleMode` on both handles, after `GetConsoleMode` saves the
exact original values. The transformation is two `const fn`s, so it is arithmetic
the Linux host both asserts and tests.

`raw_input_mode(cur)` clears `ENABLE_LINE_INPUT` (0x0002), `ENABLE_ECHO_INPUT`
(0x0004), `ENABLE_PROCESSED_INPUT` (0x0001 — Ctrl-C must arrive as `\x03`, the
`ISIG` clear on unix) and `ENABLE_MOUSE_INPUT` (0x0010); sets
`ENABLE_VIRTUAL_TERMINAL_INPUT` (0x0200); and leaves every other bit exactly as
found.

`raw_output_mode(cur)` sets `ENABLE_VIRTUAL_TERMINAL_PROCESSING` (0x0004),
`ENABLE_PROCESSED_OUTPUT` (0x0001) and `DISABLE_NEWLINE_AUTO_RETURN` (0x0008),
and clears `ENABLE_WRAP_AT_EOL_OUTPUT` (0x0002) so painting the last cell of a
row does not scroll — together, clearing `OPOST`.

`ENABLE_VIRTUAL_TERMINAL_INPUT` is the load-bearing flag: it makes the console
deliver arrows, Home/End, function keys and bracketed paste as the same ANSI
escape sequences `src/key/` already decodes, so the decoder has no Windows
branch and stays fully host-tested.

If `ENABLE_VIRTUAL_TERMINAL_PROCESSING` cannot be set (a pre-1703 conhost),
`set_raw` puts back the input mode and code pages it already changed and returns
`None`. `main.rs` treats that — like `NoTty` — as "nothing interactive here" and
dumps the document instead of painting escapes the console would print
literally.

### 2. Mouse and quick edit — the product requirement

`ENABLE_MOUSE_INPUT` is never set and `ENABLE_QUICK_EDIT_MODE` is never cleared.
Quick edit *is* how console users drag-select, so taking it away is the Windows
spelling of emitting `?1000h` (SPEC.md §"Hard constraints" #5), and enabling
mouse input turns it off as a side effect.

The trap is `ENABLE_EXTENDED_FLAGS` (0x0080): quick edit is only honoured while
it is set, and a `SetConsoleMode` that sets extended flags *without* quick edit
silently disables selection. `raw_input_mode` therefore re-asserts **both** bits
whenever the incoming mode reported quick edit, rather than trusting the
read-modify-write round trip — and never sets extended flags on its own, so a
user who had quick edit off keeps it off. Four `const _: () = assert!(…)` pins in
`windows/abi.rs` and five tests in `windows/abi_tests.rs` hold that in place on
every build, on every target.

No `?1000h`/`?1002h`/`?1003h`/`?1006h` appears anywhere in the crate. The one
escape sequence the backend itself emits is the control-handler teardown
(§4), and its bytes are asserted to contain none of them.

### 3. Input — `src/sys/windows/io.rs`

`read_input` must behave like `VMIN=0 / VTIME=1`: come back within ~100 ms even
in silence, because that return is the event loop's resize tick.

```
WaitForSingleObject(hIn, 100)  ->  WAIT_TIMEOUT   => ReadOutcome::Timeout
                                   WAIT_FAILED    => ReadOutcome::Error
PeekConsoleInputW(hIn, 32)     ->  any record that will become bytes?
                                     no  => discard the batch, Timeout
                                     yes => ReadFile
ReadFile(hIn, buf)             ->  UTF-8 bytes, because SetConsoleCP(CP_UTF8)
```

Plain `ReadFile`, not `ReadConsoleInputW`, is what produces bytes: with VT input
enabled the console does the record-to-escape-sequence translation itself, and
`SetConsoleCP(CP_UTF8)` makes the result UTF-8 rather than the OEM code page. So
there is no hand-rolled UTF-16 decoding and no surrogate-pair buffering — an
earlier draft of this document specified both, and the VT path deletes the whole
problem.

The peek exists because `WaitForSingleObject` signals for *any* input record
while `ReadFile` only returns once one of them translates to bytes: holding
Shift would otherwise block the loop and stall the resize tick. Key-ups, bare
modifiers, focus and menu events are classified as "no bytes" and consumed.
`key_record_yields_bytes` is a pure function, tested on Linux.

A successful zero-byte `ReadFile` on a **console** is `Timeout`, not `Eof` — a
record can be consumed and translate to nothing, and calling that EOF would quit
the pager because the user clicked another window. Off a console it is a real
EOF, which is what finally makes `ReadOutcome::Eof` reachable.

### 4. Exit paths — every one of them restores the console

| Exit | Path |
| --- | --- |
| `q` / normal quit | `event_loop` returns → `term.restore()` → `sys::restore` |
| Ctrl-C keystroke | arrives as `\x03` (processed input is off) → `key.rs` → quit → as above |
| Ctrl-C / Ctrl-Break out of band | `ctrl_handler` sets `INTR` → event loop quits → as above |
| Close / logoff / shutdown | `ctrl_handler` restores *inside the handler*: the process dies moments later, so the loop never gets a turn |
| Panic | `main`'s panic hook → `term::emergency_restore()` → `sys::restore` (release is `panic = "abort"`, so `Drop` never runs) |
| `Term` dropped | `Drop` → `restore`, idempotent |

`restore` works from the handles inside `SavedTermios`, never from an `Fd`, so it
is correct after the handle table has been emptied; it allocates nothing and
cannot panic. The control handler is the one place the backend emits an escape
sequence — `\x1b[?1049l\x1b[?25h\x1b[0m`, to leave the alternate screen, show the
cursor and reset SGR — because calling back up into `term.rs`, which allocates
and takes a mutex, from an injected handler thread is not worth the deadlock.
Those bytes live in `windows/abi.rs` so the Linux host can assert what is *not*
in them. They are written only when the output handle is a console the backend
configured, never into a redirected file.

### 5. Size — `GetConsoleScreenBufferInfo`

```
cols = srWindow.Right  - srWindow.Left + 1
rows = srWindow.Bottom - srWindow.Top  + 1
```

`srWindow`, **not** `dwSize`: `dwSize` is the scrollback buffer, routinely
thousands of rows tall, and laying frames out to it would paint most of the
pager where the user cannot see it. A failed call, a zero dimension or an
inverted rectangle is `None`, and `term.rs` substitutes 80x24.

There is no `SIGWINCH`. Resize is detected two ways, both feeding the same
portable `AtomicBool`: a `WINDOW_BUFFER_SIZE_EVENT` seen while peeking, and a
poll of `srWindow` on every `read_input` (at most once per 100 ms, one syscall).
The comparison — including "a failed query is not a resize" — is
`abi::size_changed`, tested on Linux.

### 6. `/dev/tty` equivalent

`open_tty()` is `CreateFileW("CONIN$" | "CONOUT$", GENERIC_READ | GENERIC_WRITE,
FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL)`. Opening
*both* is what lets `type x.md | tread` work with keys from `CONIN$`, and keeps a
writable handle when stdout is redirected too.

## What changed above `sys`

Not the terminal layer, the key decoder, the parser, the renderer or the pager:
none of them has a `cfg` and none of them needed one. Two *conventions* did move,
into `src/plat/`, as pure functions of an explicit `Platform` — so the Windows
rules are exercised by `cargo test` on Linux rather than by hope:

- **`plat::path`** — native path arithmetic. Volume prefixes (`C:`,
  `\\server\share`, `\\?\C:`), `\` as a separator, ASCII-case-insensitive
  comparison, and the `\dir` / `C:dir` shapes that are neither absolute nor
  plainly relative. `nav/` uses it for the join, the corpus-containment check and
  the status-bar relative path. A link destination stays URL-ish
  (`models/SAMPLE_MODEL.md`); only the native path it resolves to is Windows-shaped.
  The old `Component`-based fold dropped the volume prefix, which would have
  re-rooted every Windows link onto the current drive.
- **`plat::dirs`** — where files live. `%LOCALAPPDATA%\tread\last-yank.txt`, then
  `%TEMP%`, then `%USERPROFILE%\AppData\Local`; `$HOME` is never read on Windows,
  `%USERPROFILE%` is. `~`-shortening is a unix shell convention and is not
  applied to a Windows path a user could not paste back.

`main.rs` gained one thing: a terminal that cannot enter raw mode falls back to
the dump path instead of failing, which is what `sys/mod.rs` always documented
`set_raw`'s `None` to mean and what the pre-1703 conhost case depends on.

## Installing — `install.ps1`

The counterpart of `install.sh`, and the same contract: pick the build for the
machine, verify it against the release's `SHA256SUMS`, refuse to install
anything that does not match, install to a per-user location, clean up either
way. It targets **Windows PowerShell 5.1** — what ships with Windows, and what
most people will run — and **PowerShell 7**.

```powershell
irm https://raw.githubusercontent.com/viict/tread/master/install.ps1 | iex
```

Four things it does that the shell script does not have to:

- **`iex` runs the whole download as one expression**, so nothing may execute
  while it parses. Everything is inside `Install-Tread`, called on the last
  line: a transfer cut short is a parse error that does nothing at all. Nothing
  reads `$MyInvocation` or `$PSScriptRoot`, because there is no script on disk.
- **Architecture is a two-variable question.** `$env:PROCESSOR_ARCHITECTURE`
  describes the *process*: a 32-bit PowerShell on a 64-bit machine says `x86`,
  and an x64 PowerShell emulated on ARM64 says `AMD64`. `PROCESSOR_ARCHITEW6432`
  holds the machine's real architecture under WOW64 and is absent otherwise, so
  preferring it is right in every combination — including installing the native
  ARM64 build rather than the emulated x64 one.
- **A running `tread.exe` cannot be overwritten, but it can be renamed.** The
  new exe is staged beside it, any existing one is moved to `tread.exe.old-…`,
  and the staged file is moved into place — the same "write beside it and rename
  over" as `install.sh`. A running `tread` keeps working off the renamed file
  and the leftover is swept by the next install. If even the rename is refused,
  it says so instead of leaving a half-written exe.
- **PATH is reported, never written.** SPEC.md §"Installing on Windows" asks the
  installer to *report* how to add the directory to `PATH` when it is not
  already there, which is also all `install.sh` does — it prints an
  `export PATH=…` line. So `install.ps1` prints the one-line
  `[Environment]::SetEnvironmentVariable(… 'User')` command and changes nothing:
  a one-liner piped from the internet editing the persistent environment is more
  than was asked for, and it could not have been honest about it either. A
  registry write is invisible to every process already running unless
  `WM_SETTINGCHANGE` is broadcast, so the "open a new terminal" advice that used
  to follow it was false for any terminal launched from the Explorer session that
  was already up — the installer would have said it worked while `tread` stayed
  not-found. The check for "already there" still reads `HKCU:\Environment`
  unexpanded (through the registry provider, since
  `[Environment]::GetEnvironmentVariable` expands), so a stored `%USERPROFILE%`
  is compared as the directory it names rather than as literal text. `setx` is
  named in a comment as what not to reach for: it truncates at 1024 characters
  and expands what it writes.

Also: TLS 1.2 is or'ed into `ServicePointManager.SecurityProtocol` on Windows
PowerShell only (an unpatched box still defaults to SSL3 + TLS 1.0, and
github.com has required 1.2 for years); failure is a `throw` and never `exit`,
because `exit` inside `iex` would close the console the one-liner was typed
into.

## What is and is not verified

Verified, on every `cargo test` run on the Linux builder:

- **Type-checks** for `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu`.
  `extern "system"` is `stdcall` on `i686` and the C convention on `x86_64` /
  `aarch64`, which is what makes one source correct for both toolchains.
- **Constants**, re-derived from the SDK headers and pinned by tests:
  `STD_INPUT/OUTPUT/ERROR_HANDLE` = `(DWORD)-10/-11/-12`; `INVALID_HANDLE_VALUE`
  = `(HANDLE)-1`; the ten input and five output mode bits; `WAIT_OBJECT_0` /
  `WAIT_ABANDONED` (0x80) / `WAIT_TIMEOUT` (0x102) / `WAIT_FAILED`;
  `ERROR_INVALID_HANDLE` 6, `ERROR_HANDLE_EOF` 38, `ERROR_BROKEN_PIPE` 109,
  `ERROR_NO_DATA` 232, `ERROR_OPERATION_ABORTED` 995; `KEY_EVENT` 1,
  `MOUSE_EVENT` 2, `WINDOW_BUFFER_SIZE_EVENT` 4, `MENU_EVENT` 8, `FOCUS_EVENT`
  0x10; `CTRL_C` 0, `CTRL_BREAK` 1, `CTRL_CLOSE` 2, `CTRL_LOGOFF` 5,
  `CTRL_SHUTDOWN` 6; `CP_UTF8` 65001.
- **Struct layouts**, `const _: () = assert!(…)` on every target: `COORD` 4/2,
  `SMALL_RECT` 8/2, `CONSOLE_SCREEN_BUFFER_INFO` 22/2, `INPUT_RECORD` 20/4
  (`WORD` + 2 bytes of padding + a 16-byte union; `KEY_EVENT_RECORD` and
  `MOUSE_EVENT_RECORD` are both 16). The `KEY_EVENT_RECORD` field offsets are
  decoded by `u16::from_le_bytes` arithmetic, tested on Linux, legitimate because
  Windows is little-endian on every architecture it supports.
- **The opener's argument vector** — `src/sys/browser.rs`, declared on every
  target for exactly the `win_abi` reason. `argv(Desktop::Windows, url)` is
  pinned to the program `rundll32` with `url.dll,FileProtocolHandler` and then
  the URL, each as its own argument; a host test asserts no platform's vector
  contains `cmd`, `cmd.exe` or `start`, and that a URL carrying `&`, `|`, `^`,
  quotes and a newline survives as one untouched argument. `cmd /c start` is not
  used precisely because its quoting rules would make such a URL a command
  injection, and no command *string* is built anywhere on that path.
- **Behaviour that is arithmetic**: mode transformations, quick-edit
  preservation, `srWindow` geometry including saturation and inverted rectangles,
  resize comparison, read/write classification, record classification, control
  event mapping, teardown-sequence contents.

`install.ps1` is verified further than the backend is, because PowerShell 7
itself runs on Linux. Against a real `pwsh`, on the builder:

- It **parses**, via `[Parser]::ParseInput`, with no errors — and of its 278
  line-boundary prefixes, exactly one (the whole file) contains a top-level
  invocation. Truncation cannot run half of it.
- **PSScriptAnalyzer** reports no errors, and nothing at all outside
  `PSAvoidUsingWriteHost` (deliberate: this is UI, not pipeline output),
  `PSAvoidUsingEmptyCatchBlock` (three deliberate best-effort probes) and
  `PSUseShouldProcessForStateChangingFunctions`.
- Its functions are **host-tested** — 48 assertions covering the architecture
  matrix including both WOW64 shapes and x64-on-ARM64 emulation, `SHA256SUMS`
  parsing (two-space, binary-mode `*`, CRLF, absent, empty, near-miss name),
  mismatch refusal, download success and failure, the staged install including
  upgrade, leftover sweeping and failure leaving the old exe in place, and the
  PATH check including that an unexpanded `%VAR%` in the stored value is compared
  as the directory it expands to.
- It **installs, end to end, from the real GitHub release**: newest-version
  lookup, download, checksum, `Expand-Archive`, staged install — leaving a
  genuine `PE32+ … x86-64` (and, for ARM64, `PE32+ … Aarch64`) `tread.exe`. A
  tampered `SHA256SUMS` and a missing one were both served from a local
  server and both refused with nothing installed.

**Not** verified, and unverifiable from here:

- That any of the syscalls behave as documented — that `ReadFile` on a
  VT-input console really yields UTF-8 ANSI, that `WaitForSingleObject` really
  signals when it should, that `CONOUT$` opens under every shell.
- Linking. No mingw linker and no MSVC toolchain here; `cargo check` is as far as
  it goes for both Windows targets.
- That `rundll32 url.dll,FileProtocolHandler <url>` actually opens the default
  browser, that `rundll32` resolves on `PATH`, or that `CreateProcess` quotes the
  argument vector the way the Rust standard library documents. Only the vector
  itself is verified here; a failure to spawn is a status-bar message rather than
  an error either way (SPEC.md §"Opening a link outside the reader").
- Anything about a real console host's rendering: line wrapping at the last
  column, the code-page switch surviving, how conhost handles a wide CJK cell.
- Everything about `install.ps1` that is *Windows*. It has never run on
  Windows and never under **Windows PowerShell 5.1**, which is a different
  engine from the PowerShell 7 it was tested against — the .NET Framework
  behind it, its `Expand-Archive`, and the TLS branch that only 5.1 takes are
  all unexercised. Nor is: the registry write to `HKCU:\Environment`, that
  renaming a running `tread.exe` is permitted, `PROCESSOR_ARCHITEW6432` really
  holding what the documentation says, `Unblock-File`, or that the installed
  exe runs. On Linux only the shapes of these could be tested, by stubbing
  their collaborators.
- The soak harnesses. `tools/soak.sh` and `tools/soak_pty.py` are Linux-only and
  are run against the musl binary; the Windows equivalent wants a pseudoconsole
  (`CreatePseudoConsole`) driving the same key script and asserting the same
  invariants — clean exit, no panic, alternate screen exited, cursor restored, no
  mouse-tracking sequence, quick edit still on afterwards. That harness does not
  exist.
