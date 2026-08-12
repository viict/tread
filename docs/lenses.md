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
  `Enter` / `za` on a message row shows what was said in full; `zt` shows the
  record itself as a tree, whatever the message is doing. On a folded run,
  `Enter` shows the records inside it, each of which opens in turn. `Y` on a run
  copies every record it holds, as JSON.

## The message under a row

A summary row is one line, and for a message **that line is the first line of
what was said**. The rest of it is under the row, wrapped to the view width and
indented to the `what` column:

```text
▾ assistant  10:55   Reading the failing test first. The suite names a
                     fixture that no longer exists, so that is where this
                     starts.
                     ⋯ +37 lines
```

It is one wrap, split in two: row 1 is the summary row's `what` column, rows
2..N are the body. The message's opening words are therefore on the screen
**once** — the row is not an excerpt with the same words repeated under it. A
message that fits on the summary row has no rows under it at all. A message that
opens with blank lines starts on its first line with words on it, rather than
spending the headline and the rows under it on the newlines someone happened to
type first.

`Summary::what` is unchanged by this: it is still the one-line excerpt, and it
is what `--toc` prints, which is the right answer for a list rather than for a
screen.

Two states, and no third: **clipped** to six rows in all — the summary row and
five under it — whose last row says what it
is not showing — in the message's own lines, or in bytes when what is left is
the tail of one long line, counted in the message's own bytes — and **whole**,
which `Enter` / `za` toggles. A message that fits the clip has only one state,
and `Enter` there falls through to the record's own fold instead of consuming
the key for a repaint of the same rows. `zR`
opens every body the viewport has reached, `zM` clips them all again, and a
batch (`--plain`, `--toc`) is `zR`: a pipe gets the whole message, never a
viewport's clip of it.

A step has no body. Mechanics stay one line, which is also what keeps a folded
run exactly as tall as the records inside it.

**What this costs.** A block's rows now depend on the **width**, so a resize
that changes it re-lays every body — without reclassifying anything: a record is
read once, in file order, and only the wrap is redone. It is also why a `Mark`
into a record document **read through a lens** is the *record* rather than the
row, so the cursor comes back to what it was reading; the offset inside a record
does not survive, and a cursor on line four of a message lands on that message's
own row. With no lens nothing wraps, so a mark stays the row it always was and
the cursor keeps its place inside an open record.

## Keys, under a lens

A trajectory read through a lens is a list of **blocks** — a message with what
was said under it, or a folded run of mechanics — and moving through it one
terminal row at a time is moving through one message's wrap. So the cursor moves
between blocks, and a block is exactly what `Tab` always stepped between: there
is one definition of where a block starts. (`src/source/record/plan.rs` calls a
block an `Item`, which is the same run of records under an older name; *block*
is the only word above that module and the only one a reader ever sees.)

| Key | What it does |
| --- | --- |
| `j` / `k` | next / previous **block** — a message or a run, skipping what a run folded |
| `Ctrl-E` / `Ctrl-Y` | scroll one row, so every row of a block is still reachable |
| `Enter` / `za` | open the run under the cursor, or the whole of the message |
| `zt` | open the raw record under the cursor, whatever its message is doing |
| `zR` / `zM` | open the runs and messages the viewport has reached / shut every one |
| `Tab` / `S-Tab` | next / previous **message** — the conversation turn |
| `/` `n` `N` | search the record source text; a hit inside a folded run **opens that run** |
| `y` | the value under the cursor · `Y` the record (or the whole run) · `c` the record's own source text verbatim |

Landing on a block shows the block: one that fits is scrolled fully on screen,
one taller than the viewport puts its first row at the top. `d` / `u`,
`space` / `b` and `g` / `G` still count screens and rows, and `v` then `j`
therefore extends a visual selection a **block** at a time — `Ctrl-E` is how a
selection is grown a row at a time.

Two things `j` deliberately does not do. An **open** run is one block, so `j`
steps over the run rather than into the steps it just revealed; `Ctrl-E` walks
them. And where there is no next block it moves one row rather than freezing:
past the classified prefix — the tail of a big trajectory, where grouping is not
decided yet — and on the **last** block, whose rows are the end of the file. The
last block is therefore the one place the pair is not symmetric: `j` walks down
through it a row at a time while the status bar keeps reading `block N/N`, and
`k` from anywhere inside it comes back to its first row in one press.

`Ctrl-E` / `Ctrl-Y` scroll: the window moves and the cursor rides with it,
keeping its place on the screen. That is what makes them the way through a
block taller than the viewport — `j` puts the cursor at the top of the window
there, and a key that only moved the cursor would show a frozen screen until it
had crossed the whole window. At the ends of the document the window stops and
the cursor keeps going, so the last row is still reachable a row at a time.

`Tab` counts a record the dialect did **not** recognise as a message: it is not
mechanics, and it is the record SPEC.md §Lenses promises is never lost. That is
also what keeps `Tab` moving through a file no dialect reads, where every block
is one of those. With no further message — a trailing run of mechanics — `Tab`
falls back to the next block rather than dead-ending, and never moves past the
end.

The status bar reads
`agent · record 412/2354 · block 96/≥181 · .message.content[0].text`. The block
clause is lens-only, and it carries `≥` for the same reason the record count
does and then one more: the lens has read a prefix, and grouping makes the block
total *shrink* as classification catches up. Past that prefix there is no block
clause at all, rather than a number the next keystroke changes. The record count
keeps saying **record** — it is the file's own unit, which `#n` and `--toc`
also mean, and a `.jsonl` line is not a step.

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
▾ user               make the parser handle empty input rather than
                     raising, and say so in the docstring.
▾ assistant  10:55   Reading the failing test first. The suite names a
                     fixture that no longer exists.
▾ tool       10:55   thinking · bash(pytest -q tests/) → 32 lines
  ▸ ⟨4 steps · 6 tool calls⟩            10:56
```

A message row starts with the message's first line and continues under itself;
a step row has no body and is the one-line description of what it did.

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
| `message` says something | **message** — never folded away. It starts on its own summary row and continues under it. Its tool calls collapse to a count on `Summary::what` (`· 3 tool calls`) rather than becoming rows — which is what `--toc` prints; the painted row gives that column to what was said, because the message is what the reader came for |
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

Measured on a real 200KB, 49-step trajectory (`TREAD_ATIF_TRAJECTORY`, 140
columns): 50 records, 50 read by the lens, 73 rows shut and 1098 open — 36 rows
of headline, and the rest what was said under them, clipped. `--toc --lens atif`
prints the whole run as 50 tab-separated lines, headlines only.

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
| `what` | one line: `lens::excerpt` collapses whitespace and cuts to width. What `--toc` prints, and what the row paints for a record with no `body` |
| `calls` | tool calls in this record, for a run's `· 4 tool calls` |
| `body` | what was said, for the rows under the summary. `None` on a step |

A `Summary` is kept for **every** classified record, and nothing here may
allocate per document — so a `Body` is *not* the message. It is the first
`lens::BODY_KEEP` bytes of it, the whole message's byte and line counts, and the
path back to the text inside the record (`message`, or
`message.content[1].text`). The clip and the row arithmetic are answered from
the head alone, so a resize reads no file; opening a message longer than the
head reads it back out of the record that is being painted anyway. Holding whole
messages instead would make a long log's summaries as big as the log.


What a dialect **never** touches: rows, folding, row arithmetic, search,
yanking, the outline or the status bar — including how tall a message is, which
is `src/source/record/body.rs`'s answer and depends on the width. Grouping is
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

What a **body** costs is bounded per record and stated in the code: a `Summary`
keeps at most `lens::BODY_KEEP` (1 KB) of a message, whatever its size, plus the
path back to the rest. On a synthetic 4 MB / 8 MB ATIF trajectory that is
+1.6 MB / +3.2 MB of resident memory against the same file before bodies, with
open time unchanged (≈2 ms) — the cost tracks the number of *messages*, not
what was said in them. `zR` is the one keystroke that now scales with the file:
it shows every message the viewport has reached in full, which means wrapping
them (58 ms on 4 MB, 131 ms on 8 MB). A resize that *changes the width* wraps
too — the clip, which is six rows out of the head — one of them the summary row —
with no file read and no reclassification, or the whole of every message once `zR` has opened them, which
costs what `zR` costs. A resize that does not change the width — a height-only
one, which is most of what dragging a window's edge produces — costs nothing:
`RecordSource::remeasure` re-lays bodies only when `Plan::set_width` says the
width moved, and the pager calls `set_width` on every terminal size change.

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
