# Porting `mdr` to Windows

This is a specification for a backend that does not exist yet. Nothing here is
implemented. It is written down so the seam stays honest: if a change above
`src/sys/` would break this document, the change is in the wrong place.

## The seam

Every platform call in the crate lives under `src/sys/`, and the backend
modules there are the only ones containing `unsafe`. Everything above — `term`,
`key`, the parser, the renderer, the pager, `nav`, `select` — is portable safe
Rust that talks to the platform exclusively through the names below.

```
main.rs / term/ / key/            safe, portable, no #[cfg(windows)] anywhere
──────────────────────────────────────────────────────────────────────────────
sys/mod.rs     public surface + backend dispatch        (no unsafe)
sys/abi.rs     pure ABI arithmetic, host-tested         (no unsafe)
sys/layout.rs  pure C struct layouts, host-tested       (no unsafe)
──────────────────────────────────────────────────────────────────────────────
sys/unix.rs (termios + ioctl)      │ sys/windows.rs (Console API)   <- to write
  + unix_linux.rs / unix_darwin.rs │ sys/stub.rs  (today's fallback)
```

Verify the seam holds at any time:

```sh
grep -rn 'unsafe' src --include='*.rs' | grep -v '^src/sys/'   # must be empty
grep -rn 'target_os\|cfg(windows)' src --include='*.rs' | grep -v '^src/sys/'
```

## The contract a backend must satisfy

`src/sys/mod.rs` documents and exports exactly this surface, and is the
authoritative copy of the contract. A Windows backend must provide the same
names with the same signatures and the same semantics; nothing else changes.
Anything it can compute without calling the OS belongs in `src/sys/abi.rs`
(constants and arithmetic) or `src/sys/layout.rs` (C struct layouts, declared
for every OS regardless of target and asserted with `const _: () = assert!(…)`
so a wrong one fails the build on the Linux host too). The unix backend is the
worked example: it serves both Linux and Darwin, and the only per-OS files are
the two-item `unix_linux.rs` / `unix_darwin.rs`.

| Item | Signature | Meaning |
| --- | --- | --- |
| `Fd` | `type Fd = i32` | Opaque handle. On Windows this becomes a small index into a table of `HANDLE`s, or `HANDLE as i32` — callers never inspect it. |
| `STDIN` / `STDOUT` | `const Fd` | The two standard handles. |
| `SavedTermios` | opaque `Copy` struct | Whatever must be restored on exit. On Windows: the two saved console mode `DWORD`s. |
| `ReadOutcome` | `Bytes(usize) \| Timeout \| Eof \| Error(i32)` | Result of one input read. `Timeout` is required: the event loop uses it as its tick. |
| `install_signal_handlers()` | `fn()` | Arrange for `winch_pending()` / `interrupt_pending()` to become true. |
| `winch_pending()` | `fn() -> bool` | Resize seen since the last call; clears the flag. Already implemented portably over an `AtomicBool` in `sys/mod.rs`, shared by every backend. |
| `interrupt_pending()` | `fn() -> bool` | Ctrl-C seen since the last call; clears the flag. |
| `is_tty(Fd)` | `fn(Fd) -> bool` | Handle refers to a console. |
| `open_tty()` | `fn() -> Option<Fd>` | A read/write handle to the controlling terminal even when stdin is a pipe. |
| `tty_fd()` | `fn() -> Option<(Fd, bool)>` | Handle to read keys from; the `bool` is "caller owns it and must close". |
| `close_fd(Fd)` | `fn(Fd)` | Close a handle from `open_tty`. |
| `winsize()` / `winsize_of(Fd)` | `fn(..) -> Option<(u16, u16)>` | Terminal size as `(cols, rows)`. |
| `set_raw(Fd)` | `fn(Fd) -> Option<SavedTermios>` | Enter raw mode; return the previous state. |
| `restore(Fd, &SavedTermios)` | `fn(..) -> bool` | Put the terminal back exactly as found. |
| `read_input(Fd, &mut [u8])` | `fn(..) -> ReadOutcome` | Up to `buf.len()` bytes of **UTF-8 encoded terminal input**, retrying on interruption. Must return `Timeout` after roughly 100 ms of silence. |
| `read_byte(Fd)` | `fn(Fd) -> ReadOutcome` | One byte, same semantics. |
| `write_all(Fd, &[u8])` | `fn(..) -> Result<(), i32>` | Write the whole buffer, looping over short writes. `Err` carries the platform error code. |

## What the Windows implementation has to do

### 1. Console mode

Raw mode is `SetConsoleMode` on both handles, saved first with
`GetConsoleMode` so `restore` can put the exact original values back.

Input handle (`STD_INPUT_HANDLE`) — clear:

| Flag | Why |
| --- | --- |
| `ENABLE_LINE_INPUT` (0x0002) | keys must arrive unbuffered, not per line |
| `ENABLE_ECHO_INPUT` (0x0004) | the pager draws its own screen |
| `ENABLE_PROCESSED_INPUT` (0x0001) | deliver Ctrl-C as a key, matching the `ISIG` clear on Linux |
| `ENABLE_MOUSE_INPUT` (0x0010) | **must stay off** — see §Mouse |
| `ENABLE_QUICK_EDIT_MODE` (0x0040) | **must stay on**; this is what preserves native drag-select |

Input handle — set `ENABLE_VIRTUAL_TERMINAL_INPUT` (0x0200). That makes the
console deliver arrow keys, Home/End, function keys and bracketed paste as the
same ANSI escape sequences `src/key.rs` already decodes, so `key.rs` needs no
Windows branch at all. This is the single most important flag in this document.

Output handle (`STD_OUTPUT_HANDLE`) — set:

| Flag | Why |
| --- | --- |
| `ENABLE_VIRTUAL_TERMINAL_PROCESSING` (0x0004) | the frame buffer emits ANSI SGR/CUP; the console must interpret them |
| `ENABLE_PROCESSED_OUTPUT` (0x0001) | keep |

and clear `ENABLE_WRAP_AT_EOL_OUTPUT` (0x0002) so a full-width row does not
scroll the screen, which is the equivalent of clearing `OPOST` on Linux.

If `ENABLE_VIRTUAL_TERMINAL_PROCESSING` cannot be set (pre-1703 console),
`set_raw` should return `None`; `main.rs` already treats that as "no tty" and
falls back to the non-interactive dump path.

### 2. Input: `ReadConsoleInput`

`read_input` must behave like a `VMIN=0 / VTIME=1` read: return promptly with
zero bytes when nothing arrives, because the event loop uses `Timeout` to poll
`winch_pending()`.

```
WaitForSingleObject(hIn, 100)          -> WAIT_TIMEOUT  => ReadOutcome::Timeout
ReadConsoleInputW(hIn, records, n)
  for each record:
    KEY_EVENT (bKeyDown only)          -> UTF-16 UnicodeChar; encode as UTF-8
                                          into the caller's buffer. With
                                          ENABLE_VIRTUAL_TERMINAL_INPUT set,
                                          navigation keys already arrive as
                                          ESC-sequence characters.
    WINDOW_BUFFER_SIZE_EVENT           -> WINCH.store(true); do not emit bytes
    MOUSE_EVENT / FOCUS / MENU         -> ignore entirely
```

Surrogate pairs must be buffered across records before being encoded, or a
non-BMP character will be split. `key.rs` assumes well-formed UTF-8 on the way
in and will otherwise wait for continuation bytes that never come.

Ctrl-C: with `ENABLE_PROCESSED_INPUT` cleared it arrives as a normal key event
(`\x03`) and `key.rs` decodes it. Also install a `SetConsoleCtrlHandler` that
sets the interrupt flag, so a Ctrl-C delivered out of band still exits cleanly.

### 3. Size: `GetConsoleScreenBufferInfo`

`winsize_of` is `GetConsoleScreenBufferInfo(h, &csbi)` and then

```
cols = csbi.srWindow.Right  - csbi.srWindow.Left + 1
rows = csbi.srWindow.Bottom - csbi.srWindow.Top  + 1
```

Use `srWindow`, **not** `dwSize`: `dwSize` is the scrollback buffer, which is
usually far taller than the visible window and would make the pager lay out
frames the user cannot see. Return `None` on failure or on a zero dimension —
`term.rs` already substitutes 80x24.

There is no `SIGWINCH`. The resize signal is the
`WINDOW_BUFFER_SIZE_EVENT` record above, which is why the resize flag is a
plain atomic rather than anything signal-shaped.

### 4. `/dev/tty` equivalent

`open_tty()` becomes `CreateFileW("CONIN$", GENERIC_READ | GENERIC_WRITE,
FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL)`. This is what
makes `type x.md | mdr` work: stdin is a pipe, so keys come from `CONIN$`
instead. `is_tty` is `GetConsoleMode(h, &mode) != 0`.

### 5. Mouse

Do not enable `ENABLE_MOUSE_INPUT`, and do not emit `?1000h`, `?1002h`,
`?1003h` or `?1006h`. Leave `ENABLE_QUICK_EDIT_MODE` set. This is a product
requirement (SPEC.md §Hard constraints #5), not a detail: the reader must never
take the mouse away from the terminal's own click-drag selection. Both soak
harnesses (`tools/soak.sh`, `tools/soak_pty.py`) fail the build if any of those
sequences ever appear in the output stream.

## What changes above `sys`

Nothing. Concretely, the dispatch at the bottom of `src/sys/mod.rs` gains one
arm:

```rust
#[cfg(windows)]
#[path = "windows.rs"]
mod backend;
```

and the `#[cfg(not(unix))]` stub arm narrows to `not(any(unix, windows))` — or
goes away entirely once every supported target has a real backend. `main.rs`
does not change at all.

Everything else is already portable and must stay that way:

- `term.rs` writes ANSI only, and every sequence it emits is supported by the
  VT-processing console.
- `key.rs` decodes an ANSI byte stream, which is what
  `ENABLE_VIRTUAL_TERMINAL_INPUT` produces.
- `nav.rs` uses `std::path`, so drive letters and `\` separators work already;
  link resolution is `Path::join` plus a root-escape check, not string surgery.
- `select/clip.rs` writes OSC 52, which Windows Terminal supports; the
  `~/.cache/mdr/last-yank.txt` fallback uses `std::env::var_os("HOME")` and
  should gain a `USERPROFILE` fallback — that is the one genuine change outside
  `sys`, and it is a two-line `or_else`.

## Testing a backend

`cargo test` is platform-independent: the whole parser, renderer and pager
suite runs anywhere, because none of it touches `sys`. What needs a real
console is the raw-mode round trip. The Linux equivalent lives in
`tools/soak_pty.py`; a Windows version wants a pseudoconsole
(`CreatePseudoConsole`) driving the same key script and asserting the same
invariants: clean exit, no panic, alternate screen exited, cursor restored, no
mouse-tracking sequence, no stray escape.
