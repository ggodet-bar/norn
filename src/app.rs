use std::collections::{HashMap, VecDeque};

use ansi_to_tui::IntoText;
use ratatui::text::Line;

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
                });
            }
        }

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

                for cat in &mut self.categories {
                    let dropped_here = cat.indices.iter().take_while(|&&i| i < drop).count();
                    cat.indices.drain(..dropped_here);
                    for i in &mut cat.indices {
                        *i -= drop;
                    }
                    cat.view.scroll = cat.view.scroll.saturating_sub(dropped_here);
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
}
