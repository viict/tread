---
status: Active
updated: 2026-08-06
related:
  - layout.md
  - testing.md
---

# Lenses

`--lens <name>` is a **semantic view over a record file** (SPEC.md §Lenses).
Without it a `.jsonl` renders as the generic JSON tree, one collapsed record per
row. With it, records that a dialect recognises get a summary line a person can
scan, and consecutive mechanical records fold into one row that opens.

```sh
tread --lens agent ~/.claude/projects/<slug>/<session>.jsonl
tread --lens list          # what there is; exits 2
```

```text
▾ user       21:28   I want to create a reader for the terminal…
▾            21:29   ⟨16 steps · 4 tool calls⟩
▾ assistant  21:29   The codex is 106 markdown files with a README index…
▾            21:31   ⟨15 steps · 3 tool calls⟩
▾ assistant  21:32   Scaffold and contract are in place. Now the workflow…
```

Two rules the seam is built around, and neither is negotiable:

* **A lens only ever adds interpretation.** A record it does not recognise
  renders exactly as it would with no lens: the generic collapsed-record row,
  openable into the whole record. Nothing is hidden and nothing is dropped.
* **Every row still opens into the raw record.** A summary is a headline.
  `Enter` / `za` on a message row shows the record as a tree; on a folded run it
  shows the records inside it, each of which opens in turn. `Y` on a run copies
  every record it holds, as JSON.

## Keys, under a lens

| Key | What it does |
| --- | --- |
| `Enter` / `za` | open the run under the cursor, or the record's raw tree |
| `zR` / `zM` | open the runs the viewport has reached / shut every run |
| `Tab` / `S-Tab` | next / previous **item** — a message or a run, skipping what a run folded |
| `/` `n` `N` | search the record source text; a hit inside a folded run **opens that run** |
| `y` | the value under the cursor · `Y` the record (or the whole run) · `c` the source line verbatim |

The status bar reads `agent · record 412/2354 · .message.content[0].text`.
`--toc --lens agent` prints the same reading as a list — `line, actor, time,
what`, one record per line, tab-separated — with the records the lens does not
recognise keeping their generic summary.

## The `agent` dialect

Claude Code session logs: `~/.claude/projects/<project-slug>/<session-uuid>.jsonl`,
one JSON object per line, in wall-clock order.

**Top-level keys.** `type` names the record. `timestamp` is ISO-8601 UTC
(`2026-08-05T21:28:58.659Z`); the row shows `HH:MM` of it, as recorded — there
is no timezone database in a zero-dependency binary, and the date is the same
for nearly every row of one session. `uuid` / `parentUuid` chain the records,
`sessionId`, `cwd`, `gitBranch` and `version` describe the run, and
**`isSidechain: true`** marks a subagent's own conversation (a `Task` run),
which the row shows as `↳ assistant`.

**Record types seen in a real 2354-record session** (which also contained one
truncated line — two records run together — that renders as an error row naming
the line, and does not stop the file):

| `type` | Rows as | Notes |
| --- | --- | --- |
| `user` | message, or step | `message.content` is a string for a typed prompt, or a block array |
| `assistant` | message, or step | `message.content` a block array; `message.model` names the model |
| `system` | step | `subtype` (`turn_duration`, …), `durationMs` |
| `mode`, `permission-mode`, `last-prompt`, `ai-title`, `bridge-session`, `attachment`, `queue-operation`, `file-history-snapshot`, `file-history-delta`, `summary` | step | the transcript's own bookkeeping |

**Content blocks**, in `message.content`:

| Block | Rows as |
| --- | --- |
| `text` | the record becomes a **message** — the only thing that does |
| `thinking` | step: `thinking`, `thinking ×3` |
| `tool_use` (`id`, `name`, `input`) | step: `Bash(cargo test)` — the argument named by `command`, `file_path`, `pattern`, `query`, `url`, `subagent_type`, … in that order |
| `tool_result` (`tool_use_id`, `content`, `is_error`) | step: `Bash → 42 lines`, `Edit → error` — named after the call it answers, which the lens remembers |

The shape worth knowing: **a tool result arrives as a `user` record**, because
the API calls it a user turn. That is precisely why the generic tree is close to
useless on a trajectory — half the "user messages" are machine output — and why
a `user` record whose only content is a `tool_result` is mechanics here.

## Adding a dialect

opencode and OpenAI Codex logs are wanted and **not implemented**. Adding one is
a module and a line, and nothing else:

1. **A module under `src/lens/`** with a struct implementing `lens::Lens`:
   * `name()` — the `--lens` word.
   * `about()` — one line for `--lens list`.
   * `read(&mut self, &Value) -> Option<Summary>` — called **once per record, in
     file order**, so a dialect may carry state (the agent lens keeps a bounded
     ring of `tool_use` ids so a result can name its call). Return `None` for
     anything it does not recognise; that record then renders generically.
2. **One entry in `lens::LENSES`** — `(NAME, || Box::new(Mine::default()))`.
3. **Tests beside it** (`src/lens/<name>_tests.rs`), with **hand-written
   fixtures**. Real session logs are private; read one to learn the shape, never
   copy it into the repository.

A `Summary` is five fields and no styling:

| Field | Meaning |
| --- | --- |
| `class` | `Message` (never folded away) or `Step` (folds into a run with its neighbours) |
| `who` | `User` / `Assistant` / `Tool` / `System` — colour only |
| `actor` | the text in the speaker column: `user`, `tool`, `↳ assistant` |
| `time` | `HH:MM`, or `None`; `lens::clock` does ISO-8601 |
| `what` | one line: `lens::excerpt` collapses whitespace and cuts to width |
| `calls` | tool calls in this record, for a run's `· 4 tool calls` |

What a dialect **never** touches: rows, folding, row arithmetic, search,
yanking, the outline or the status bar. Grouping is
`src/source/jsonl/plan.rs`, painting is `src/source/jsonl/lensrow.rs`, and both
are dialect-agnostic. If a new dialect needs a change there, that change is
about *all* lenses and belongs in the seam.

## What a lens costs

Classifying a record parses it, so the plan is built **as a prefix, extended
ahead of the viewport** — the same discipline as the lazy line index. Rows above
the viewport never move; rows below shift as grouping catches up, exactly as
`len()` grows while the index scans. `G` waits for the lens the way it waits for
the index, reporting progress rather than jumping to a wrong end.

Measured on a real 4.0 MB, 2354-record session (release build, 140 columns):
open plus the first screen **25–30 ms**, reading the whole file through the
lens ~65 ms, painting the last screen under 1 ms. 2354 of 2354 records
reachable, folded into 633 rows.
