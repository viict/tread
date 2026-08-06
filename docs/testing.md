# Testing

How the suite is organised and what each layer is for.

```sh
cargo test
cargo clippy --all-targets
```

Unit tests live beside the code; [`tests/`](../tests/) drives the real binary with
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

## Screenshots

The pictures in the root README are generated from real output, not mocked up,
so they can be regenerated whenever the theme or the layout changes:

```sh
tools/screenshot.py target/x86_64-unknown-linux-musl/release/tread \
    path/to/doc.md docs/img/markdown.svg --cols 92 --rows 26 --keys za
```

It drives the binary through a pty, replays the escape sequences it paints into
a cell grid, and writes that grid as SVG — so what is in the README is what the
program printed.
