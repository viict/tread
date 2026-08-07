---
status: Active
updated: 2026-08-06
related:
  - layout.md
  - lenses.md
  - testing.md
  - windows.md
---

# tread docs

Everything past the front page. The [root README](../README.md) covers
installing and using `tread`; this covers how it is built and how it is proven.

This folder is itself a corpus — `tread --index docs` walks it, `i` lists these
pages grouped by the section they appear under, and `]` / `[` step between them.
The two links under "Elsewhere" climb above that root, so `tread` refuses to
follow them from here; run `tread --index .` from the repository root to walk
the whole project instead.

## Contributing

| Doc | What it covers |
| --- | --- |
| [layout.md](layout.md) — module map | where every module lives and the one rule it obeys |
| [lenses.md](lenses.md) — `--lens` | the seam, the `agent` dialect field by field, and what a new dialect must provide |
| [testing.md](testing.md) — the suite | unit, golden and integration layers, plus the two soak harnesses |
| [../install.sh](../install.sh) — the installer | what `curl \| sh` runs: platform detection, checksum verification, atomic install |

## Platforms

| Doc | What it covers |
| --- | --- |
| [windows.md](windows.md) — console backend | what the Windows backend does, and precisely what about it is verified |

## Elsewhere

| Doc | What it covers |
| --- | --- |
| [../SPEC.md](../SPEC.md) — the contract | binding behaviour: rendering rules, keys, the `Source` seam, CSV |
| [../CLAUDE.md](../CLAUDE.md) — agent guide | non-negotiables and build commands for anyone automating on this repo |
