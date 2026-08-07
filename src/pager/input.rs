//! Input dispatch: key press -> [`Action`] -> state change.
//!
//! Split out of `mod.rs` to keep both files well under the size limit; these
//! are the same `Pager` methods, just living next door.
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

    // -- movement -----------------------------------------------------------

    fn move_cursor(&mut self, delta: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        let max = n as isize - 1;
        let next = (self.cursor as isize).saturating_add(delta).clamp(0, max);
        self.cursor = next as usize;
        self.clamp();
    }

    pub(super) fn goto(&mut self, row: usize) {
        if self.len() == 0 {
            return;
        }
        self.cursor = row.min(self.len() - 1);
        self.clamp();
    }

    /// `h` / `l`. A format may scroll by its own unit — a CSV moves a whole
    /// column, and its pinned header moves with the body because both are
    /// painted at this one offset (SPEC.md §CSV).
    ///
    /// The format is told the *viewport* width, the same `cols` the painter
    /// slices rows to and [`Pager::max_hoff`] measures against — not the layout
    /// width, which `--width` can set far wider than the terminal.
    pub(super) fn scroll_h(&mut self, delta: isize) {
        let max = self.max_hoff();
        let hoff = self.hoff;
        let view = self.cols.max(1);
        let next = match self.src.hscroll(hoff, delta.signum(), view) {
            Some(off) => off as isize,
            None => (hoff as isize).saturating_add(delta),
        };
        self.hoff = next.clamp(0, max as isize) as usize;
    }

    /// `w`: let the format widen whatever is under the cursor.
    fn widen(&mut self) {
        match self.src.widen() {
            Some(msg) => {
                self.notify(msg);
                self.clamp();
            }
            None => self.notify("nothing to widen here"),
        }
    }

    /// Move the cursor to a row that is already visible, scrolling it in.
    pub(super) fn jump_to_row(&mut self, row: usize) {
        self.cursor = row.min(self.len().saturating_sub(1));
        let h = self.content_rows().max(1);
        if self.cursor < self.top || self.cursor >= self.top + h {
            self.top = self.cursor.saturating_sub(h / 3);
        }
        self.clamp();
    }

    /// Move the cursor to a place in the document, unfolding whatever hides it.
    pub(super) fn jump_to(&mut self, anchor: Anchor) {
        match self.src.reveal(anchor) {
            Some(row) => self.jump_to_row(row),
            None => self.clamp(),
        }
    }

    fn jump_heading(&mut self, forward: bool) {
        match self.src.next_landmark(self.cursor, forward) {
            Some(row) => self.goto(row),
            None => self.notify(match forward {
                true => "no further heading",
                false => "no previous heading",
            }),
        }
    }

    // -- folding ------------------------------------------------------------

    /// `want` = `None` toggles, `Some(true)` closes, `Some(false)` opens the
    /// section at or above the cursor.
    pub(super) fn fold(&mut self, want: Option<bool>) {
        if self.len() == 0 {
            return;
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
        self.jump_to(anchor);
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
