//! The [`Source`] implementation itself: what the pager sees.
//!
//! Split from `source.rs`, which holds the state and the reads, so both stay
//! under the size limit. Written once for every record format: what a `.jsonl`
//! and an array inside a document disagree about is [`Store`] and nothing here.
//! The order of the sections below is the order of the trait's own
//! (`src/source/mod.rs`), and every method a record file has no honest answer
//! for says so rather than inventing one.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;

impl<S: Store> Source for RecordSource<S> {
    // -- layout ---------------------------------------------------------------

    /// A tree row is never wrapped, so one node stays one row at any width —
    /// but the **message** under a lens row is wrapped, so a width change
    /// re-lays every body and moves every row below the first of them. That is
    /// why [`Mark`] here is a *record* rather than a row: the cursor comes back
    /// to what it was reading, not to row 41 of a different layout.
    fn set_width(&mut self, cols: usize) {
        self.view = cols.max(1);
        // Index a first slice so `len()` is not zero before the first paint.
        // A screenful of offsets, not the file: the same bounded work the CSV
        // source does when it samples its columns.
        self.index_to(FIRST_RECORDS, FRAME_BYTES);
        // And read that slice through the lens, so the first frame is already
        // grouped rather than re-flowing under the reader.
        self.classify_to(FIRST_CLASS);
        // Re-wrap what is already classified. No record is re-read and none is
        // re-classified: the summaries hold what the measurement needs.
        self.remeasure();
        // The last search hit was a row in the old layout.
        self.found = None;
    }

    fn len(&self) -> usize {
        self.len_rows()
    }

    /// The window, built record by record rather than row by row: laying a
    /// record's tree out is O(that record), so asking for it once per painted
    /// row would be quadratic in a big record.
    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        let first = self.record_at(rows.start);
        self.index_to(first.saturating_add(LOOKAHEAD), FRAME_BYTES);
        // Grouping is decided ahead of the window, so nothing above the
        // viewport ever moves under the reader (see `plan`).
        self.classify_to(first.saturating_add(CLASS_AHEAD));
        self.fill_expansion(rows.end.saturating_add(LOOKAHEAD));
        let end = rows.end.min(self.len_rows());
        let start = rows.start.min(end);
        self.window = start..end;
        self.rebuild_outline(start..end);
        self.collect(start, end - start)
    }

    // -- viewport affordances ---------------------------------------------------

    /// A record tree is not prose: a 200-column terminal should show 200
    /// columns of it, the same call the CSV grid makes.
    fn full_width(&self) -> bool {
        true
    }

    /// A lens turns a record document into a list of blocks — a message with
    /// what was said under it, or a folded run of mechanics — and a row is then
    /// a part of one rather than a thing, so `j`/`k` move between blocks. With
    /// no lens this is the generic tree again: one record per row, one node per
    /// row, each already its own unit. `plan.is_some()` is the one spelling of
    /// "is a lens on" this file uses (see [`Source::mark`]).
    fn blocks(&self) -> bool {
        self.plan.is_some()
    }

    fn block_at(&self, row: usize) -> Option<Range<usize>> {
        self.block_rows(row)
    }

    fn position_text(&self, row: usize) -> Option<String> {
        let (record, sub) = match self.spot(row) {
            Spot::Record { record, sub } => (record, sub),
            // A body row is still that record's row: the message is what the
            // summary above it said, not a place inside the record.
            Spot::Body { record, .. } => (record, 0),
            Spot::Group { item } => (self.item_first(item), 0),
        };
        let known = self.known();
        let total = match self.complete() {
            true => format!("{known}"),
            false => {
                let pct = self.store.progress();
                format!("\u{2265}{known} (indexing {pct}%)")
            }
        };
        let head = match self.lens_name() {
            Some(lens) => format!(
                "{lens}  \u{b7}  record {}/{total}{}",
                record.saturating_add(1),
                self.block_text(row)
            ),
            None => format!("record {}/{total}", record.saturating_add(1)),
        };
        // The path only means something inside an open record; on the summary
        // row the record *is* the value, and `.` would be noise.
        Some(match sub {
            0 => head,
            _ => match self.path_of(row) {
                Some(p) => format!("{head}  \u{b7}  {p}"),
                None => head,
            },
        })
    }

    /// `G`. Until the line index has reached the end of the file there is no
    /// honest answer, so this reports how far the scan got and lets the pager
    /// drive it (SPEC.md §CSV, the same contract).
    fn end(&self) -> End {
        if !self.complete() {
            return End::Scanning(self.store.progress());
        }
        // The index has read the file, but the lens decides how many rows
        // those records make: answering now would put `G` in the middle.
        if let Some(plan) = self.plan.as_ref() {
            let (done, known) = (plan.classified(), self.known());
            if done < known {
                let pct = (done as u64 * 100 / known.max(1) as u64) as u8;
                return End::Scanning(pct);
            }
        }
        End::At(self.len_rows().saturating_sub(1))
    }

    /// True while there is more to do *or* this slice found new records:
    /// a caller that walks the document by `len()` — `dump::write_source` —
    /// asks again only while this says yes, and the slice that finishes the
    /// index is also the one that produces most of its rows.
    fn extend(&mut self) -> bool {
        let more = self.store.extend(IDLE_BYTES);
        // The lens gets its own bounded slice of the same idle tick: a record
        // is only grouped once it has been read, and `G` waits on that.
        let done = self.plan.as_ref().map(|p| p.classified());
        if let Some(done) = done {
            if done < self.known() {
                self.classify_to(done.saturating_add(CLASS_SLICE));
                return true;
            }
        }
        more
    }

    // -- positions --------------------------------------------------------------
    //
    // An anchor is a row. It survives its *own* record's fold, which is what
    // the pager needs: opening or closing a record only ever adds or removes
    // rows below its summary row, never moves it. Rows below a record that
    // someone else folded do move, and an anchor pointing there lands on a
    // neighbour rather than on a panic — which is the contract's floor.

    fn anchor(&self, row: usize) -> Option<Anchor> {
        (row < self.len_rows()).then_some(Anchor(row))
    }

    fn row_of(&self, anchor: Anchor) -> Option<usize> {
        (anchor.0 < self.len_rows()).then_some(anchor.0)
    }

    fn reveal(&mut self, anchor: Anchor) -> Option<usize> {
        let n = self.len_rows();
        (n > 0).then(|| anchor.0.min(n - 1))
    }

    /// Under a lens a mark is the **record**, not the row: a body is wrapped,
    /// so a resize moves rows and a row number would mean somewhere else
    /// afterwards (`src/source/mod.rs`: a mark is the same *content*).
    ///
    /// What that costs is the offset *inside* a record: the cursor on tree row
    /// seven of an open record, or on line four of a message, comes back to
    /// that record's summary row. A `Mark` is one number, and packing an offset
    /// into it would be a lie about which record a bigger number is in.
    ///
    /// With no lens there is no body and nothing wraps — a tree row is one row
    /// at every width — so the row *is* the same content, and paying that cost
    /// would throw away the place inside an open record for nothing.
    fn mark(&self, row: usize) -> Option<Mark> {
        if row >= self.len_rows() {
            return None;
        }
        Some(match self.plan.is_some() {
            true => Mark(self.record_at(row)),
            false => Mark(row),
        })
    }

    fn locate(&self, mark: Mark) -> Option<usize> {
        let rows = self.len_rows();
        if rows == 0 {
            return None;
        }
        if self.plan.is_none() {
            return Some(mark.0.min(rows - 1));
        }
        let known = self.known();
        if known == 0 {
            return None;
        }
        Some(self.row_of_record(mark.0.min(known - 1)))
    }

    // -- structure ---------------------------------------------------------------

    /// The records the last frame painted, not the whole file — see
    /// [`RecordSource::rebuild_outline`] for why a million-record outline is a
    /// million parses.
    fn outline(&self) -> &[Entry] {
        &self.outline
    }

    fn section_at(&self, row: usize) -> Option<usize> {
        let id = self.fold_id_at(self.spot(row));
        self.outline.iter().position(|e| e.id == id)
    }

    fn set_fold(&mut self, entry: usize, closed: bool) -> bool {
        let id = match self.outline.get(entry) {
            Some(e) => e.id.clone(),
            None => return false,
        };
        // A group id (`g12`) cannot collide with a record id (`/12`); the seam
        // tells them apart, and `None` is "not a group" — this file only has to
        // answer for its own records, which is the half that costs a parse.
        if let Some(changed) = self.set_group_by_id(&id, !closed) {
            if changed {
                if let Some(e) = self.outline.get_mut(entry) {
                    e.folded = closed;
                }
            }
            return changed;
        }
        let record: usize = match jsonrow::top_index(&id) {
            Some(r) => r,
            None => return false,
        };
        let changed = match closed {
            true => self.map.close(record),
            false => self.open_record(record),
        };
        if changed {
            if let Some(e) = self.outline.get_mut(entry) {
                e.folded = closed;
            }
        }
        changed
    }

    /// `zM` shuts every record, which is free. `zR` opens them as the viewport
    /// reaches them, up to [`EXPAND_CAP`]: opening a million records means
    /// parsing a million records, and a reader that froze on one keystroke
    /// would be worse than one that opens what you can see.
    fn fold_all(&mut self, closed: bool) {
        self.expand_all = !closed;
        self.filled = 0;
        self.map.clear();
        // Messages go with it: `zR` is "show me everything here", and a dump
        // (`--plain`, which is `fold_all(false)` and nothing else) must print
        // the whole of what was said rather than a viewport's clip of it.
        self.set_all_bodies(!closed);
        if closed {
            self.close_groups();
        }
        if !closed {
            let upto = self.window.end.max(1).saturating_add(LOOKAHEAD);
            self.fill_expansion(upto);
        }
    }

    /// `Enter` / `za` on a message shows what was said, whole or clipped —
    /// the row's own fold, which is not an outline entry and could not be
    /// reached through one. Everything else falls through to the outline, so
    /// a group row still opens its run and a record row still opens its tree.
    fn fold_here(&mut self, row: usize) -> Option<bool> {
        self.toggle_body(row)
    }

    /// `zt`: the raw record under the cursor, whatever its body is doing.
    fn toggle_tree(&mut self, row: usize) -> Option<String> {
        self.open_tree_at(row)
    }

    /// The shared fold-id vocabulary ([`jsonrow::ALL_OPEN`]): a default plus the
    /// ids that disagree with it, each id a member-index path from the root.
    /// A record file's root is the implicit list of records, so record 4 is
    /// `/4` — the same id the document reader would give member 4 of its root.
    ///
    /// The default here is *shut*, so the exceptions are the open records: a
    /// million-record file has a million closed ones and listing those is the
    /// one thing this source must never do. `zR` is [`jsonrow::ALL_OPEN`] and
    /// nothing else, exactly as it is on the document side.
    fn folds(&self) -> FoldState {
        let mut out: Vec<String> = Vec::new();
        if self.expand_all {
            out.push(jsonrow::ALL_OPEN.to_string());
        }
        out.extend(self.group_folds());
        out.extend(self.map.records().map(fold_id));
        out
    }

    fn set_folds(&mut self, folds: FoldState) {
        self.map.clear();
        self.expand_all = folds.iter().any(|s| s == jsonrow::ALL_OPEN);
        self.filled = 0;
        self.restore_groups(&folds);
        for id in &folds {
            if let Some(record) = jsonrow::top_index(id) {
                self.open_record(record);
            }
        }
        if self.expand_all {
            self.fill_expansion(self.window.end.max(1).saturating_add(LOOKAHEAD));
        }
    }

    fn hidden_at(&self, row: usize) -> Option<usize> {
        let (record, sub) = match self.spot(row) {
            // A body row hides nothing: the clip says what it is not showing,
            // on its own last row, in the message's own lines.
            Spot::Body { .. } => return None,
            // A closed group hides one row per record it holds; their trees
            // are closed with it, so there is nothing else under it.
            Spot::Group { item } => {
                let hidden = self.plan.as_ref().map(|p| p.hidden(item)).unwrap_or(0);
                return (hidden > 0).then_some(hidden);
            }
            Spot::Record { record, sub } => (record, sub),
        };
        if sub != 0 || self.map.is_open(record) || record >= self.known() {
            return None;
        }
        match self.tree_len(record) {
            0 => None,
            n => Some(n),
        }
    }

    /// Block boundaries: `j`/`k`, and `Tab`'s fallback — see `plan_block.rs`.
    fn next_landmark(&self, row: usize, forward: bool) -> Option<usize> {
        if self.plan.is_some() {
            return self.next_block_row(row, forward);
        }
        let (record, sub) = self.map.at(row);
        match forward {
            true if record + 1 < self.known() => Some(self.map.row_of(record + 1)),
            true => None,
            false if sub > 0 => Some(self.map.row_of(record)),
            false if record > 0 => Some(self.map.row_of(record - 1)),
            false => None,
        }
    }

    /// `Tab` / `S-Tab` under a lens: the next **message**, since `j`/`k` already
    /// step between blocks and half of those are mechanics. With no lens there
    /// are no messages and `Tab` is the next record, exactly what it was.
    fn next_message(&self, row: usize, forward: bool) -> Option<usize> {
        match self.plan.is_some() {
            true => self.next_message_row(row, forward),
            false => self.next_landmark(row, forward),
        }
    }

    /// A record number as an id, so `#41` jumps to line 42's record.
    fn goto_id(&mut self, id: &str) -> Option<usize> {
        let record: usize = id.trim_start_matches('#').parse().ok()?;
        self.index_to(record.saturating_add(2), FRAME_BYTES);
        self.classify_to(record.saturating_add(1));
        (record < self.known()).then(|| {
            self.reveal_record(record);
            self.row_of_record(record)
        })
    }

    fn links(&self) -> &[LinkSite] {
        &self.none_links
    }

    // -- search --------------------------------------------------------------

    fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.sensitive = search::case_sensitive(query);
        self.needle = match self.sensitive {
            true => query.to_string(),
            false => query.to_lowercase(),
        };
        self.found = None;
    }

    /// 1 while the last sweep has a hit, 0 otherwise. A lazily indexed file
    /// cannot be counted without reading all of it, and a number would be a
    /// lie — the same answer the CSV source gives.
    fn match_count(&self) -> usize {
        usize::from(self.found.is_some())
    }

    fn current_match(&self) -> Option<usize> {
        self.found.map(|_| 0)
    }

    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit> {
        let found = self.sweep(origin.0, dir, true);
        self.hit(found)
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        let start = self.found.unwrap_or(from.0);
        let found = self.sweep(start, dir, false);
        self.hit(found)
    }

    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        if self.query.is_empty() {
            return Vec::new();
        }
        let current = self.found == Some(row);
        search::find_in(&self.row_text(row), &self.query, self.sensitive)
            .into_iter()
            .map(|(start, end)| MatchSpan { start, end, current })
            .collect()
    }

    // -- yank -----------------------------------------------------------------

    /// A visual selection: each row as the value on it, one per line.
    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let end = rows.end.min(self.len_rows());
        let start = rows.start.min(end);
        let mut out = String::new();
        let mut n = 0;
        for row in start..end {
            if let Some(text) = self.row_json(row) {
                out.push_str(&text);
                out.push('\n');
                n += 1;
            }
        }
        let what = match n {
            1 => "1 value".to_string(),
            n => format!("{n} values"),
        };
        RecordSource::<S>::yank(out, what)
    }

    /// `y`: the value under the cursor (SPEC.md §JSON).
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let (record, sub) = match self.spot(row) {
            Spot::Record { record, sub } => (record, sub),
            Spot::Body { record, .. } => (record, 0),
            Spot::Group { item } => (self.item_first(item), 0),
        };
        let text = self.row_json(row)?;
        let what = match self.path_of(row) {
            Some(p) if sub > 0 => format!("{p} \u{b7} record {}", record + 1),
            _ => format!("record {}", record + 1),
        };
        RecordSource::<S>::yank(format!("{text}\n"), what)
    }

    /// `Y`: the whole record as one line of valid JSON, wherever in it the
    /// cursor happens to be.
    fn yank_section(&self, row: usize) -> Option<Yank> {
        // On a group's row the section *is* the run it folds: every record it
        // holds, one JSON document per line. Copying the first record alone
        // would be copying what the row does not stand for.
        if let Spot::Group { item } = self.spot(row) {
            return self.yank_group(item);
        }
        let record = self.record_at(row);
        if record >= self.known() {
            return None;
        }
        let text = self.with_record(record, |r| r.value().map(|v| v.to_json()))?;
        RecordSource::<S>::yank(format!("{text}\n"), format!("record {}", record + 1))
    }

    /// `c`: the record's source text, verbatim — the bytes the file holds, not
    /// this reader's re-serialisation of them. A `.jsonl` record is a line and
    /// says so; a record inside a document is a member and says that.
    fn yank_block(&self, row: usize) -> Option<Yank> {
        let record = self.record_at(row);
        if record >= self.known() {
            return None;
        }
        let raw = String::from_utf8_lossy(&self.raw(record)).into_owned();
        let what = format!("{} {} verbatim", self.store.unit(), record + 1);
        RecordSource::<S>::yank(format!("{raw}\n"), what)
    }
}
