# Layout

Where everything lives, and the rule each module obeys.

| Path | Role |
| --- | --- |
| [`src/main.rs`](../src/main.rs) | entry, wiring, panic guard, exit codes |
| [`src/cli.rs`](../src/cli.rs) | argument parser, `--help`/`--version` |
| [`src/sys/`](../src/sys/) | the platform seam and its backends — the only `unsafe` in the tree |
| [`src/plat/`](../src/plat/) | per-OS conventions above `sys`: path syntax, file locations |
| [`src/term/`](../src/term/) | raw-mode guard, alt screen, ANSI, frame buffer, OSC 52 |
| [`src/key/`](../src/key/) | byte stream to `Key` decoder |
| [`src/md/`](../src/md/) | the markdown parser: block, inline, table, list, AST |
| [`src/csv/`](../src/csv/) | the CSV foundation: RFC 4180 parser, lazy row index, windowed reads |
| [`src/json/`](../src/json/) | the JSON foundation: RFC 8259 parser, value tree, serialiser — iterative, no recursion on nesting |
| [`src/lens/`](../src/lens/) | the `--lens` seam: `Lens`, the registry, and one module per dialect (`agent`) — a transform over records, never a format |
| [`src/source/`](../src/source/) | the format seam: `Source`, the markdown, CSV, JSON and record sources, detection |
| [`src/render/`](../src/render/) | AST + width to styled wrapped lines |
| [`src/theme.rs`](../src/theme.rs) | palette, heading styles, banner glyphs |
| [`src/pager/`](../src/pager/) | viewport, scrolling, collapse tree, search, keymap |
| [`src/nav/`](../src/nav/) | document stack, index parsing, link resolution; `external.rs` holds the scheme allowlist for links that leave the reader |
| [`src/select/`](../src/select/) | visual selection, yank text, clipboard |
| [`src/open.rs`](../src/open.rs) | resolving the input and building the source behind it; `open/lens.rs` pairs a `--lens` with a format |
| [`src/dump.rs`](../src/dump.rs) | non-interactive render for pipes and `--no-alt` |

Every file is under 500 lines and every function under 50; modules split rather
than grow. Adding an OS means a new backend beside the others and one arm in the
dispatch — nothing above `src/sys/` is platform-specific.
