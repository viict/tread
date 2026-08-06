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
| [`src/source/`](../src/source/) | the format seam: `Source`, the markdown and CSV sources, detection |
| [`src/render/`](../src/render/) | AST + width to styled wrapped lines |
| [`src/theme.rs`](../src/theme.rs) | palette, heading styles, banner glyphs |
| [`src/pager/`](../src/pager/) | viewport, scrolling, collapse tree, search, keymap |
| [`src/nav/`](../src/nav/) | document stack, index parsing, link resolution |
| [`src/select/`](../src/select/) | visual selection, yank text, clipboard |
| [`src/dump.rs`](../src/dump.rs) | non-interactive render for pipes and `--no-alt` |

Every file is under 500 lines and every function under 50; modules split rather
than grow. Adding an OS means a new backend beside the others and one arm in the
dispatch — nothing above `src/sys/` is platform-specific.
