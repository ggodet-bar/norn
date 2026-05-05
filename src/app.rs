use std::collections::{HashMap, VecDeque};

use ansi_to_tui::IntoText;
use ratatui::text::Line;
use regex::Regex;

use crate::capture::LogLine;
use crate::categorize;

/// A candidate must appear in this many distinct input lines before it gets a
/// real pane — keeps line-unique noise out of the tab strip.
const PROMOTION_THRESHOLD: usize = 12;
/// Fast-track: a candidate seen in `BURST_HITS` of the last `BURST_WINDOW`
/// input lines is a strong signal of a real category, so promote without
/// waiting for the full hit count.
const BURST_HITS: usize = 3;
const BURST_WINDOW: usize = 6;
/// A pending candidate that hasn't been seen for this many input lines is
/// dropped so memory doesn't grow with one-off tags.
const PENDING_EVICTION_AGE: usize = 200;

pub struct ViewState {
    pub scroll: usize,
    pub follow: bool,
}

impl ViewState {
    fn new() -> Self {
        Self { scroll: 0, follow: true }
    }

    fn scroll_up(&mut self, n: usize) {
        self.follow = false;
        self.scroll = self.scroll.saturating_sub(n);
    }

    fn scroll_down(&mut self, n: usize, total: usize, viewport: usize) {
        let max = total.saturating_sub(viewport);
        self.scroll = (self.scroll + n).min(max);
        if self.scroll >= max {
            self.follow = true;
        }
    }
}

pub struct Category {
    pub name: String,
    /// Strictly increasing row indices into `App::rendered`.
    pub indices: Vec<usize>,
    pub view: ViewState,
    pub search: SearchState,
}

/// Compiled query backing a `SearchState`. Literal queries are stored after
/// `regex::escape` so the matcher path is single-source.
pub struct CompiledQuery {
    pub regex: Regex,
    pub raw: String,
    pub is_regex: bool,
}

/// One match within a single rendered row. `start`/`end` are byte offsets
/// into the row's plain (un-styled) text.
#[derive(Clone, Copy, Debug)]
pub struct RowMatch {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Per-pane search state. Matches stay sorted by `(row, start)` so
/// next/prev navigation is a simple index walk.
#[derive(Default)]
pub struct SearchState {
    pub query: Option<CompiledQuery>,
    pub matches: Vec<RowMatch>,
    pub current: Option<usize>,
}

struct PendingCategory {
    /// Number of distinct input lines that mentioned this candidate.
    hits: usize,
    /// Row indices in `rendered` for the lines that have mentioned it. Kept
    /// in sync with trims so promotion produces correct refs.
    rows: Vec<usize>,
    /// `App::input_seq` of the most recent mention, used for eviction.
    last_seen_seq: usize,
    /// `input_seq` of the most recent up-to-`BURST_HITS` mentions, oldest
    /// first. Used to detect bursts (`BURST_HITS` hits within `BURST_WINDOW`
    /// input lines) for the fast-track promotion path.
    recent_seqs: VecDeque<usize>,
}

pub struct App {
    pub rendered: Vec<Line<'static>>,
    pub main: ViewState,
    pub main_search: SearchState,
    pub max_lines: Option<usize>,
    pub categories: Vec<Category>,
    category_index: HashMap<String, usize>,
    pending: HashMap<String, PendingCategory>,
    /// Monotonic input-line counter; not affected by trimming.
    input_seq: usize,
    /// 0 = main "all" view, 1..=N = categories[N-1].
    pub selected: usize,
}

impl App {
    pub fn new(max_lines: Option<usize>) -> Self {
        Self {
            rendered: Vec::new(),
            main: ViewState::new(),
            main_search: SearchState::default(),
            max_lines,
            categories: Vec::new(),
            category_index: HashMap::new(),
            pending: HashMap::new(),
            input_seq: 0,
            selected: 0,
        }
    }

    pub fn push(&mut self, line: LogLine) {
        let cats = categorize::extract(&line.raw);

        let parsed = line
            .raw
            .as_bytes()
            .into_text()
            .map(|t| t.lines)
            .unwrap_or_else(|_| vec![Line::raw(line.raw)]);
        let start = self.rendered.len();
        self.rendered.extend(parsed);
        let end = self.rendered.len();

        self.input_seq += 1;
        let seq = self.input_seq;

        for cat_name in cats {
            if let Some(&idx) = self.category_index.get(&cat_name) {
                self.categories[idx].indices.extend(start..end);
                self.scan_new_category_rows(idx, end - start);
                continue;
            }

            let promote = {
                let entry = self.pending.entry(cat_name.clone()).or_insert_with(|| {
                    PendingCategory {
                        hits: 0,
                        rows: Vec::new(),
                        last_seen_seq: seq,
                        recent_seqs: VecDeque::with_capacity(BURST_HITS),
                    }
                });
                if entry.recent_seqs.len() == BURST_HITS {
                    entry.recent_seqs.pop_front();
                }
                entry.recent_seqs.push_back(seq);
                entry.hits += 1;
                entry.rows.extend(start..end);
                entry.last_seen_seq = seq;
                let burst = entry.recent_seqs.len() == BURST_HITS
                    && seq - entry.recent_seqs.front().copied().unwrap() < BURST_WINDOW;
                entry.hits >= PROMOTION_THRESHOLD || burst
            };

            if promote {
                let pending = self.pending.remove(&cat_name).expect("just promoted");
                let idx = self.categories.len();
                self.category_index.insert(cat_name.clone(), idx);
                self.categories.push(Category {
                    name: cat_name,
                    indices: pending.rows,
                    view: ViewState::new(),
                    search: SearchState::default(),
                });
            }
        }

        self.scan_new_main_rows(start, end);

        // Periodic eviction: drop pending candidates we haven't seen in a
        // while. Cheap enough to do every push when `pending` is small.
        self.pending
            .retain(|_, p| seq.saturating_sub(p.last_seen_seq) <= PENDING_EVICTION_AGE);

        if let Some(limit) = self.max_lines {
            let len = self.rendered.len();
            if len > limit {
                let drop = len - limit;
                self.rendered.drain(..drop);
                self.main.scroll = self.main.scroll.saturating_sub(drop);
                // Pane rows for "all" are 1:1 with rendered rows.
                adjust_search_after_drop(&mut self.main_search, drop);

                for cat in &mut self.categories {
                    let dropped_here = cat.indices.iter().take_while(|&&i| i < drop).count();
                    cat.indices.drain(..dropped_here);
                    for i in &mut cat.indices {
                        *i -= drop;
                    }
                    cat.view.scroll = cat.view.scroll.saturating_sub(dropped_here);
                    adjust_search_after_drop(&mut cat.search, dropped_here);
                }
                for p in self.pending.values_mut() {
                    let dropped_here = p.rows.iter().take_while(|&&i| i < drop).count();
                    p.rows.drain(..dropped_here);
                    for i in &mut p.rows {
                        *i -= drop;
                    }
                }
            }
        }
    }

    pub fn active_view_mut(&mut self) -> (&mut ViewState, usize) {
        if self.selected == 0 {
            let total = self.rendered.len();
            (&mut self.main, total)
        } else {
            let cat = &mut self.categories[self.selected - 1];
            let total = cat.indices.len();
            (&mut cat.view, total)
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        let (v, _) = self.active_view_mut();
        v.scroll_up(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport: usize) {
        let (v, total) = self.active_view_mut();
        v.scroll_down(n, total, viewport);
    }

    pub fn scroll_top(&mut self) {
        let (v, _) = self.active_view_mut();
        v.follow = false;
        v.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        let (v, _) = self.active_view_mut();
        v.follow = true;
    }

    pub fn next_tab(&mut self) {
        self.selected = (self.selected + 1) % (self.categories.len() + 1);
    }

    pub fn prev_tab(&mut self) {
        let n = self.categories.len() + 1;
        self.selected = (self.selected + n - 1) % n;
    }

    pub fn select_tab(&mut self, idx: usize) {
        if idx <= self.categories.len() {
            self.selected = idx;
        }
    }

    /// Compile `raw` and replace the active pane's search state. On success
    /// returns the number of matches found. Empty input clears the search.
    /// Pane rows are local (0-based within the active pane), so "all" rows
    /// map 1:1 with `rendered`, while category rows index `cat.indices`.
    pub fn commit_search(
        &mut self,
        raw: &str,
        is_regex: bool,
    ) -> Result<usize, regex::Error> {
        if raw.is_empty() {
            self.clear_search();
            return Ok(0);
        }
        let pattern = if is_regex { raw.to_string() } else { regex::escape(raw) };
        let regex = Regex::new(&pattern)?;
        let query = CompiledQuery { regex, raw: raw.to_string(), is_regex };

        let mut matches = Vec::new();
        if self.selected == 0 {
            for (pane_row, line) in self.rendered.iter().enumerate() {
                collect_matches(&query.regex, line, pane_row, &mut matches);
            }
        } else {
            let cat = &self.categories[self.selected - 1];
            for (pane_row, &r) in cat.indices.iter().enumerate() {
                collect_matches(&query.regex, &self.rendered[r], pane_row, &mut matches);
            }
        }

        let current = if matches.is_empty() { None } else { Some(0) };
        let new_state = SearchState { query: Some(query), matches, current };
        let count = new_state.matches.len();
        *self.active_search_mut() = new_state;
        Ok(count)
    }

    pub fn clear_search(&mut self) {
        *self.active_search_mut() = SearchState::default();
    }

    /// Advance to the next match, wrapping. Returns the pane-local row of
    /// the new current match.
    pub fn search_next(&mut self) -> Option<usize> {
        let s = self.active_search_mut();
        if s.matches.is_empty() {
            return None;
        }
        let next = match s.current {
            Some(i) => (i + 1) % s.matches.len(),
            None => 0,
        };
        s.current = Some(next);
        Some(s.matches[next].row)
    }

    pub fn search_prev(&mut self) -> Option<usize> {
        let s = self.active_search_mut();
        if s.matches.is_empty() {
            return None;
        }
        let prev = match s.current {
            Some(i) => (i + s.matches.len() - 1) % s.matches.len(),
            None => s.matches.len() - 1,
        };
        s.current = Some(prev);
        Some(s.matches[prev].row)
    }

    pub fn active_search(&self) -> &SearchState {
        if self.selected == 0 {
            &self.main_search
        } else {
            &self.categories[self.selected - 1].search
        }
    }

    fn active_search_mut(&mut self) -> &mut SearchState {
        if self.selected == 0 {
            &mut self.main_search
        } else {
            &mut self.categories[self.selected - 1].search
        }
    }

    /// Scan the last `new_count` rendered rows against `main_search` and
    /// append matches. No-op when there's no active query or no new rows.
    fn scan_new_main_rows(&mut self, start: usize, end: usize) {
        if self.main_search.query.is_none() || start == end {
            return;
        }
        let q = self.main_search.query.as_ref().unwrap();
        let mut new_matches = Vec::new();
        for pane_row in start..end {
            collect_matches(&q.regex, &self.rendered[pane_row], pane_row, &mut new_matches);
        }
        self.main_search.matches.extend(new_matches);
    }

    /// Scan the last `new_count` pane rows of `categories[cat_idx]` against
    /// that pane's search query. Caller must extend `cat.indices` first.
    fn scan_new_category_rows(&mut self, cat_idx: usize, new_count: usize) {
        if new_count == 0 {
            return;
        }
        let cat = &self.categories[cat_idx];
        let Some(q) = cat.search.query.as_ref() else { return };
        let total = cat.indices.len();
        let first = total.saturating_sub(new_count);
        let mut new_matches = Vec::new();
        for pane_row in first..total {
            let r = cat.indices[pane_row];
            collect_matches(&q.regex, &self.rendered[r], pane_row, &mut new_matches);
        }
        self.categories[cat_idx].search.matches.extend(new_matches);
    }
}

/// Concatenated text of all spans in `line`, ignoring style. Search runs
/// against this flat string so highlighting can later splice spans back in.
fn plain_text(line: &Line<'static>) -> String {
    let mut s = String::new();
    for span in &line.spans {
        s.push_str(&span.content);
    }
    s
}

fn collect_matches(regex: &Regex, line: &Line<'static>, pane_row: usize, out: &mut Vec<RowMatch>) {
    let plain = plain_text(line);
    for m in regex.find_iter(&plain) {
        out.push(RowMatch { row: pane_row, start: m.start(), end: m.end() });
    }
}

/// Adjust a `SearchState` after `dropped` pane rows are trimmed off the
/// front: drop matches whose row is below the cutoff and shift the rest.
/// `current` slides with the surviving matches; if it pointed at a
/// just-dropped match, it falls back to the new first match.
fn adjust_search_after_drop(state: &mut SearchState, dropped: usize) {
    if dropped == 0 || state.matches.is_empty() {
        return;
    }
    let drop_count = state.matches.iter().take_while(|m| m.row < dropped).count();
    state.matches.drain(..drop_count);
    for m in &mut state.matches {
        m.row -= dropped;
    }
    state.current = match state.current {
        Some(c) if c >= drop_count => Some(c - drop_count),
        Some(_) if !state.matches.is_empty() => Some(0),
        _ => None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::LogLine;

    fn push_lines(app: &mut App, lines: &[&str]) {
        for line in lines {
            app.push(LogLine { raw: (*line).to_string() });
        }
    }

    #[test]
    fn literal_search_finds_matches_in_main() {
        let mut app = App::new(None);
        push_lines(&mut app, &["foo bar", "no match", "another foo"]);
        let n = app.commit_search("foo", false).unwrap();
        assert_eq!(n, 2);
        assert_eq!(app.main_search.matches.len(), 2);
        assert_eq!(app.main_search.matches[0].row, 0);
        assert_eq!(app.main_search.matches[1].row, 2);
        assert_eq!(app.main_search.current, Some(0));
    }

    #[test]
    fn regex_search_finds_matches() {
        let mut app = App::new(None);
        push_lines(&mut app, &["abc 123", "xyz 7", "no digits here"]);
        let n = app.commit_search(r"\d+", true).unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn literal_search_treats_metacharacters_literally() {
        let mut app = App::new(None);
        push_lines(&mut app, &["a.b", "axb"]);
        // In literal mode, `.` matches a dot, not any char.
        assert_eq!(app.commit_search(".", false).unwrap(), 1);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let mut app = App::new(None);
        push_lines(&mut app, &["foo"]);
        assert!(app.commit_search("(", true).is_err());
    }

    #[test]
    fn empty_query_clears_search() {
        let mut app = App::new(None);
        push_lines(&mut app, &["foo"]);
        app.commit_search("foo", false).unwrap();
        assert!(app.main_search.query.is_some());
        app.commit_search("", false).unwrap();
        assert!(app.main_search.query.is_none());
        assert!(app.main_search.matches.is_empty());
    }

    #[test]
    fn search_next_and_prev_cycle() {
        let mut app = App::new(None);
        push_lines(&mut app, &["a", "a", "a"]);
        app.commit_search("a", false).unwrap();
        assert_eq!(app.main_search.current, Some(0));
        assert_eq!(app.search_next(), Some(1));
        assert_eq!(app.search_next(), Some(2));
        assert_eq!(app.search_next(), Some(0));
        assert_eq!(app.search_prev(), Some(2));
    }

    #[test]
    fn push_extends_active_search_matches() {
        let mut app = App::new(None);
        push_lines(&mut app, &["foo"]);
        app.commit_search("foo", false).unwrap();
        assert_eq!(app.main_search.matches.len(), 1);
        push_lines(&mut app, &["foo bar foo"]);
        assert_eq!(app.main_search.matches.len(), 3);
        assert_eq!(app.main_search.matches[1].row, 1);
        assert_eq!(app.main_search.matches[2].row, 1);
        assert_eq!(app.main_search.matches[2].start, 8);
    }

    #[test]
    fn trim_drops_and_shifts_main_matches() {
        let mut app = App::new(Some(2));
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        assert_eq!(app.rendered.len(), 2);
        assert_eq!(app.main_search.matches.len(), 2);
        push_lines(&mut app, &["foo"]);
        assert_eq!(app.rendered.len(), 2);
        assert_eq!(app.main_search.matches.len(), 2);
        assert_eq!(app.main_search.matches[0].row, 0);
        assert_eq!(app.main_search.matches[1].row, 1);
    }

    #[test]
    fn trim_keeps_current_when_above_drop_cutoff() {
        let mut app = App::new(Some(3));
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        app.search_next();
        app.search_next();
        assert_eq!(app.main_search.current, Some(2));
        push_lines(&mut app, &["foo"]);
        // Oldest match dropped; current slides from 2 -> 1.
        assert_eq!(app.main_search.current, Some(1));
    }

    #[test]
    fn trim_resets_current_when_it_was_dropped() {
        let mut app = App::new(Some(2));
        push_lines(&mut app, &["foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        // current = Some(0); push enough to drop the first match.
        push_lines(&mut app, &["foo", "foo"]);
        // Two oldest dropped; surviving matches shift; current falls back
        // to 0 (the new first match).
        assert_eq!(app.main_search.current, Some(0));
        assert_eq!(app.main_search.matches.len(), 2);
    }

    #[test]
    fn no_matches_yields_none_current() {
        let mut app = App::new(None);
        push_lines(&mut app, &["abc", "def"]);
        assert_eq!(app.commit_search("zzz", false).unwrap(), 0);
        assert_eq!(app.main_search.current, None);
        assert_eq!(app.search_next(), None);
    }
}
