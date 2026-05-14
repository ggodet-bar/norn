use std::collections::VecDeque;

use ansi_to_tui::IntoText;
use ratatui::text::Line;
use splog::log_view_state::{LogViewState, SearchState, ViewState};

use crate::capture::LogLine;

pub struct App {
    /// `VecDeque` rather than `Vec` so the front-trim at `max_lines`
    /// capacity is O(1) (head-pointer advance) instead of an O(N)
    /// memmove of the entire buffer on every overflowing push.
    pub rendered: VecDeque<Line<'static>>,
    /// 0 = main "all" view, 1..=N = categories[N-1].
    pub selected: usize,
    /// Render an input-line-number gutter in the log pane.
    pub show_line_numbers: bool,
    /// Input line number flagged by the most recent successful goto. The
    /// run loop clears this on the next user keypress so the highlight
    /// disappears as soon as the user moves on.
    pub goto_highlight: Option<usize>,

    pub max_lines: Option<usize>,
    pub log_view_state: LogViewState,
}

impl App {
    pub fn new(max_lines: Option<usize>, display_follow: bool) -> Self {
        Self {
            rendered: VecDeque::new(),
            selected: 0,
            show_line_numbers: false,
            goto_highlight: None,
            max_lines,
            log_view_state: LogViewState::new(max_lines, display_follow),
        }
    }

    pub fn push(&mut self, line: LogLine) {
        let parsed = line
            .raw
            .as_bytes()
            .into_text()
            .map(|t| t.lines)
            .unwrap_or_else(|_| vec![Line::raw(line.raw.clone())]);
        self.rendered.extend(parsed);
        self.log_view_state.push(line.raw);
        if let Some(limit) = self.max_lines {
            let len = self.rendered.len();
            if len > limit {
                let drop = len - limit;
                self.rendered.drain(..drop);
            }
        }
    }

    pub fn active_view_mut(&mut self) -> (&mut ViewState, usize) {
        if self.selected == 0 {
            let total = self.rendered.len();
            (self.log_view_state.main_view_mut(), total)
        } else {
            let cat = self.log_view_state.get_category_mut(self.selected - 1);
            let total = cat.indices_count();
            (cat.view_mut(), total)
        }
    }

    pub fn active_search(&self) -> &SearchState {
        self.log_view_state.get_search(self.selected)
    }

    fn active_search_mut(&mut self) -> &mut SearchState {
        self.log_view_state.active_search_mut(self.selected)
    }

    pub fn scroll_up(&mut self, n: usize) {
        let (v, _) = self.active_view_mut();
        v.scroll_up(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport: usize) {
        let (v, total) = self.active_view_mut();
        v.scroll_down(n, total, viewport);
    }

    pub fn scroll_left(&mut self, n: usize) {
        let (v, _) = self.active_view_mut();
        v.scroll_left(n);
    }

    pub fn scroll_right(&mut self, n: usize) {
        let (v, _) = self.active_view_mut();
        v.scroll_right(n);
    }

    pub fn scroll_top(&mut self) {
        let (v, _) = self.active_view_mut();
        v.scroll_top();
    }

    pub fn scroll_bottom(&mut self) {
        let (v, _) = self.active_view_mut();
        v.scroll_bottom();
    }

    pub fn next_tab(&mut self) {
        self.selected = (self.selected + 1) % (self.log_view_state.category_count() + 1);
    }

    pub fn prev_tab(&mut self) {
        let n = self.log_view_state.category_count() + 1;
        self.selected = (self.selected + n - 1) % n;
    }

    pub fn select_tab(&mut self, idx: usize) {
        if idx <= self.log_view_state.category_count() {
            self.selected = idx;
        }
    }

    /// Drop the currently-selected category and remember its name so future
    /// pushes don't promote it again. No-op when the "all" pane is active.
    /// Selection moves one tab to the left so the user lands on the pane
    /// that sat next to the one they hid.
    pub fn ignore_active_category(&mut self) {
        if self.selected == 0 {
            return;
        }
        let idx = self.selected - 1;
        self.selected -= 1;
        self.log_view_state.remove_category_at_idx(idx);
    }

    /// Advance to the next match, wrapping. Returns the pane-local row of
    /// the new current match.
    pub fn search_next(&mut self) -> Option<usize> {
        let s = self.active_search_mut();
        s.search_next()
    }

    pub fn search_prev(&mut self) -> Option<usize> {
        let s = self.active_search_mut();
        s.search_prev()
    }

    pub fn clear_search(&mut self) {
        self.log_view_state.clear_search(self.selected);
    }

    pub fn commit_search(&mut self, raw: &str, is_regex: bool) -> Result<usize, regex::Error> {
        self.log_view_state
            .commit_search(raw, is_regex, self.selected)
    }

    pub fn promote_search_to_category(
        &mut self,
        raw: &str,
        is_regex: bool,
    ) -> Result<bool, regex::Error> {
        if let Some(new_idx) = self
            .log_view_state
            .promote_search_to_category(raw, is_regex)?
        {
            self.selected = new_idx;
            return Ok(true);
        }
        Ok(false)
    }

    /// Map an input line number (as shown in the gutter) to a pane-local row
    /// in the active pane. Returns `None` when the pane is empty. When `target`
    /// is not present in the pane, returns the row with the closest input
    /// line number; ties prefer the earlier row. Also stores the matching
    /// input line in `goto_highlight` so the renderer can flag it.
    pub fn goto_input_line(&mut self, target: usize) -> Option<usize> {
        let row = if self.selected == 0 {
            closest_row_by(
                self.log_view_state.line_numbers().len(),
                |i| self.log_view_state.line_numbers()[i],
                target,
            )
        } else {
            let cat = &self.log_view_state.get_category(self.selected - 1);
            closest_row_by(
                cat.indices_count(),
                |i| self.log_view_state.line_numbers()[cat.index(i)],
                target,
            )
        }?;
        let actual = if self.selected == 0 {
            self.log_view_state.line_numbers()[row]
        } else {
            self.log_view_state.line_numbers()[self
                .log_view_state
                .get_category(self.selected - 1)
                .index(row)]
        };
        self.goto_highlight = Some(actual);
        Some(row)
    }

    pub fn clear_goto_highlight(&mut self) {
        self.goto_highlight = None;
    }
}

/// Find the row in `0..len` whose `key(row)` is closest to `target`. The
/// sequence produced by `key` must be non-decreasing. On exact match, the
/// first matching row wins; on ties between neighbours, the lower row wins.
fn closest_row_by<F: Fn(usize) -> usize>(len: usize, key: F, target: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    // Binary search for the first row whose key is >= target.
    let mut lo = 0;
    let mut hi = len;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if key(mid) < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let pos = lo;
    if pos < len && key(pos) == target {
        return Some(pos);
    }
    match (pos.checked_sub(1), pos < len) {
        (Some(prev), true) => {
            let d_prev = target - key(prev);
            let d_next = key(pos) - target;
            if d_next < d_prev {
                Some(pos)
            } else {
                Some(prev)
            }
        }
        (Some(prev), false) => Some(prev),
        (None, true) => Some(pos),
        (None, false) => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn push_lines(app: &mut App, lines: &[&str]) {
        for line in lines {
            app.push(LogLine {
                raw: (*line).to_string(),
            });
        }
    }

    #[test]
    fn ignore_on_all_pane_is_noop() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["foo"]);
        app.ignore_active_category();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn goto_returns_none_on_empty_pane() {
        let mut app = App::new(None, true);
        assert_eq!(app.goto_input_line(1), None);
        assert_eq!(app.goto_highlight, None);
    }

    #[test]
    fn search_next_and_prev_cycle() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["a", "a", "a"]);
        app.commit_search("a", false).unwrap();
        assert_eq!(app.log_view_state.get_search(0).current(), Some(0));
        assert_eq!(app.search_next(), Some(1));
        assert_eq!(app.search_next(), Some(2));
        assert_eq!(app.search_next(), Some(0));
        assert_eq!(app.search_prev(), Some(2));
    }

    #[test]
    fn trim_keeps_current_when_above_drop_cutoff() {
        let mut app = App::new(Some(3), true);
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        app.search_next();
        app.search_next();
        assert_eq!(app.log_view_state.get_search(0).current(), Some(2));
        push_lines(&mut app, &["foo"]);
        // Oldest match dropped; current slides from 2 -> 1.
        assert_eq!(app.log_view_state.get_search(0).current(), Some(1));
    }

    #[test]
    fn ignore_active_category_drops_pane_and_blocks_reappearance() {
        let mut app = App::new(None, true);
        // Burst-promote "[db]" so we have a category pane to ignore.
        push_lines(&mut app, &["[db] a", "[db] b", "[db] c"]);
        assert_eq!(app.log_view_state.category_count(), 1);
        let name = app.log_view_state.get_category(0).name().to_owned();
        app.selected = 1;
        app.ignore_active_category();
        assert_eq!(app.log_view_state.category_count(), 0);
        // Selection slides one tab left — from the only category back to "all".
        assert_eq!(app.selected, 0);
        // Subsequent matching lines must not re-promote the category.
        for _ in 0..20 {
            push_lines(&mut app, &[&format!("[{name}] again")]);
        }
        assert_eq!(app.log_view_state.category_count(), 0);
    }

    #[test]
    fn ignore_active_category_lands_on_left_neighbor() {
        let mut app = App::new(None, true);
        // Two distinct burst-promoted categories.
        push_lines(
            &mut app,
            &[
                "[db] 1", "[db] 2", "[db] 3", "[auth] 1", "[auth] 2", "[auth] 3",
            ],
        );
        assert_eq!(app.log_view_state.category_count(), 2);
        // Select the second category and ignore it; selection should fall
        // back to the first category (tab index 1), not "all".
        app.selected = 2;
        app.ignore_active_category();
        assert_eq!(app.log_view_state.category_count(), 1);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn goto_exact_match_in_all_pane() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["one", "two", "three"]);
        assert_eq!(app.goto_input_line(2), Some(1));
        assert_eq!(app.goto_highlight, Some(2));
        app.clear_goto_highlight();
        assert_eq!(app.goto_highlight, None);
    }

    #[test]
    fn goto_clamps_above_max_in_all_pane() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["one", "two", "three"]);
        assert_eq!(app.goto_input_line(99), Some(2));
    }

    #[test]
    fn goto_clamps_below_min_in_all_pane() {
        let mut app = App::new(Some(2), true);
        push_lines(&mut app, &["one", "two", "three", "four"]);
        // Oldest survivor is line 3 (input_seq starts at 1).
        assert_eq!(app.goto_input_line(1), Some(0));
    }

    #[test]
    fn goto_in_category_pane_picks_closest() {
        let mut app = App::new(None, true);
        // Burst-promote `[db]`, then push lines that bypass it so the
        // category's input line numbers skip values.
        push_lines(&mut app, &["[db] 1", "[db] 2", "[db] 3"]);
        assert_eq!(app.log_view_state.category_count(), 1);
        push_lines(&mut app, &["plain a", "plain b", "[db] 4"]);
        app.selected = 1;
        // The pane's input line numbers are [1, 2, 3, 6]. Target 5 sits
        // closer to 6 (distance 1) than 3 (distance 2).
        assert_eq!(app.goto_input_line(5), Some(3));
        // Tie: target 4 is equidistant from 3 and 6; the earlier row wins.
        assert_eq!(app.goto_input_line(4), Some(2));
        // Highlight follows the actual line we landed on, not the typed target.
        assert_eq!(app.goto_highlight, Some(3));
        // Exact match.
        assert_eq!(app.goto_input_line(2), Some(1));
        assert_eq!(app.goto_highlight, Some(2));
    }

    #[test]
    fn promote_search_overrides_ignored_name() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["[db] 1", "[db] 2", "[db] 3"]);
        let name = app.log_view_state.get_category(0).name().to_owned();
        app.selected = 1;
        app.ignore_active_category();
        assert_eq!(app.log_view_state.category_count(), 0);
        // Explicit promotion bypasses the ignore-list and re-creates the pane.
        assert!(app.promote_search_to_category(&name, false).unwrap());
        assert_eq!(app.log_view_state.category_count(), 1);
    }

    #[test]
    fn promote_search_existing_name_just_switches() {
        let mut app = App::new(None, true);
        // Burst-promote `[db]` via the tag path.
        push_lines(&mut app, &["[db] 1", "[db] 2", "[db] 3"]);
        assert_eq!(app.log_view_state.category_count(), 1);
        let name = app.log_view_state.get_category(0).name().to_owned();
        let before = app.log_view_state.get_category(0).indices().to_vec();
        app.selected = 0;
        assert!(app.promote_search_to_category(&name, false).unwrap());
        assert_eq!(app.log_view_state.category_count(), 1);
        assert_eq!(app.log_view_state.get_category(0).indices(), before);
        // Switched to that pane.
        assert_eq!(app.selected, 1);
        // Did not retroactively pin a regex on the tag-extracted pane.
        assert!(app.log_view_state.get_category(0).match_regex().is_none());
    }

    #[test]
    fn no_matches_yields_none_current() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["abc", "def"]);
        assert_eq!(app.commit_search("zzz", false).unwrap(), 0);
        assert_eq!(app.log_view_state.get_search(0).current(), None);
        assert_eq!(app.search_next(), None);
    }
}
