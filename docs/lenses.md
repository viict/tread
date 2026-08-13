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
  `Enter` / `za` on a record descends one level — what was said in full and its
  tool calls listed, and back again; `r` shows that record as a tree
  from any level. On a folded run, `Enter` shows the records inside it, each of
  which opens in turn. `Y` on a run copies every record it holds, as JSON.

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

## The two levels a record has, and the key that shows its bytes

`Enter` / `za` toggles the two (SPEC.md §Lenses); `r` is the record itself, from
either of them:

```text
clipped  <->  open        r: the raw JSON tree, over whichever level is showing
```

| Level | What is on the screen |
| --- | --- |
| **clipped** | the headline, and the text under it cut to six rows in all — whose last row says what it is not showing, in the text's own lines, or in bytes when what is left is the tail of one long line |
| **open** | the whole of that text, then one row per tool call: `▸ bash  cargo test -q  → 32 lines` |

A record has only the levels it has content for. A message the clip already
shows whole that made no calls has nothing between its headline and its JSON, so
it has no `Enter` rung at all and the key does nothing there rather than
repainting the same rows. It does **not** fall through to the record's own tree:
a record row claims `Enter` whether or not it has a rung, at either level, with
that tree open or shut — the absolute below has no exception for the records that
happen to have nothing to open. `r` is still the way to its bytes.

**`Enter` never opens a tree and never shuts one.** It used to: the tree was a
third rung, and getting from it back to the clip meant walking round the whole
ladder. Now `r` owns the tree and `Enter` owns the reading, so with a tree open
`Enter` leaves it alone and toggles the record's own rows *underneath* it — a
key that silently undid another key's work is the thing that was removed. Two
keys, two jobs, and each one is its own way back.

**A call row opens too.** `Enter` on one shows the arguments the call was made
with, one to a line, and then the output it returned under a row that **names**
it — clipped like a message body, with the same `⋯ +N lines` tail. One call at a
time, wherever the other one was; leaving the level shuts it.

```text
▾ assistant  10:55  Reading the failing test first, since the suite names a
                    fixture that no longer exists.
                    thinking
                      The fixture was renamed two commits ago.
                    ▾ bash      cargo test -q parse          → 32 lines
                        command   cargo test -q parse
                        timeout   120
                      output
                        test parse::empty_input … ok
                        ⋯ +26 lines
                    ▸ read      src/parse.rs                 → 40 lines
```

The `output` row is not decoration: without it, output whose lines happen to
read `key   value` sits at the argument-name column with nothing between it and
the arguments, and a reader sees three arguments two of which are output.

An argument's name is written **into** the pad its value's wrap left for it, so
the value's first row starts in the same column as its own continuation rows.
That is one column of arithmetic (`KEY_COL`, the name field *and* the space
after it) and getting it wrong pushed every argument's first row off the side of
the view while its wrapped remainder sat at the correct indent.

A call with **nothing** under it — no arguments and no answer — carries no
marker, and `Enter` there is the record's rather than the call's. It is the same
rule the rung above obeys: a key that repaints the same screen has not descended.
Everything under a member of an open run is inset with that member's own row, so
a step's reasoning and its calls line up with the step's words rather than
sitting two columns left of them.

`r` is orthogonal to all of it: it opens the record's own tree from either
level and leaves the level alone, which makes it the way to the bytes in one
press and the same press back. On a **call row** `r` is the row's *record*: a
call has no JSON of its own separate from the record it was made in, so that is
the honest raw thing under it. On a **run's row** there is no one record, so `r`
says "nothing to open here" rather than appearing to do nothing — `Enter` is the
key that opens a run.

`zR` puts every record the viewport has reached at the **open** level and opens
its tree; `zM` puts every one back to its clip and drops any call that was open.
A batch (`--plain`, `--toc`) is `zR`: a pipe gets what was said, what was
thought, what was called, and the whole record under it. `--toc` is unchanged —
it is a list, one line per record, and `Summary::what` is that line.

`zR` deliberately does **not** expand every call in the file. A run of steps
would become thousands of rows behind one keystroke, and every argument and
every output is already in the record's own tree, which `zR` opened.

## Reasoning is text, so it is shown

A step that only thought is one line — what it *did* — and the thought goes
under that line, clipped, muted. It appears wherever the step is, which, since
steps fold into runs, means as soon as the run is open.

That cost a change to the row arithmetic worth knowing about. A group's own rows
are still `1 + count`, and a member's own rows — its body and its parts — are a
**second prefix sum** beside the tree one (`Plan::extra`, a `RowMap` of exactly
the shape the trees use). So `own`, `inside`, `row_of_record` and
`blocks_of_item` keep the shape they had, and the invariant they rest on is the
one they always rested on: *a hidden record owns no rows*, which is why closing
a run closes its members' rows as well as their trees.

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
was said under it, or a folded run of mechanics. `j` and `k` do **not** move by
those: they move one visible row, here as in every other format
(SPEC.md §"Moving through a document"). A closed block is one row, so a block
cursor differed from a row cursor only where a block was open — over exactly the
message body, the opened run and the opened record the reader had just asked to
see. That is what it skipped, and it is why it is gone.

Blocks are what `Tab` / `S-Tab` jump between, and there is exactly one
definition of where one starts — `src/source/record/plan_block.rs`, which the
jump, the framing of a landing and the status bar's counter are all read off.
(`plan.rs` calls a run of records that share a row an `Item`. An item is one
block while it is shut, and its own row plus one block per step — `1 + count` —
while it is open, which is the whole of the descent rule; *block* is the only
word above that module and the only one a reader ever sees.)

| Key | What it does |
| --- | --- |
| `j` / `k` | next / previous **row** — a message's next line, a step, a row of an opened record |
| `Tab` / `S-Tab` | next / previous **block** — a message, a shut run, or a step inside a run that is open |
| `Enter` / `za` | the record's two levels: clipped ↔ open. On a run's row, open the run; on a call row, that call's arguments and output. Never opens or shuts a tree |
| `r` | show the raw record under the cursor, from either level, leaving the level where it was; on a call row, that row's record |
| `zR` / `zM` | every record the viewport has reached at the open level, with its tree / every one back to its clip |
| `/` `n` `N` | search the record source text; a hit inside a folded run **opens that run** |
| `y` | the value under the cursor · `Y` the record (or the whole run) · `c` the record's own source text verbatim |

Landing on a block with `Tab` shows the block: one that fits is scrolled fully
on screen, one taller than the viewport puts its first row at the top. `d` / `u`,
`space` / `b` and `g` / `G` still count screens and rows, and `v` then `j`
extends a visual selection a row at a time, which is what a selection is made
of.

**Why the jump is the block and not the message.** `Tab` used to be the
conversation turn. A message is a block, so the block jump reaches every message
the message jump did; the reverse is false, and a jump that stepped over the
runs would leave the mechanics — most of what a trajectory is — reachable only
by `j`. One boundary for every format also beats a second one that exists only
under a lens. `S-Tab` is the exact mirror: the same table, walked backwards,
framed the same way.

**A block boundary descends into an open run.** A shut run is one block — that
is what makes it a summary — and `Tab` steps over it. Once it is open, the
blocks inside it are the run's own row and then each step: `Enter` is the reader
asking for what is in there, so that is what the jump walks. A step and the tree
it may have open are **one** block, exactly as a message and its body are, so
`Tab` clears an opened record in one press while `j` reads it a row at a time.
`S-Tab` mirrors the sequence step for step, including stepping back out of the
run to the block above it. At the tail of a document — a trailing run of
mechanics with nothing said after it — `Tab` keeps going through the run rather
than dead-ending, and never moves past the end.

Where a block cannot be placed — past the classified prefix, the tail of a big
trajectory where grouping is not decided yet — the jump has no answer and says
so, rather than landing on a row the next keystroke would renumber. `j` is
unaffected: it never asked what block it was in.

The status bar reads
`agent · record 412/2354 · block 96/≥181 · .message.content[0].text`. The block
clause is lens-only, and it carries `≥` for the same reason the record count
does and then one more: the lens has read a prefix, and grouping makes the block
total *shrink* as classification catches up. Opening a run moves it the other
way — its steps are blocks while it is open — and the counter says so on the
keystroke that opens it, because the index and the total are read off the same
table `Tab` jumps by. Past that prefix there is no block clause at all, rather
than a number the next keystroke changes. The record count keeps saying
**record** — it is the file's own unit, which `#n` and `--toc` also mean, and a
`.jsonl` line is not a step.

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

A step's row is what it did, and what it was **thinking** is under that row,
clipped and muted — `reasoning_content` is text, and a run of steps that only
thought used to say `thinking` five times and nothing else. Opening the step
(`Enter`) shows the thought whole and lists its calls; opening a call shows the
arguments it was given and the output it returned, matched by `source_call_id`
within the step. A result no call claimed gets a part of its own at that level
rather than the bare `· 1 result` the row can spare.

`arguments` is read as an object *or* as the JSON-encoded string the wire format
emits, and in the second case the decoded object is a temporary — so an argument
is always a head with no path back, which is why a long `command` or a
`patchText` shows its opening and says how much more there is. A string that
does **not** parse is kept as the string it is, under the name `arguments`: a
truncated `arguments` is exactly the thing a reader opened the level to look at.

A result's size is stated once and measured once. A `content` that is not a
string is measured as the JSON it is — the same bytes the open level shows one
rung down — rather than as the word `ok`, which is not a size; and a `content`
of `null` reads as absent, which is what `null` means everywhere else here.

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
columns): 50 records, 50 read by the lens, 91 rows shut and 1231 open. Of the
records visible with every run shut, 26 have an open level, contributing 103
part rows in all, and every one of those carries a call that opens further.
Opening every one of those calls in turn — 38 of them — paints every row at 80
columns and at 140, with no row *under* an opened call wider than the view: the
arguments and the output are a wrap, and the call row above them is the only
thing that scrolls sideways.
`--toc --lens atif` prints the whole run as 50 tab-separated lines, headlines
only — a list is a list, and the levels are for a screen.

## The `usage` dialects

The same two files, a different question: not what was said, but what each
record **spent**. `usage` reads a Claude Code session log
(`records_at() -> Lines`) and `usage-atif` reads an ATIF trajectory
(`records_at() -> Member("steps")`); everything that is not that one declaration
is shared, in `src/lens/usage.rs`.

```text
  user       14:01  user
▸ ⟨3 steps · 1 tool call · 22k tokens⟩   14:02
  assistant  14:02  in  1.2k  out  380  read  18k  new 2.1k  ·  Bash(cargo test)
  ↳assistant 14:03  in   800  out   20  read    -  new    -  ·  assistant
  system     14:03  file-history-snapshot
```

They are named `usage` and not `cost`: no price table is compiled in and no money
is shown. A lens must not promise a currency it cannot compute.

**The columns.** The actor is 10 and the clock is 5, so `what` starts at column
21 — the same column every lens's `what` starts at. Then the numeric block, and
then `  ·  ` and what the record did:

| Field | Columns | Made of |
| --- | --- | --- |
| one field | 8 | its label left-justified in 4, then `lens::tokens` right-aligned in 4 |
| the gap between two fields | 2 | |
| `usage`'s block (`in`, `out`, `read`, `new`) | **38** | 4×8 + 3×2 |
| `usage-atif`'s block (`in`, `out`, `read`) | **28** | 3×8 + 2×2 |

The block is that width on **every** row of the file, whatever the numbers are,
which is what makes it a column a reader can scan down. The row never wraps — it
scrolls sideways like every other summary row — so a narrow terminal pans across
the action rather than losing it.

**Three different things a cell can say**, and this is the whole design:

| On the row | Means |
| --- | --- |
| a number, `0` included | the format recorded that value. A recorded zero is a fact about the session, and it is shown |
| `-` | **this record** did not record a field its format has. The column stays, because other records in the same file fill it |
| no column at all | **the format** has no such field. ATIF-v1.7 records no cache-*creation* counter of any kind, so a `usage-atif` row has three fields and never a fourth |

A format-level absence removes the column for the whole file, because alignment
is what a number column is for and alignment is per file; a record-level absence
inside a format that has the field prints `-`. A `0` in the third case would say
"this agent wrote nothing to cache" when the truth is "this format does not
record cache writes", which is a different and false claim.

The middle row is a **defensive** path, and a later reader should not take it
for observed behaviour: in every session log and trajectory this has been
measured against, a record that carries a usage object carries either all of its
format's counters or none of them, so no real row has yet printed a `-`. It is
exercised by hand-written tests only. It stays because a counter set is the
schema of another program and the day one goes missing the column must still
line up and must not read as a zero.

**A record with no usage shows its kind and nothing more** —
`file-history-snapshot`, `queue-operation`, `user` — with no number fields at
all, so the numeric column is simply absent where nothing was spent rather than
a row of zeroes. A `type` the dialect has never seen prints its own name rather
than being swallowed.

**Numbers are floored, never rounded up.** `lens::tokens` is the one spelling
there is: `380`, `1.2k`, `18k`, `1.8M`, never wider than four columns. `999` is
`999` and not `1.0k`; `1999` is `1.9k` and not `2.0k`. A row that says `18k` is
therefore a promise of *at least* 18,000, which is the reading a person makes of
a truncated number and the only one that never overstates what a session spent.

The cost is real and is stated here rather than discovered: a bucket hides its
magnitude, so `18k` is anything up to 18,999 and **the floored numbers on the
rows will not add up by eye to the total on the group row**. Both totals are the
exact sum, spelled once at the end; and the exact integers of any one record are
one `Enter` away.

**Where a total lives, and why neither place is in a dialect.** A lens may not
decide a row, so both are in the record seam:

* the **group row** over a folded run — `lensrow::group_counts` sums the
  members' exact counts and `group_text` spells the third clause,
  `⟨15 steps · 3 tool calls · 128k tokens⟩`, omitted entirely at zero;
* the **status bar** over the document — `record::view::position_text` appends
  `  ·  ≥1.2M tokens` to its `record 812/≥1204 (indexing 44%)` head. The `≥` is
  not cosmetic: classification runs only as far as the reader has scrolled, so an
  unqualified total on an 8.8 MB log would be wrong by most of the file. It is a
  running sum kept by `Plan::classify`, one add per record and nothing per frame,
  and classification runs once per record — so opening a folded run cannot make
  the total jump.

Both are dialect-agnostic: any lens that fills `Summary::tokens` gets them, and a
lens that fills none (`agent`, `atif`) changes not one byte of either.

**Where the numbers are.** For `usage`, exactly one path — `message.usage` —
mapping `input_tokens` → `in`, `output_tokens` → `out`,
`cache_read_input_tokens` → `read`, `cache_creation_input_tokens` → `new`. For
`usage-atif`, `steps[].metrics`, mapping `prompt_tokens` → `in`,
`completion_tokens` → `out`, `cached_tokens` → `read`.

Two things are deliberately **not** added to the total, and both would be a
double count:

* `output_tokens_details.thinking_tokens` and ATIF's
  `metrics.extra.reasoning_tokens` are *subsets* of the output count;
* `usage.iterations[]` is one element per attempt at the request, and on a
  record that carries more than one the outer counters are the **last**
  element's, never the sum of them. Its **length** is the fact — the request was
  retried — and its contents are never summed. This is the mistake the next
  contributor will make, so it is in a comment on `usage::Tokens::total` and in
  a test as well as here.

What that means for a retried request is that the row, the group row and the
status bar all say what the **last attempt** spent. No field of the file states
what the attempts cost between them, and the lens invents nothing: the row says
the request was retried, and the list itself is one `Enter` (or `r`) away.

Neither is lost. Both are on the open level, along with the exact integers, the
model the numbers were spent on, the cache-creation breakdown, and — for
`usage` — a `service_tier` or `speed` that is **not** `standard`, which is
exactly the anomaly a reader opened the row to find, and which would be 99.96%
noise as a column.

**The one decision that shapes the document.** For `usage`, a **human** `user`
turn is a `Class::Message` and everything else is a `Class::Step`, so the run
between two human turns is exactly one turn's mechanics and the group row over
it totals what that turn cost. A `user` record whose content blocks are
`tool_result` is mechanics — the same line `agent` draws — and drawing it
anywhere else would shred every run into pairs and leave no group row totalling
anything. For `usage-atif`, `source` is the discriminator: `user` is the turn
boundary, `agent` is a step.

**A subagent is `↳assistant`, with no space** — ten columns exactly, which is
what the actor field is wide. `agent`'s `↳ assistant` is eleven and pushes every
column on that row one to the right; more than half the records of a real session
carry `isSidechain`, and on a lens whose whole product is a column of numbers
that is fatal. `Who` still carries the colour, so the row still paints as an
assistant. ATIF has no such flag and no row is marked: inventing one would be a
guess.

**Neither dialect shows any message text**: `Summary::body` is `None` on every
row, because "what was said" is what `--lens agent` and `--lens atif` are for.
That also means these dialects allocate nothing per record but their own one-line
`what`, so a log whose longest line is most of a megabyte costs them a parse and
no more.

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
     into a format and a refusal. **It is one answer**: a dialect that wants to
     read two file shapes is two entries over one shared module, which is what
     `usage` and `usage-atif` are.
   * `read(&mut self, &Value) -> Option<Summary>` — called **once per record, in
     file order**, so a dialect may carry state (the agent lens keeps a bounded
     ring of `tool_use` ids so a result can name its call). Return `None` for
     anything it does not recognise; that record then renders generically.
   * `detail(&self, &Value) -> Vec<Part>` — optional, and what the **open
     level** shows: what this record's parts *are*. Called for the record the
     reader opened, when they open it, and thrown away when they close it —
     never once per record and never stored. `&self` rather than `&mut self` is
     the contract: `read` runs far ahead of the viewport, so whatever state a
     dialect carried across records is long past by the time a key is pressed.
     A dialect that cannot answer from *this record alone* returns what it can
     and leaves the rest `None`; the raw tree is one `r` away, and a gap is
     better than a guess.
2. **One entry in `lens::LENSES`** — `(NAME, || Box::new(Mine::default()))`.
3. **Tests beside it** (`src/lens/<name>_tests.rs`), with **hand-written
   fixtures**. Real session logs are private; read one to learn the shape, never
   copy it into the repository.

A `Summary` is eight fields and no styling:

| Field | Meaning |
| --- | --- |
| `class` | `Message` (never folded away) or `Step` (folds into a run with its neighbours) |
| `who` | `User` / `Assistant` / `Tool` / `System` — colour only |
| `actor` | the text in the speaker column: `user`, `tool`, `↳ assistant` |
| `time` | `HH:MM`, or `None`; `lens::clock` does ISO-8601 |
| `what` | one line: `lens::excerpt` collapses whitespace and cuts to width. What `--toc` prints, and what the row paints for a record with no `body` |
| `calls` | tool calls in this record, for a run's `· 4 tool calls` |
| `tokens` | every token unit this record recorded, **exact**; `0` when it records none. The two places a total appears add these up and spell the sum once with `lens::tokens`, because a sum of rounded numbers is not the rounding of a sum |
| `body` | the record's own text, for the rows under the summary: what was said on a message, what it was thinking on a step. `None` when it has none. `class` decides how it is painted — a message's body is one wrap split between its row and the rows under it; a step's row keeps saying what it *did* and its text goes wholly underneath |

A `Part` is two variants and no styling either:

| Variant | Fields | What it is |
| --- | --- | --- |
| `Text` | `label: &'static str`, `body: Body` | a named stretch of text the row's own body is not already showing: a thought beside a message, a second text block. Shown **whole** — parts are the open level, and the open level is the whole of that text |
| `Call` | `tool`, `arg`, `args: Vec<(String, Body)>`, `result: Option<Body>` | a call to a tool: what was called, the one argument a headline shows, every argument, and what came back |

`Part::opens` is what the call row's marker is painted from, and it is false for
a call with no arguments *and* no result — the row is still there, saying what
was called; it just does not advertise a fold it has not got.

There is **one** reading of an argument list, `lens::part::args_of`, and both
dialects use it. An object is its members; anything else — an array, a bare or
half-written `arguments` string — is one entry named `arguments` holding what
the file said. Dropping those was the seam's one silent clip: the open level
said the call had no arguments while the command sat in the tree one `r` away.
Absent, `null` and `[]` still mean the same thing, and mean it quietly.

`result` is an `Option` because of the `agent` dialect and not in spite of it:
its calls are answered by a **later record**, and a `Body`'s path starts at the
record it belongs to. The call part carries `None` and the result record
contributes a part of its own. A dialect whose answers are in the same record —
`atif` — fills it in.

Every stretch of text in a `Part` is a `Body`, on purpose. One record can carry
five calls whose outputs are fifteen kilobytes apiece, and a level that read
them into memory to count its rows would make opening one step cost what the
step cost. Two of those paths are real and one is not, and the code says which:
a **result** is one string node of the record (`observation.results[3].content`)
and opens whole; an **argument** sits under a key that is its own name, and a
`Step::Key` is `&'static str`, so it is a head with no path — clipped, with the
row under it stating the true remainder, and whole in the tree one `r` away.

A `Summary` is kept for **every** classified record, and nothing here may
allocate per document — so a `Body` is *not* the message. It is the first
`lens::BODY_KEEP` bytes of it, the whole message's byte and line counts, and the
path back to the text inside the record (`message`, or
`message.content[1].text`). The clip and the row arithmetic are answered from
the head alone, so a resize reads no file; opening a message longer than the
head reads it back out of the record that is being painted anyway. Holding whole
messages instead would make a long log's summaries as big as the log.


What a dialect **never** touches: rows, folding, row arithmetic, the levels, the
key that walks them, search, yanking, the outline or the status bar — including
how tall a message is, which is `src/source/record/body.rs`'s answer and depends
on the width. A dialect says what a record *is* and what its parts *are*; where
those land on a screen is not its business. Grouping is
`src/source/record/plan.rs` with the levels and heights in
`src/source/record/plan_rows.rs`, painting is `src/source/record/lensrow.rs` and
`src/source/record/parts.rs`, the ladder is `src/source/record/ladder.rs`, the
fold keys are `src/source/record/ops.rs`, and all of them are dialect-agnostic —
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
path back to the rest. Since a step's reasoning is a body too, a trajectory that
records thinking pays that ceiling on those records as well: on a synthetic
45 MB, 20 000-step ATIF file with a 420-byte thought on every other step, that
is +4 MB of resident memory against the same file with the thoughts removed —
the cost tracks the *number* of records that thought, not what they thought.
Open time is unmoved (2 ms either way), because none of it is read until the
record is classified.

What the **open level** costs is nothing at all until it is asked for.
`Lens::detail` runs for the record being measured or painted and its answer is
dropped; a document holds one `Under` — two `usize` — per classified record and
not one byte of a tool's output. The one keystroke that pays is `zR`, which puts
every record it reached at that level and so asks every dialect about every
record: 215 ms on that 45 MB file, against 24 ms before there was a level to
open. `zR` was already the keystroke that scales with the file, and this is the
same trade in the same place. On a synthetic 4 MB / 8 MB ATIF trajectory that is
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
