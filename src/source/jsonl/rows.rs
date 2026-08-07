//! Turning records into rows: the summary row, the expanded tree, the window
//! and the outline over it.
//!
//! A second `impl JsonlSource` rather than a second type, split out of `mod.rs`
//! so both stay under the size limit. Everything here is `&self` where it can
//! be: laying a row out is a read of the file behind a [`RefCell`], not a
//! change to the document.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;

impl JsonlSource {
    // -- rows -------------------------------------------------------------------

    /// A record's own row: the lens's reading of it when there is one, and the
    /// generic collapsed-record row otherwise.
    ///
    /// The fallback is the point (SPEC.md §Lenses): a record the lens does not
    /// recognise is *never* hidden and never summarised wrongly — it renders
    /// exactly as it would with no lens at all.
    pub(super) fn summary_row(&self, record: usize) -> Line {
        let inset = self.in_open_group(record);
        match self.lens_row(record, inset) {
            Some(line) => line,
            None => self.record_row(record, inset),
        }
    }

    /// Is this record a member of a group that is currently open? Its row is
    /// then indented under the group's, so a run reads as one thing.
    pub(super) fn in_open_group(&self, record: usize) -> bool {
        let Some(plan) = self.plan.as_ref() else {
            return false;
        };
        plan.item_of_record(record)
            .and_then(|i| plan.item(i))
            .map(|it| it.is_group() && it.open)
            .unwrap_or(false)
    }

    /// The summary row of record `n`: the fold marker, then what the record is.
    pub(super) fn record_row(&self, record: usize, inset: bool) -> Line {
        let spans = self.with_record(record, |r| match r {
            Record::Value(v) => match tree::row_count(v) {
                0 => leaf(tree::record_spans(v)),
                _ => marker(tree::record_spans(v)),
            },
            Record::Bad(why) => leaf(vec![Span::new(
                format!("line {}: {why}", record + 1),
                tree::style::error(),
            )]),
        });
        let spans = lensrow::inset_after_gutter(spans, inset);
        Line {
            spans,
            block: 0,
            source_line: record + 1,
            heading: None,
            scroll: false,
            kind: LineKind::Heading,
        }
    }

    /// One row, wherever it falls. O(one record) for a row inside a tree, which
    /// is why [`Source::lines`] does not build a window out of this.
    pub(crate) fn row_line(&self, row: usize) -> Option<Line> {
        if row >= self.len_rows() {
            return None;
        }
        match self.spot(row) {
            Spot::Group { item } => Some(self.group_row(item)),
            Spot::Record { record, sub } => match sub {
                0 => Some(self.summary_row(record)),
                n => self.with_tree(record, |rows| rows.get(n - 1).cloned()),
            },
        }
    }

    /// `count` rows from `start`, laid out record by record so a record's tree
    /// is walked once per frame rather than once per row of it.
    pub(super) fn collect(&self, start: usize, count: usize) -> Vec<Line> {
        if self.plan.is_some() {
            // Under a lens a row is a group row, a record row or a tree row,
            // and the caches behind them (`laid`, `Cache`) already make
            // consecutive rows of one record cheap.
            return (start..start.saturating_add(count))
                .map_while(|row| self.row_line(row))
                .collect();
        }
        let mut out: Vec<Line> = Vec::with_capacity(count);
        let (mut record, mut sub) = self.map.at(start);
        let known = self.known();
        while out.len() < count && record < known {
            if sub == 0 {
                out.push(self.record_row(record, false));
                sub = 1;
            }
            if self.map.is_open(record) {
                let want = count - out.len();
                self.with_tree(record, |rows| {
                    let from = (sub - 1).min(rows.len());
                    let to = (from + want).min(rows.len());
                    out.extend(rows[from..to].iter().cloned());
                });
            }
            record += 1;
            sub = 0;
        }
        out
    }

    /// The path of the row under the cursor: `.content[0].text`. Empty on a
    /// record's own summary row, which is the record rather than a place in it.
    pub(crate) fn path_of(&self, row: usize) -> Option<String> {
        let path = self.at_row(row, |p, _| p.to_string())?;
        (!path.is_empty()).then_some(path)
    }

    pub(super) fn row_text(&self, row: usize) -> String {
        self.row_line(row).map(|l| l.text()).unwrap_or_default()
    }

    /// Rows in the document right now: one per known record, plus whatever the
    /// open ones add.
    pub(crate) fn len_rows(&self) -> usize {
        match (self.known(), self.plan.as_ref()) {
            (0, _) => 0,
            (known, Some(plan)) => plan.rows(known, &self.map),
            (n, None) => n + self.map.extra_total(),
        }
    }

    /// The record a row belongs to. A group's row stands for its first record,
    /// which is what keeps `record N/M`, search and yanking agreeing with what
    /// the cursor is on.
    pub(crate) fn record_at(&self, row: usize) -> usize {
        match self.spot(row) {
            Spot::Record { record, .. } => record,
            Spot::Group { item } => self.item_first(item),
        }
    }

    /// The value (and its path) on a tree row, or the whole record on a
    /// summary row. What `y` copies and what the status bar names.
    pub(super) fn at_row<T>(&self, row: usize, f: impl FnOnce(&str, &Value) -> T) -> Option<T> {
        let (record, sub) = match self.spot(row) {
            Spot::Record { record, sub } => (record, sub),
            // A group's row stands for the run it holds; the value under the
            // cursor there is its first record.
            Spot::Group { item } => (self.item_first(item), 0),
        };
        if record >= self.known() {
            return None;
        }
        self.with_record(record, |r| {
            let v = r.value()?;
            match sub {
                0 => Some(f("", v)),
                n => tree::node_at(v, n - 1).map(|(path, node)| f(&path, node)),
            }
        })
    }

    // -- expansion ---------------------------------------------------------------

    /// Open record `n`, laying its tree out to learn how many rows it adds.
    pub(super) fn open_record(&mut self, record: usize) -> bool {
        if record >= self.known() {
            return false;
        }
        let extra = self.tree_len(record);
        self.map.open(record, extra)
    }

    /// Under `zR`, open records the viewport has reached. Bounded by the rows
    /// asked for, so one frame never opens more than it is about to paint.
    pub(super) fn fill_expansion(&mut self, upto_row: usize) {
        if !self.expand_all {
            return;
        }
        self.open_groups(upto_row);
        let known = self.known();
        while self.filled < known && self.filled < EXPAND_CAP {
            // A record still inside a closed group has no row to open; it will
            // be reached once the group the viewport is approaching opens.
            if !self.record_visible(self.filled) {
                return;
            }
            if self.row_of_record(self.filled) > upto_row {
                return;
            }
            let record = self.filled;
            self.open_record(record);
            self.filled += 1;
        }
    }

    /// `zR` under a lens opens the groups the viewport has reached, the same
    /// bounded way it opens records.
    pub(super) fn open_groups(&mut self, upto_row: usize) {
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        let mut map = std::mem::take(&mut self.map);
        plan.open_upto(upto_row, &mut map);
        self.plan = Some(plan);
        self.map = map;
    }

    /// Open the group holding `record`, so a search hit is never left behind a
    /// fold. Does nothing when there is no lens, or the record is not in one.
    pub(super) fn reveal_record(&mut self, record: usize) {
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        let mut map = std::mem::take(&mut self.map);
        if let Some(item) = plan.item_of_record(record) {
            plan.set_open(item, true, &mut map);
            plan.sync();
        }
        self.plan = Some(plan);
        self.map = map;
    }

    /// The first record of a plan item.
    pub(crate) fn item_first(&self, item: usize) -> usize {
        self.plan
            .as_ref()
            .and_then(|p| p.item(item))
            .map(|it| it.first)
            .unwrap_or(0)
    }

    /// Open or close the group whose first record is `first`.
    pub(super) fn set_group(&mut self, first: usize, open: bool) -> bool {
        let Some(mut plan) = self.plan.take() else {
            return false;
        };
        let mut map = std::mem::take(&mut self.map);
        let changed = match plan.item_of_record(first) {
            Some(item) => plan.set_open(item, open, &mut map),
            None => false,
        };
        plan.sync();
        self.plan = Some(plan);
        self.map = map;
        changed
    }

    /// `zM`: shut every group, which is free — nothing is parsed to close one.
    pub(super) fn close_groups(&mut self) {
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        let mut map = std::mem::take(&mut self.map);
        plan.close_all(&mut map);
        plan.sync();
        self.plan = Some(plan);
        self.map = map;
    }

    /// `Tab` / `S-Tab` under a lens: the next thing that stands on its own —
    /// a message, or a folded run — rather than the next record, most of which
    /// a lens has deliberately folded away.
    pub(super) fn next_item(&self, row: usize, forward: bool) -> Option<usize> {
        let plan = self.plan.as_ref()?;
        let record = self.record_at(row);
        let cur = plan.item_of_record(record)?;
        let base = plan.row_of_item(cur, &self.map);
        match forward {
            true if cur + 1 < plan.items().len() => Some(plan.row_of_item(cur + 1, &self.map)),
            true => None,
            // Inside an item, the first press goes back to its header row.
            false if row > base => Some(base),
            false if cur > 0 => Some(plan.row_of_item(cur - 1, &self.map)),
            false => None,
        }
    }

    /// `Y` on a group's row: every record the run holds, one JSON document per
    /// line — the folded rows as data, so what was copied is what was hidden.
    pub(super) fn yank_group(&self, item: usize) -> Option<Yank> {
        let plan = self.plan.as_ref()?;
        let it = plan.item(item)?;
        let (first, count) = (it.first, it.count);
        let mut out = String::new();
        for record in first..first + count {
            if let Some(text) = self.with_record(record, |r| r.value().map(|v| v.to_json())) {
                out.push_str(&text);
                out.push('\n');
            }
        }
        JsonlSource::yank(out, format!("{count} records"))
    }

    // -- the outline ---------------------------------------------------------------

    /// Rebuild the outline over the records the last frame painted.
    ///
    /// Deliberately a *window*, not the document: an [`Entry`] carries the
    /// record's summary text, so an outline over a million records would parse
    /// a million records — the one thing this source must never do. The pager
    /// only ever indexes it with a row the frame just painted
    /// ([`Source::section_at`] then [`Source::set_fold`], in one keystroke), so
    /// a window is enough for folding; what it costs is that `o` lists the
    /// records on screen rather than all of them, which for a log is the more
    /// useful answer anyway.
    pub(super) fn rebuild_outline(&mut self, rows: Range<usize>) {
        self.outline.clear();
        if rows.start >= rows.end {
            return;
        }
        if self.plan.is_some() {
            return self.rebuild_lens_outline(rows);
        }
        let first = self.record_at(rows.start);
        let last = self.record_at(rows.end.saturating_sub(1));
        for record in first..=last.min(self.known().saturating_sub(1)) {
            if self.tree_len(record) == 0 {
                continue;
            }
            let text = self.with_record(record, |r| match r.value() {
                Some(v) => tree::summary_text(v),
                None => String::new(),
            });
            self.outline.push(Entry {
                level: 1,
                id: fold_id(record),
                text: format!("line {}  {text}", record + 1),
                anchor: Anchor(self.row_of_record(record)),
                folded: !self.map.is_open(record),
            });
        }
    }

    /// The outline a lens gives: one entry per header row the frame painted —
    /// a group, or a record that can be opened. The same window discipline the
    /// generic outline follows, for the same reason.
    fn rebuild_lens_outline(&mut self, rows: Range<usize>) {
        let mut entries: Vec<Entry> = Vec::new();
        for row in rows.start..rows.end.min(self.len_rows()) {
            match self.spot(row) {
                Spot::Group { item } => {
                    let open = self
                        .plan
                        .as_ref()
                        .and_then(|p| p.item(item))
                        .map(|it| it.open)
                        .unwrap_or(false);
                    let first = self.record_at(row);
                    entries.push(Entry {
                        level: 1,
                        id: plan::group_id(first),
                        text: self.group_row(item).text().trim_end().to_string(),
                        anchor: Anchor(row),
                        folded: !open,
                    });
                }
                Spot::Record { record, sub: 0 } if self.tree_len(record) > 0 => {
                    entries.push(Entry {
                        level: u8::from(self.in_open_group(record)) + 1,
                        id: fold_id(record),
                        text: self.summary_row(record).text().trim_end().to_string(),
                        anchor: Anchor(row),
                        folded: !self.map.is_open(record),
                    });
                }
                _ => {}
            }
        }
        self.outline = entries;
    }

    /// The first `n` records as one summary line each — `--toc`.
    ///
    /// Through the lens when there is one, so `--toc --lens agent` is the
    /// conversation as a list rather than a column of `{…12 keys}`.
    pub fn summaries(&mut self, n: usize) -> Vec<String> {
        self.index_to(n, FRAME_BYTES);
        self.classify_to(n);
        let take = n.min(self.known());
        (0..take)
            .map(|r| {
                if let Some(sum) = self.plan.as_ref().and_then(|p| p.summary(r)) {
                    let time = sum.time.clone().unwrap_or_default();
                    // Through the sanitiser, exactly as the painted row is: a
                    // record may hold any byte, and `--toc` writes straight to
                    // a terminal (SPEC.md §JSON, the shared sanitiser).
                    let actor = crate::render::visible(&sum.actor);
                    let what = crate::render::visible(&sum.what);
                    return format!("{}\t{actor}\t{time}\t{what}", r + 1);
                }
                let body = self.with_record(r, |rec| match rec {
                    Record::Value(v) => tree::record_spans(v)
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<String>(),
                    Record::Bad(why) => why.clone(),
                });
                format!("{}\t{body}", r + 1)
            })
            .collect()
    }
}
