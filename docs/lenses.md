---
status: Active
updated: 2026-08-12
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
tread --lens atif  trajectory.json          # records inside a document
tread --lens list                           # what there is; exits 2
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
| `y` | the value under the cursor · `Y` the record (or the whole run) · `c` the record's own source text verbatim |

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

## The `atif` dialect

Agent trajectories in the ATIF interchange format (`ATIF-v1.7`, as written by
`opencode`). One JSON **document**, not a record per line: `schema_version`,
`session_id` and `agent` describe the run, and the records are the elements of
`steps`. That is all the lens says about the envelope —
`records_at() -> RecordsAt::Member("steps")` — and `src/source/jsonarray/` does
the rest, knowing nothing about ATIF.

```text
▾ session            ATIF-v1.7 · opencode 1.2.3 · vendor/model · sxs_…
▾ user               build the parser from the sources in /app…
▾ assistant  10:55   Configuring first, then building. · 2 tool calls
▾ tool       10:55   thinking · bash(autoreconf -i) → 32 lines
  ▸ ⟨4 steps · 6 tool calls⟩            10:56
```

**Record 0 is the session**: the document's top-level keys that are not `steps`,
as one row that opens into their tree. Nothing the document says is lost, and
the price is that record numbering is shifted by one — `steps[0]` is record 2,
which is what `record n/N` and `#n` mean.

**Top-level keys of a step.** `step_id` (an integer, and the recogniser),
`source` (`user` / `agent`), `timestamp` (ISO-8601 with a numeric `+00:00`
offset — the row shows `HH:MM`, as recorded), `message`, `reasoning_content`,
`tool_calls[]`, `observation.results[]`, plus `model_name`, `metrics` and
`llm_call_count`, which the summary leaves to the opened tree.

| A step whose… | Rows as |
| --- | --- |
| `message` says something | **message** — never folded away. Its tool calls collapse to a count on the row (`· 3 tool calls`) rather than becoming rows: the message is what the reader came for |
| `message` is empty | **step** — folds into a run with its neighbours |

A step row is what it did: `thinking`, then each call as
`bash(make -j8) → 42 lines`. The argument is named by `command`, `filePath`,
`pattern`, `query`, `url`, in that order — a `glob` carrying both `path` and
`pattern` shows the pattern — and a tool whose arguments name none of them
shows `todowrite()`. A result is matched to its call by `source_call_id`
**within the step**, which is where every answer sits; an orphan result is
counted (`· 1 result`) rather than dropped. Adjacent entries that read
identically collapse: `bash(pkg-config --exists onig) ×2`.

**The first step of a real trajectory carries `message`, `source` and `step_id`
and nothing else** — no timestamp. The row leaves the clock empty rather than
inventing one, and that is the first row on the first screen. Absent, `null` and
`[]` all mean the same thing, `arguments` is read as an object *or* as the
JSON-encoded string the wire format this schema descends from emits, and a step
this does not recognise keeps its generic row.

Measured on a real 200KB, 49-step trajectory (`TREAD_ATIF_TRAJECTORY`): 50
records, 50 read by the lens, 36 rows shut and 1089 open. `--toc --lens atif`
prints the whole run as 50 tab-separated lines.

## Adding a dialect

opencode's own logs and OpenAI Codex logs are wanted and **not implemented**.
Adding one is a module and a line, and nothing else:

1. **A module under `src/lens/`** with a struct implementing `lens::Lens`:
   * `name()` — the `--lens` word.
   * `about()` — one line for `--lens list`.
   * `records_at()` — where the records are: `RecordsAt::Lines` (the default, a
     `.jsonl`), `RecordsAt::Root` (a document whose root array is the records)
     or `RecordsAt::Member("steps")` (a document whose records are one of its
     keys, every *other* key becoming record 0). This is the only thing a
     dialect says about files, and the routing in `src/open/lens.rs` turns it
     into a format and a refusal.
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
`src/source/record/plan.rs`, painting is `src/source/record/lensrow.rs`, the
fold keys are `src/source/record/ops.rs`, and all three are dialect-agnostic —
and format-agnostic with it. There is one record source
(`src/source/record/source.rs`), and what a record *format* provides is the
`Store` trait: how many records the index has found, how to push it along
inside a byte budget, and record `i` as bytes or as a value.
`src/source/jsonl/` is a record per line over the CSV line index;
`src/source/jsonarray/` is an array inside a document over the JSON structural
index. Neither holds a row number. If a new dialect needs a change there, that
change is about *all* lenses and belongs in the seam.

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

A **document** lens pays one thing more, and SPEC.md §Lenses states it: the
records are one member of the document, and the structural index only knows a
member once it has walked past its last byte, so the first row waits on a byte
walk of the file. No record is parsed for it and nothing is held in memory —
the wait is a scan, reported as `≥N (indexing P%)` — but it is O(file) where a
record per line is O(screen). `--toc` and `--plain` are batches and wait for it:
they spend slice after slice until the index has what they asked for, because a
truncated list that exits 0 reads as an empty file.
