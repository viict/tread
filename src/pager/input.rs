//! Input dispatch: key press -> [`Action`] -> state change.
//!
//! Split out of `mod.rs` to keep both files well under the size limit; these
//! are the same `Pager` methods, just living next door.
#![deny(unsafe_code)]

use super::collapse;
use super::keys::{self, Action};
use super::search::{self, Dir};
use super::{Mode, Pager, HSTEP};
use crate::key::{Key, KeyEvent};

impl Pager {
    // -- input --------------------------------------------------------------

    /// Feed one decoded key press.
    pub fn handle(&mut self, ev: KeyEvent) {
        self.dirty = true;
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
        let h = self.body_rows().max(1);
        match a {
            Action::LineDown => self.move_cursor(1),
            Action::LineUp => self.move_cursor(-1),
            Action::HalfDown => self.move_cursor((h as isize + 1) / 2),
            Action::HalfUp => self.move_cursor(-((h as isize + 1) / 2)),
            Action::PageDown => self.move_cursor(h as isize),
            Action::PageUp => self.move_cursor(-(h as isize)),
            Action::Top => self.goto(0),
            Action::Bottom => self.goto(self.visible.len().saturating_sub(1)),
            Action::ScrollLeft => self.scroll_h(-(HSTEP as isize)),
            Action::ScrollRight => self.scroll_h(HSTEP as isize),
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
        let n = self.visible.len();
        if n == 0 {
            return;
        }
        let max = n as isize - 1;
        let next = (self.cursor as isize).saturating_add(delta).clamp(0, max);
        self.cursor = next as usize;
        self.clamp();
    }

    fn goto(&mut self, index: usize) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = index.min(self.visible.len() - 1);
        self.clamp();
    }

    fn scroll_h(&mut self, delta: isize) {
        let max = self.max_hoff();
        let next = (self.hoff as isize).saturating_add(delta).clamp(0, max as isize);
        self.hoff = next as usize;
    }

    /// Move the cursor to a row of `lines`, expanding folds if it is hidden.
    pub(super) fn jump_to(&mut self, line: usize) {
        if collapse::reveal(&self.lines, &mut self.collapsed, line) {
            self.refresh_view();
        }
        match self.visible.binary_search(&line) {
            Ok(at) | Err(at) => self.cursor = at.min(self.visible.len().saturating_sub(1)),
        }
        let h = self.body_rows().max(1);
        if self.cursor < self.top || self.cursor >= self.top + h {
            self.top = self.cursor.saturating_sub(h / 3);
        }
        self.clamp();
    }

    fn jump_heading(&mut self, forward: bool) {
        let n = self.visible.len();
        let found = match forward {
            true => (self.cursor + 1..n).find(|i| self.is_heading(*i)),
            false => (0..self.cursor).rev().find(|i| self.is_heading(*i)),
        };
        match found {
            Some(i) => self.goto(i),
            None => self.notify(match forward {
                true => "no further heading",
                false => "no previous heading",
            }),
        }
    }

    fn is_heading(&self, visible_index: usize) -> bool {
        self.visible
            .get(visible_index)
            .map(|i| self.lines[*i].heading.is_some())
            .unwrap_or(false)
    }

    // -- folding ------------------------------------------------------------

    /// `want` = `None` toggles, `Some(true)` closes, `Some(false)` opens the
    /// heading at or above the cursor.
    pub(super) fn fold(&mut self, want: Option<bool>) {
        let line = match self.cursor_line() {
            Some(l) => l,
            None => return,
        };
        let head = match collapse::heading_at_or_above(&self.lines, line) {
            Some(h) => h,
            None => return self.notify("no heading here"),
        };
        let id = match &self.lines[head].heading {
            Some(h) => h.id.clone(),
            None => return,
        };
        let is_closed = self.collapsed.contains(&id);
        let close = want.unwrap_or(!is_closed);
        if close == is_closed {
            return;
        }
        if close {
            self.collapsed.push(id);
        } else {
            self.collapsed.retain(|c| *c != id);
        }
        self.refresh_view();
        self.jump_to(head);
    }

    fn fold_all(&mut self, close: bool) {
        let anchor = self.cursor_line();
        self.collapsed = match close {
            true => collapse::all_ids(&self.lines),
            false => Vec::new(),
        };
        self.refresh_view();
        if let Some(l) = anchor {
            let head = collapse::heading_at_or_above(&self.lines, l).unwrap_or(l);
            self.jump_to(if close { head } else { l });
        }
        self.notify(if close { "all sections collapsed" } else { "all sections expanded" });
    }

    // -- overlays -----------------------------------------------------------

    fn open_outline(&mut self) {
        if self.outline.is_empty() {
            return self.notify("no headings");
        }
        self.mode = Mode::Outline;
        let line = self.cursor_line().unwrap_or(0);
        self.outline_sel = self
            .outline
            .iter()
            .rposition(|h| h.index <= line)
            .unwrap_or(0);
    }

    /// Keys consumed by the outline/help overlays. Returns true when handled.
    fn overlay_key(&mut self, ev: KeyEvent) -> bool {
        let rows = self.body_rows().max(1);
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
            (Mode::Outline, Key::Char('j')) | (Mode::Outline, Key::Down) => {
                self.outline_sel = (self.outline_sel + 1).min(self.outline.len().saturating_sub(1));
                true
            }
            (Mode::Outline, Key::Char('k')) | (Mode::Outline, Key::Up) => {
                self.outline_sel = self.outline_sel.saturating_sub(1);
                true
            }
            (Mode::Outline, Key::Char('g')) => {
                self.outline_sel = 0;
                true
            }
            (Mode::Outline, Key::Char('G')) => {
                self.outline_sel = self.outline.len().saturating_sub(1);
                true
            }
            (Mode::Outline, Key::PageDown) => {
                self.outline_sel = (self.outline_sel + rows).min(self.outline.len().saturating_sub(1));
                true
            }
            (Mode::Outline, Key::PageUp) => {
                self.outline_sel = self.outline_sel.saturating_sub(rows);
                true
            }
            (Mode::Outline, Key::Enter) => {
                self.follow_outline();
                true
            }
            _ => false,
        }
    }

    fn follow_outline(&mut self) {
        self.mode = Mode::Normal;
        if let Some(h) = self.outline.get(self.outline_sel) {
            let target = h.index;
            self.jump_to(target);
        }
    }

    // -- search -------------------------------------------------------------

    fn start_search(&mut self, dir: Dir) {
        self.mode = Mode::Search(dir);
        self.dir = dir;
        self.query.clear();
        self.matches.clear();
        self.current = None;
        self.search_origin = self.cursor_line().unwrap_or(0);
    }

    fn search_key(&mut self, ev: KeyEvent, dir: Dir) {
        match ev.key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.clear_search();
                self.jump_to(self.search_origin);
            }
            Key::Enter => {
                self.mode = Mode::Normal;
                if self.matches.is_empty() && !self.query.is_empty() {
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

    /// Recompute matches for the current query (after an edit or a re-layout).
    pub(super) fn rescan(&mut self) {
        self.matches = search::find_all(&self.lines, &self.query);
        if self.matches.is_empty() {
            self.current = None;
        }
    }

    /// Incremental jump while typing: first hit at or after the origin.
    fn preview(&mut self, dir: Dir) {
        if self.query.is_empty() {
            self.current = None;
            self.jump_to(self.search_origin);
            return;
        }
        let start = self.search_origin;
        if let Some((i, _)) = search::seek(&self.matches, start, 0, dir, true) {
            self.current = Some(i);
            let line = self.matches[i].line;
            self.jump_to(line);
        }
    }

    fn cycle(&mut self, dir: Dir) {
        if self.query.is_empty() {
            return self.notify("no previous search");
        }
        if self.matches.is_empty() {
            return self.notify(format!("pattern not found: {}", self.query));
        }
        let (line, col) = match self.current.and_then(|i| self.matches.get(i)) {
            Some(m) => (m.line, m.start),
            None => (self.cursor_line().unwrap_or(0), 0),
        };
        if let Some((i, wrapped)) = search::seek(&self.matches, line, col, dir, false) {
            self.current = Some(i);
            let target = self.matches[i].line;
            self.jump_to(target);
            if wrapped {
                self.notify(match dir {
                    Dir::Forward => "search hit bottom, continuing at top",
                    Dir::Backward => "search hit top, continuing at bottom",
                });
            }
        }
    }

    fn clear_search(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current = None;
    }
}
