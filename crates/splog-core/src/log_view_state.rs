//! This module handles the abstract representation of `splog`'s user interface: the panel
//! contents, their attached search queries and potential matches. It also handles the system
//! for promoting category candidates to valid categories that will have their own view.
use std::collections::{HashMap, HashSet, VecDeque};

use regex::Regex;

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

/// Handles scrolling-related data for a given displayed category, and whether the display should
/// follow additions to the bottom of the buffer.
pub struct ViewState {
    scroll: usize,
    /// Horizontal column offset of the body relative to its rendered column 0.
    /// Clamped at draw time to the longest visible line's width so the user
    /// can't scroll past the content.
    hscroll: usize,
    /// Defines whether the view should stick to the bottom of the buffer or move freely. On init,
    /// `false` will display the top of the buffer, `true` the bottom.
    follow: bool,
    /// When displaying a file without the `follow` option, hide the FOLLOW/PAUSE text.
    display_follow: bool,
}

impl ViewState {
    fn new(display_follow: bool) -> Self {
        Self {
            scroll: 0,
            hscroll: 0,
            follow: display_follow,
            display_follow,
        }
    }

    pub fn display_follow(&self) -> bool {
        self.display_follow
    }

    pub fn is_following(&self) -> bool {
        self.follow
    }

    pub fn follow_or_clamp(&mut self, max_scroll: usize) -> usize {
        if self.follow {
            self.scroll = max_scroll;
        } else {
            self.scroll = self.scroll.min(max_scroll);
        }
        self.scroll
    }

    pub fn scroll_to_row(&mut self, row: usize) {
        self.scroll = row;
        self.follow = false;
    }

    pub fn scroll_top(&mut self) {
        self.follow = false;
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.follow = true;
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.follow = false;
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize, total: usize, viewport: usize) {
        let max = total.saturating_sub(viewport);
        self.scroll = (self.scroll + n).min(max);
        if self.scroll >= max {
            self.follow = true;
        }
    }

    pub fn scroll_left(&mut self, n: usize) {
        self.hscroll = self.hscroll.saturating_sub(n);
    }

    pub fn scroll_right(&mut self, n: usize) {
        self.hscroll = self.hscroll.saturating_add(n);
    }

    pub fn clamp_hscroll(&mut self, max_hscroll: usize) -> usize {
        self.hscroll = self.hscroll.min(max_hscroll);
        self.hscroll
    }
}

/// Handles data relative to a promoted category, i.e. a category that will be displayed to the
/// user in its own view. Also handles the category's associated search, if any.
pub struct Category {
    name: String,
    /// Strictly increasing row indices into `LogViewState::lines`.
    indices: Vec<usize>,
    view: ViewState,
    search: SearchState,
    /// When `Some`, every newly-pushed row whose text matches this regex is
    /// appended to `indices`. Set only when the user explicitly promotes a
    /// search term to a category from the search bar; tag-extracted
    /// categories leave this as `None` and rely on `categorize::extract`.
    match_regex: Option<Regex>,
}

impl Category {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn indices_count(&self) -> usize {
        self.indices.len()
    }

    pub fn last_index(&self) -> Option<&usize> {
        self.indices.last()
    }

    /// Returns the index at `idx`.
    ///
    /// # Panics
    ///
    /// May panic if `idx` is out of bounds.
    pub fn index(&self, idx: usize) -> usize {
        self.indices[idx]
    }

    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    pub fn view(&self) -> &ViewState {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut ViewState {
        &mut self.view
    }

    pub fn match_regex(&self) -> Option<&Regex> {
        self.match_regex.as_ref()
    }
}

/// Compiled query backing a `SearchState`. Literal queries are stored after
/// `regex::escape` so the matcher path is single-source.
pub struct CompiledQuery {
    regex: Regex,
    raw: String,
    is_regex: bool,
}

impl CompiledQuery {
    pub fn raw_query(&self) -> &str {
        &self.raw
    }

    pub fn is_regex(&self) -> bool {
        self.is_regex
    }
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
    query: Option<CompiledQuery>,
    matches: Vec<RowMatch>,
    current: Option<usize>,
}

impl SearchState {
    pub fn is_inactive(&self) -> bool {
        self.query.is_none() || self.matches.is_empty() || self.current.is_none()
    }

    pub fn query(&self) -> Option<&CompiledQuery> {
        self.query.as_ref()
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn current_match_row(&self) -> Option<usize> {
        self.current
            .and_then(|c| self.matches.get(c))
            .map(|m| m.row)
    }

    pub fn search_next(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let next = match self.current {
            Some(i) => (i + 1) % self.matches.len(),
            None => 0,
        };
        self.current = Some(next);
        Some(self.matches[next].row)
    }

    pub fn search_prev(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let prev = match self.current {
            Some(i) => (i + self.matches.len() - 1) % self.matches.len(),
            None => self.matches.len() - 1,
        };
        self.current = Some(prev);
        Some(self.matches[prev].row)
    }

    /// Returns the match at `idx`.
    ///
    /// # Panics
    ///
    /// May panic if `idx` is out of bounds.
    pub fn r#match(&self, idx: usize) -> &RowMatch {
        &self.matches[idx]
    }

    pub fn matches_count(&self) -> usize {
        self.matches.len()
    }

    pub fn matches_range(&self, scroll: usize, visible_end: usize) -> (usize, usize) {
        let lo = self.matches.partition_point(|m| m.row < scroll);
        let hi = self.matches.partition_point(|m| m.row < visible_end);
        (lo, hi)
    }
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

/// Primary struct for storing the UI's abstract state.
pub struct LogViewState {
    /// `VecDeque` rather than `Vec` so the front-trim at `max_lines`
    /// capacity is O(1) (head-pointer advance) instead of an O(N)
    /// memmove of the entire buffer on every overflowing push.
    lines: VecDeque<String>,
    /// Parallel to `lines`: each entry is the input line number of the
    /// row. Numbers come from `input_seq` and so survive trimming.
    line_numbers: VecDeque<usize>,
    main: ViewState,
    main_search: SearchState,
    max_lines: Option<usize>,
    categories: Vec<Category>,
    category_index: HashMap<String, usize>,
    pending: HashMap<String, PendingCategory>,
    /// Category names the user has explicitly hidden. `push` skips these
    /// before they can land in `pending` or get promoted again.
    ignored: HashSet<String>,
    /// Monotonic input-line counter; not affected by trimming.
    input_seq: usize,
}

impl LogViewState {
    pub fn new(max_lines: Option<usize>, display_follow: bool) -> Self {
        Self {
            lines: VecDeque::new(),
            line_numbers: VecDeque::new(),
            main: ViewState::new(display_follow),
            main_search: SearchState::default(),
            max_lines,
            categories: Vec::new(),
            category_index: HashMap::new(),
            pending: HashMap::new(),
            ignored: HashSet::new(),
            input_seq: 0,
        }
    }

    fn record_category_occurrence(
        &mut self,
        cat_name: &str,
        start: usize,
        end: usize,
        seq: usize,
    ) -> bool {
        let entry = self
            .pending
            .entry(cat_name.to_owned())
            .or_insert_with(|| PendingCategory {
                hits: 0,
                rows: Vec::new(),
                last_seen_seq: seq,
                recent_seqs: VecDeque::with_capacity(BURST_HITS),
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
    }

    fn promote_category(&mut self, cat_name: String) {
        let pending = self.pending.remove(&cat_name).expect("just promoted");
        let idx = self.categories.len();
        self.category_index.insert(cat_name.clone(), idx);
        self.categories.push(Category {
            name: cat_name,
            indices: pending.rows,
            view: ViewState::new(self.main.display_follow),
            search: SearchState::default(),
            match_regex: None,
        });
    }

    /// Records a new log line.
    ///
    /// This is where most of the application work occurs. This method:
    ///
    /// * extracts candidate categories from the `line` header,
    /// * adjusts the size of the internal buffer to keep it within the configured `max_lines`,
    /// * records category occurrences and promotes them to new views if necessary,
    /// * dispatches new lines to the various views, including the views created for displaying
    ///   search matches.
    pub fn push(&mut self, line: String) {
        let cats = categorize::extract(&line);

        let new_lines = line.split("\n").map(|t| t.to_owned());
        let start = self.lines.len();
        self.lines.extend(new_lines);
        let end = self.lines.len();

        self.input_seq += 1;
        let seq = self.input_seq;
        self.line_numbers.resize(end, seq);

        for cat_name in cats {
            if self.ignored.contains(&cat_name) {
                continue;
            }
            if let Some(&idx) = self.category_index.get(&cat_name) {
                self.categories[idx].indices.extend(start..end);
                self.scan_new_category_rows(idx, end - start);
                continue;
            }

            if self.record_category_occurrence(&cat_name, start, end, seq) {
                self.promote_category(cat_name);
            }
        }

        // Append matching rows to any user-promoted (regex-backed) category
        // so the pane keeps growing with the live stream after promotion.
        for cat_idx in 0..self.categories.len() {
            let Some(re) = self.categories[cat_idx].match_regex.clone() else {
                continue;
            };
            let mut added = 0;
            for row in start..end {
                if re.is_match(&self.lines[row]) {
                    self.categories[cat_idx].indices.push(row);
                    added += 1;
                }
            }
            if added > 0 {
                self.scan_new_category_rows(cat_idx, added);
            }
        }

        self.scan_new_main_rows(start, end);

        // Periodic eviction: drop pending candidates we haven't seen in a
        // while. Cheap enough to do every push when `pending` is small.
        self.pending
            .retain(|_, p| seq.saturating_sub(p.last_seen_seq) <= PENDING_EVICTION_AGE);

        if let Some(limit) = self.max_lines {
            let len = self.lines.len();
            if len > limit {
                let drop = len - limit;
                self.lines.drain(..drop);
                self.line_numbers.drain(..drop);
                self.main.scroll = self.main.scroll.saturating_sub(drop);
                // Pane rows for "all" are 1:1 with rendered rows.
                adjust_search_after_drop(&mut self.main_search, drop);

                // `cat.indices` and `pending.rows` are strictly increasing,
                // so the count of entries below `drop` is the partition
                // point — O(log n) instead of the prior linear scan.
                for cat in &mut self.categories {
                    let dropped_here = cat.indices.partition_point(|&i| i < drop);
                    cat.indices.drain(..dropped_here);
                    for i in &mut cat.indices {
                        *i -= drop;
                    }
                    cat.view.scroll = cat.view.scroll.saturating_sub(dropped_here);
                    adjust_search_after_drop(&mut cat.search, dropped_here);
                }
                for p in self.pending.values_mut() {
                    let dropped_here = p.rows.partition_point(|&i| i < drop);
                    p.rows.drain(..dropped_here);
                    for i in &mut p.rows {
                        *i -= drop;
                    }
                }
            }
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
            collect_matches(&q.regex, &self.lines[pane_row], pane_row, &mut new_matches);
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
        let Some(q) = cat.search.query.as_ref() else {
            return;
        };
        let total = cat.indices.len();
        let first = total.saturating_sub(new_count);
        let mut new_matches = Vec::new();
        for pane_row in first..total {
            let r = cat.indices[pane_row];
            collect_matches(&q.regex, &self.lines[r], pane_row, &mut new_matches);
        }
        self.categories[cat_idx].search.matches.extend(new_matches);
    }

    pub fn remove_category_at_idx(&mut self, idx: usize) {
        let name = self.categories.remove(idx).name;
        self.category_index.remove(&name);
        for v in self.category_index.values_mut() {
            if *v > idx {
                *v -= 1;
            }
        }
        self.pending.remove(&name);
        self.ignored.insert(name);
    }

    pub fn active_search_mut(&mut self, selected: usize) -> &mut SearchState {
        if selected == 0 {
            &mut self.main_search
        } else {
            &mut self.categories[selected - 1].search
        }
    }

    pub fn clear_search(&mut self, selected: usize) {
        *self.active_search_mut(selected) = SearchState::default();
    }

    /// Compile `raw` and replace the active pane's search state. On success
    /// returns the number of matches found. Empty input clears the search.
    /// Pane rows are local (0-based within the active pane), so "all" rows
    /// map 1:1 with `rendered`, while category rows index `cat.indices`.
    pub fn commit_search(
        &mut self,
        raw: &str,
        is_regex: bool,
        selected: usize,
    ) -> Result<usize, regex::Error> {
        if raw.is_empty() {
            self.clear_search(selected);
            return Ok(0);
        }
        let pattern = if is_regex {
            raw.to_string()
        } else {
            regex::escape(raw)
        };
        let regex = Regex::new(&pattern)?;
        let query = CompiledQuery {
            regex,
            raw: raw.to_string(),
            is_regex,
        };

        let mut matches = Vec::new();
        if selected == 0 {
            for (pane_row, line) in self.lines.iter().enumerate() {
                collect_matches(&query.regex, line, pane_row, &mut matches);
            }
        } else {
            let cat = &self.categories[selected - 1];
            for (pane_row, &r) in cat.indices.iter().enumerate() {
                collect_matches(&query.regex, &self.lines[r], pane_row, &mut matches);
            }
        }

        let current = if matches.is_empty() { None } else { Some(0) };
        let new_state = SearchState {
            query: Some(query),
            matches,
            current,
        };
        let count = new_state.matches.len();
        *self.active_search_mut(selected) = new_state;
        Ok(count)
    }

    /// Turn `raw` into a new category pane: scan existing rendered rows for
    /// matches, then keep the regex so future pushes append to the pane.
    /// If a category with the same name already exists, just switches to it.
    /// Empty input is a no-op. Selection moves to the resulting pane on
    /// success. Returns an optional category index corresponding to the search
    /// result, otherwise `None`.
    pub fn promote_search_to_category(
        &mut self,
        raw: &str,
        is_regex: bool,
    ) -> Result<Option<usize>, regex::Error> {
        if raw.is_empty() {
            return Ok(None);
        }
        let name = raw.to_string();
        if let Some(&idx) = self.category_index.get(&name) {
            return Ok(Some(idx + 1));
        }
        let pattern = if is_regex {
            raw.to_string()
        } else {
            regex::escape(raw)
        };
        let regex = Regex::new(&pattern)?;

        // Explicit promotion overrides a prior hide and any pending state
        // tracked under the same name.
        self.pending.remove(&name);
        self.ignored.remove(&name);

        let mut indices = Vec::new();
        for (row, line) in self.lines.iter().enumerate() {
            if regex.is_match(line) {
                indices.push(row);
            }
        }

        let idx = self.categories.len();
        self.category_index.insert(name.clone(), idx);
        self.categories.push(Category {
            name,
            indices,
            view: ViewState::new(self.main.display_follow),
            search: SearchState::default(),
            match_regex: Some(regex),
        });
        Ok(Some(idx + 1))
    }

    pub fn category_count(&self) -> usize {
        self.categories.len()
    }

    /// Returns the category with at index `idx`.
    ///
    /// # Panics
    ///
    /// May panic if the index is out of bounds.
    pub fn get_category(&self, idx: usize) -> &Category {
        &self.categories[idx]
    }

    /// Returns the mutable category with at index `idx`.
    ///
    /// # Panics
    ///
    /// May panic if the index is out of bounds.
    pub fn get_category_mut(&mut self, idx: usize) -> &mut Category {
        &mut self.categories[idx]
    }

    pub fn category_indices_and_names(&self) -> impl Iterator<Item = (usize, &str)> {
        self.categories
            .iter()
            .enumerate()
            .map(|(idx, c)| (idx, c.name.as_str()))
    }

    pub fn line_numbers(&self) -> &VecDeque<usize> {
        &self.line_numbers
    }

    pub fn main_view(&self) -> &ViewState {
        &self.main
    }

    pub fn main_view_mut(&mut self) -> &mut ViewState {
        &mut self.main
    }

    pub fn get_search(&self, selected: usize) -> &SearchState {
        if selected == 0 {
            &self.main_search
        } else {
            &self.get_category(selected - 1).search
        }
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

fn collect_matches(regex: &Regex, line: &str, pane_row: usize, out: &mut Vec<RowMatch>) {
    for m in regex.find_iter(line) {
        out.push(RowMatch {
            row: pane_row,
            start: m.start(),
            end: m.end(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_lines(app: &mut LogViewState, lines: &[&str]) {
        for line in lines {
            app.push((*line).to_string());
        }
    }

    #[test]
    fn literal_search_finds_matches_in_main() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["foo bar", "no match", "another foo"]);
        let n = app.commit_search("foo", false, 0).unwrap();
        assert_eq!(n, 2);
        assert_eq!(app.main_search.matches.len(), 2);
        assert_eq!(app.main_search.matches[0].row, 0);
        assert_eq!(app.main_search.matches[1].row, 2);
        assert_eq!(app.main_search.current, Some(0));
    }

    #[test]
    fn regex_search_finds_matches() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["abc 123", "xyz 7", "no digits here"]);
        let n = app.commit_search(r"\d+", true, 0).unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn literal_search_treats_metacharacters_literally() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["a.b", "axb"]);
        // In literal mode, `.` matches a dot, not any char.
        assert_eq!(app.commit_search(".", false, 0).unwrap(), 1);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["foo"]);
        assert!(app.commit_search("(", true, 0).is_err());
    }

    #[test]
    fn empty_query_clears_search() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["foo"]);
        app.commit_search("foo", false, 0).unwrap();
        assert!(app.main_search.query.is_some());
        app.commit_search("", false, 0).unwrap();
        assert!(app.main_search.query.is_none());
        assert!(app.main_search.matches.is_empty());
    }

    #[test]
    fn push_extends_active_search_matches() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["foo"]);
        app.commit_search("foo", false, 0).unwrap();
        assert_eq!(app.main_search.matches.len(), 1);
        push_lines(&mut app, &["foo bar foo"]);
        assert_eq!(app.main_search.matches.len(), 3);
        assert_eq!(app.main_search.matches[1].row, 1);
        assert_eq!(app.main_search.matches[2].row, 1);
        assert_eq!(app.main_search.matches[2].start, 8);
    }

    #[test]
    fn trim_drops_and_shifts_main_matches() {
        let mut app = LogViewState::new(Some(2), true);
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false, 0).unwrap();
        assert_eq!(app.lines.len(), 2);
        assert_eq!(app.main_search.matches.len(), 2);
        push_lines(&mut app, &["foo"]);
        assert_eq!(app.lines.len(), 2);
        assert_eq!(app.main_search.matches.len(), 2);
        assert_eq!(app.main_search.matches[0].row, 0);
        assert_eq!(app.main_search.matches[1].row, 1);
    }

    #[test]
    fn trim_resets_current_when_it_was_dropped() {
        let mut app = LogViewState::new(Some(2), true);
        push_lines(&mut app, &["foo", "foo"]);
        app.commit_search("foo", false, 0).unwrap();
        // current = Some(0); push enough to drop the first match.
        push_lines(&mut app, &["foo", "foo"]);
        // Two oldest dropped; surviving matches shift; current falls back
        // to 0 (the new first match).
        assert_eq!(app.main_search.current, Some(0));
        assert_eq!(app.main_search.matches.len(), 2);
    }

    #[test]
    fn promote_search_creates_category_with_existing_matches() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["alpha one", "beta two", "alpha three"]);
        assert!(app.categories.is_empty());
        let res = app.promote_search_to_category("alpha", false).unwrap();
        assert_eq!(app.categories.len(), 1);
        assert_eq!(app.categories[0].name, "alpha");
        assert_eq!(app.categories[0].indices, vec![0, 2]);
        // Selection moved to the new pane.
        assert_eq!(res, Some(1));
    }

    #[test]
    fn promote_search_captures_future_matches() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["alpha one"]);
        app.promote_search_to_category("alpha", false).unwrap();
        assert_eq!(app.categories[0].indices, vec![0]);
        push_lines(&mut app, &["beta", "alpha two", "gamma alpha"]);
        assert_eq!(app.categories[0].indices, vec![0, 2, 3]);
    }

    #[test]
    fn promote_search_empty_input_is_noop() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["alpha"]);
        assert!(app.promote_search_to_category("", false).unwrap().is_none());
        assert!(app.categories.is_empty());
    }

    #[test]
    fn promote_search_regex_mode() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["err 42", "ok", "err 7"]);
        assert!(
            app.promote_search_to_category(r"err \d+", true)
                .unwrap()
                .is_some()
        );
        assert_eq!(app.categories[0].indices, vec![0, 2]);
    }

    #[test]
    fn promote_search_invalid_regex_errors() {
        let mut app = LogViewState::new(None, true);
        push_lines(&mut app, &["x"]);
        assert!(app.promote_search_to_category("(", true).is_err());
        assert!(app.categories.is_empty());
    }
}
