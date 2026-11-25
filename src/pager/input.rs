//! Input dispatch: key press -> [`Action`] -> state change.
//!
//! Split out of `mod.rs` to keep both files well under the size limit; these
//! are the same `Pager` methods, just living next door. Where the cursor
//! actually goes once an action is picked is `motion.rs`, one more door down.
#![deny(unsafe_code)]

use super::keys::{self, Action};
use super::search::Dir;
use super::{Mode, Pager, HSTEP};
use crate::key::{Key, KeyEvent};
use crate::source::Anchor;
use crate::select::Yank;

impl Pager {
    // -- input --------------------------------------------------------------

    /// Feed one decoded key press.
    pub fn handle(&mut self, ev: KeyEvent) {
        self.dirty = true;
        // Any key abandons a `G` scan, and is then acted on normally — which
        // is what makes the scan interruptible rather than a freeze the reader
        // has to sit through (SPEC.md §CSV).
        if self.stop_seek() {
            self.notify("scan stopped");
        }
        // Ctrl-C is the escape hatch and must work from every mode, including
        // the overlays and while a search query is being typed, so it is
        // resolved before any mode gets a look at the key. It still goes
        // through the binding table, which stays the single source of truth.
        if keys::lookup(None, ev) == Some(Action::ForceQuit) {
            return self.act(Action::ForceQuit);
        }
        if let Mode::Search(dir) = self.mode {
            self.search_key(ev, dir);
            return;
        }
        if self.mode == Mode::Index {
            self.index_key(ev);
            return;
        }
        if self.mode != Mode::Normal && self.overlay_key(ev) {
            return;
        }
        let prefix = self.pending.take();
        if prefix.is_none() && keys::is_prefix(ev) {
            if let Key::Char(c) = ev.key {
                self.pending = Some(c);
            }
            return;
        }
        match keys::lookup(prefix, ev) {
            Some(a) => self.act(a),
            None => {
                if ev.key == Key::Esc {
                    match self.select.is_some() {
                        true => self.cancel_visual(),
                        false => self.clear_search(),
                    }
                }
            }
        }
    }

    /// Yank commands are their own thing; everything else may move the cursor,
    /// and a live visual selection follows it.
    pub(super) fn act(&mut self, a: Action) {
        match a {
            Action::Visual => return self.toggle_visual(),
            Action::Yank => return self.yank_selection(),
            Action::YankSection => return self.yank_section(),
            Action::YankCode => return self.yank_code(),
            _ => {}
        }
        self.motion(a);
        if let Some(s) = &mut self.select {
            s.set_head(self.cursor);
        }
    }

    /// Cursor and viewport movement. Corpus navigation lives in
    /// [`Self::nav_action`], folding and overlays in [`Self::view_action`];
    /// the three-way split is what keeps each under the 50-line limit.
    fn motion(&mut self, a: Action) {
        let h = self.content_rows().max(1);
        match a {
            Action::LineDown => self.move_cursor(1),
            Action::LineUp => self.move_cursor(-1),
            Action::HalfDown => self.move_cursor((h as isize + 1) / 2),
            Action::HalfUp => self.move_cursor(-((h as isize + 1) / 2)),
            Action::PageDown => self.move_cursor(h as isize),
            Action::PageUp => self.move_cursor(-(h as isize)),
            Action::Top => self.goto(0),
            Action::Bottom => self.goto_end(),
            Action::ScrollLeft => self.scroll_h(-(HSTEP as isize)),
            Action::ScrollRight => self.scroll_h(HSTEP as isize),
            Action::ArrowLeft => self.arrow(false),
            Action::ArrowRight => self.arrow(true),
            Action::Widen => self.widen(),
            Action::ToggleHidden => match self.src.toggle_hidden() {
                Some(msg) => {
                    self.notify(msg);
                    self.clamp();
                }
                None => self.notify("nothing hidden here"),
            },
            other => self.nav_action(other),
        }
    }

    /// Quitting, following links and walking the document history.
    fn nav_action(&mut self, a: Action) {
        match a {
            // SPEC.md §Keybindings: `q` pops the nav stack first if deep.
            Action::Quit => match self.can_pop() {
                true => self.go_back(),
                false => self.quit = true,
            },
            Action::ForceQuit => self.quit = true,
            Action::Follow => self.follow(),
            Action::Back => self.go_back(),
            Action::Forward => self.go_forward(),
            Action::OpenIndex => self.open_index(),
            Action::NextDoc => self.step_doc(1),
            Action::PrevDoc => self.step_doc(-1),
            other => self.view_action(other),
        }
    }

    /// Folding, overlays and search: the half of the action table that changes
    /// what is on screen rather than where the cursor is. Split out of
    /// [`Self::motion`] only to keep both under the 50-line limit.
    fn view_action(&mut self, a: Action) {
        match a {
            Action::ToggleCollapse => self.fold(None),
            Action::OpenTree => self.open_tree(),
            Action::OpenSection => self.fold(Some(false)),
            Action::CloseSection => self.fold(Some(true)),
            Action::CollapseAll => self.fold_all(true),
            Action::ExpandAll => self.fold_all(false),
            Action::NextHeading => self.jump_heading(true),
            Action::PrevHeading => self.jump_heading(false),
            Action::Outline => self.open_outline(),
            Action::Help => {
                self.mode = Mode::Help;
                self.help_top = 0;
            }
            Action::Reveal => {
                self.reveal = !self.reveal;
                self.notify(match self.reveal {
                    true => "hidden characters shown (I hides them)",
                    false => "hidden characters hidden",
                });
            }
            Action::SearchForward => self.start_search(Dir::Forward),
            Action::SearchBackward => self.start_search(Dir::Backward),
            // With no live search, `n`/`N` walk the document's links instead.
            Action::NextMatch => match self.query.is_empty() {
                true => self.step_link(true),
                false => self.cycle(self.dir),
            },
            Action::PrevMatch => match self.query.is_empty() {
                true => self.step_link(false),
                false => self.cycle(self.dir.flip()),
            },
            // Handled by `act`, or already matched in `motion`.
            _ => {}
        }
    }

    // -- folding ------------------------------------------------------------

    /// `want` = `None` toggles, `Some(true)` closes, `Some(false)` opens the
    /// section at or above the cursor.
    pub(super) fn fold(&mut self, want: Option<bool>) {
        if self.len() == 0 {
            return;
        }
        // Where the reader is looking, taken before anything moves. Folding
        // renumbers the rows under it, so the line the cursor is on is a place
        // in the document, not a row index.
        let here = self.cursor_anchor();
        // A format with regions of its own answers first: code folds the branch
        // the cursor is in, which is not an outline entry and could not be
        // reached through one (`Source::fold_here`).
        if want.is_none() {
            if let Some(folded) = self.src.fold_here(self.cursor) {
                self.folds_changed();
                self.stay_at(here, folded.marker);
                // The screen changed, so whatever the status bar was still
                // saying about the last command is now answering for this one.
                self.clear_message();
                return;
            }
        }
        let entry = match self.src.section_at(self.cursor) {
            Some(e) => e,
            None => return self.notify("no heading here"),
        };
        let (anchor, is_closed) = match self.src.outline().get(entry) {
            Some(e) => (e.anchor, e.folded),
            None => return,
        };
        let close = want.unwrap_or(!is_closed);
        if !self.src.set_fold(entry, close) {
            return;
        }
        self.folds_changed();
        let head = self.src.row_of(anchor);
        // Closing is the case that can take the cursor's line away, and the row
        // the section now answers on is not always its first.
        let marker = match close {
            true => head.map(|h| self.count_row(entry, h)),
            false => head,
        };
        self.stay_at(here, marker);
    }

    /// Put the cursor back where the reader left it.
    ///
    /// Folding a section must not scroll the reader somewhere else: pressing
    /// the key on a signature and landing three lines up on the first line of
    /// its doc comment is a jump nobody asked for. The cursor stays on the same
    /// *line*, whatever row number that line now has.
    ///
    /// It moves for one reason only — that line has just been folded away —
    /// and then to the row the fold answers on, which is the one place the same
    /// key reopens it from.
    fn stay_at(&mut self, here: Option<Anchor>, marker: Option<usize>) {
        // Nothing to aim at leaves the cursor to `folds_changed`'s clamp, which
        // has already kept it inside the document.
        if let Some(row) = here.and_then(|a| self.src.row_of(a)).or(marker) {
            self.jump_to_row(row);
        }
    }

    /// The row a shut section prints its `(N lines)` on, searched from its head.
    ///
    /// The two models disagree and the pager stays out of it: prose puts the
    /// count on the heading, so the answer is the head itself; code puts it on
    /// the signature, which is the *last* row the declaration keeps — landing on
    /// the head there means the top of a doc comment that can be fifteen lines
    /// long. Asking which row carries the count is the one question that gets
    /// the right answer from both without naming either.
    fn count_row(&self, entry: usize, head: usize) -> usize {
        let mut row = head;
        while row < self.len() && self.src.section_at(row) == Some(entry) {
            if self.src.hidden_at(row).is_some() {
                return row;
            }
            row += 1;
        }
        head
    }

    fn fold_all(&mut self, close: bool) {
        let here = self.cursor_anchor();
        let section = self
            .src
            .section_at(self.cursor)
            .and_then(|e| self.src.outline().get(e))
            .map(|e| e.anchor);
        self.src.fold_all(close);
        self.folds_changed();
        if let Some(a) = here {
            let target = match close {
                true => section.unwrap_or(a),
                false => a,
            };
            self.jump_to(target);
        }
        self.notify(match close {
            true => "all sections collapsed",
            false => "all sections expanded",
        });
    }

    // -- overlays -----------------------------------------------------------

    fn open_outline(&mut self) {
        if self.src.outline().is_empty() {
            return self.notify("no headings");
        }
        self.mode = Mode::Outline;
        let here = self.cursor_anchor().unwrap_or(Anchor(0));
        self.outline_sel = self
            .src
            .outline()
            .iter()
            .rposition(|e| e.anchor <= here)
            .unwrap_or(0);
    }

    /// Keys consumed by the outline/help overlays. Returns true when handled.
    fn overlay_key(&mut self, ev: KeyEvent) -> bool {
        match (self.mode, ev.key) {
            (_, Key::Esc) | (_, Key::Char('q')) => {
                self.mode = Mode::Normal;
                true
            }
            (Mode::Help, Key::Char('j')) | (Mode::Help, Key::Down) => {
                self.help_top = (self.help_top + 1).min(keys::BINDINGS.len().saturating_sub(1));
                true
            }
            (Mode::Help, Key::Char('k')) | (Mode::Help, Key::Up) => {
                self.help_top = self.help_top.saturating_sub(1);
                true
            }
            (Mode::Help, _) => {
                self.mode = Mode::Normal;
                true
            }
            (Mode::Outline, _) => self.outline_key(ev),
            (Mode::Detail, _) => self.detail_key(ev),
            _ => false,
        }
    }

    /// The row-detail overlay's own keys: scroll the field list, nothing else.
    /// Esc and `q` are handled above, for every overlay at once.
    fn detail_key(&mut self, ev: KeyEvent) -> bool {
        let page = self.body_rows().max(1);
        let last = match &self.detail {
            Some(d) => d.fields.len().saturating_sub(1),
            None => return false,
        };
        match ev.key {
            Key::Char('j') | Key::Down => self.detail_sel = (self.detail_sel + 1).min(last),
            Key::Char('k') | Key::Up => self.detail_sel = self.detail_sel.saturating_sub(1),
            Key::Char('g') => self.detail_sel = 0,
            Key::Char('G') => self.detail_sel = last,
            Key::PageDown => self.detail_sel = (self.detail_sel + page).min(last),
            Key::PageUp => self.detail_sel = self.detail_sel.saturating_sub(page),
            Key::Char('y') => self.yank_field(),
            _ => return false,
        }
        true
    }

    /// `y` in the row detail copies the highlighted *field*.
    ///
    /// The value verbatim, not re-quoted: in the form you are reading a value,
    /// not exporting a record, so what lands on the clipboard is what is on the
    /// line. `Y` on the grid is still there for the whole row as valid CSV.
    fn yank_field(&mut self) {
        let Some(d) = &self.detail else { return };
        let Some((label, value)) = d.fields.get(self.detail_sel) else {
            return;
        };
        if value.is_empty() {
            let empty = format!("{label} is empty");
            return self.notify(empty);
        }
        let yank = Yank {
            text: format!("{value}\n"),
            what: format!("{label} \u{b7} {}", d.title.to_lowercase()),
        };
        self.queue_yank(yank);
    }

    /// The outline overlay's own keys. An unhandled key falls through to the
    /// normal dispatcher, exactly as it did when this was one match arm.
    fn outline_key(&mut self, ev: KeyEvent) -> bool {
        let page = self.body_rows().max(1);
        let last = self.src.outline().len().saturating_sub(1);
        match ev.key {
            Key::Char('j') | Key::Down => self.outline_sel = (self.outline_sel + 1).min(last),
            Key::Char('k') | Key::Up => self.outline_sel = self.outline_sel.saturating_sub(1),
            Key::Char('g') => self.outline_sel = 0,
            Key::Char('G') => self.outline_sel = last,
            Key::PageDown => self.outline_sel = (self.outline_sel + page).min(last),
            Key::PageUp => self.outline_sel = self.outline_sel.saturating_sub(page),
            Key::Enter => self.follow_outline(),
            _ => return false,
        }
        true
    }

    fn follow_outline(&mut self) {
        self.mode = Mode::Normal;
        if let Some(target) = self.src.outline().get(self.outline_sel).map(|e| e.anchor) {
            self.jump_to(target);
        }
    }

    // -- search -------------------------------------------------------------

    fn start_search(&mut self, dir: Dir) {
        self.mode = Mode::Search(dir);
        self.dir = dir;
        self.query.clear();
        self.src.set_query("");
        self.search_origin = self.cursor_anchor();
    }

    fn search_key(&mut self, ev: KeyEvent, dir: Dir) {
        match ev.key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.clear_search();
                self.jump_to_origin();
            }
            Key::Enter => {
                self.mode = Mode::Normal;
                if self.src.match_count() == 0 && !self.query.is_empty() {
                    self.notify(format!("pattern not found: {}", self.query));
                }
            }
            Key::Backspace => {
                if self.query.pop().is_none() {
                    self.mode = Mode::Normal;
                }
                self.rescan();
                self.preview(dir);
            }
            Key::Char(c) => {
                self.query.push(c);
                self.rescan();
                self.preview(dir);
            }
            _ => {}
        }
    }

    /// Hand the live query to the source, which owns the match list.
    pub(super) fn rescan(&mut self) {
        self.src.set_query(&self.query);
    }

    fn jump_to_origin(&mut self) {
        if let Some(a) = self.search_origin {
            self.jump_to(a);
        }
    }

    /// Incremental jump while typing: first hit at or after the origin.
    fn preview(&mut self, dir: Dir) {
        if self.query.is_empty() {
            self.jump_to_origin();
            return;
        }
        let origin = match self.search_origin {
            Some(a) => a,
            None => return,
        };
        if let Some(hit) = self.src.preview_match(origin, dir) {
            self.jump_to(hit.anchor);
        }
    }

    fn cycle(&mut self, dir: Dir) {
        if self.query.is_empty() {
            return self.notify("no previous search");
        }
        if self.src.match_count() == 0 {
            return self.notify(format!("pattern not found: {}", self.query));
        }
        let from = self.cursor_anchor().unwrap_or(Anchor(0));
        if let Some(hit) = self.src.cycle_match(from, dir) {
            self.jump_to(hit.anchor);
            if hit.wrapped {
                self.notify(match dir {
                    Dir::Forward => "search hit bottom, continuing at top",
                    Dir::Backward => "search hit top, continuing at bottom",
                });
            }
        }
    }

    fn clear_search(&mut self) {
        self.query.clear();
        self.src.set_query("");
    }
}
