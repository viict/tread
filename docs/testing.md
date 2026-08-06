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
