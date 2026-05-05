use ansi_to_tui::IntoText;
use ratatui::text::Line;

use crate::capture::LogLine;

pub struct App {
    /// Pre-rendered log lines, ANSI parsed once on ingest.
    pub rendered: Vec<Line<'static>>,
    /// First visible row when not following.
    pub scroll: usize,
    /// View pinned to the tail.
    pub follow: bool,
    /// Cap on retained rendered rows; `None` means unbounded.
    pub max_lines: Option<usize>,
}

impl App {
    pub fn new(max_lines: Option<usize>) -> Self {
        Self {
            rendered: Vec::new(),
            scroll: 0,
            follow: true,
            max_lines,
        }
    }

    pub fn push(&mut self, line: LogLine) {
        let parsed = line
            .raw
            .as_bytes()
            .into_text()
            .map(|t| t.lines)
            .unwrap_or_else(|_| vec![Line::raw(line.raw)]);
        self.rendered.extend(parsed);

        if let Some(limit) = self.max_lines {
            let len = self.rendered.len();
            if len > limit {
                let drop = len - limit;
                self.rendered.drain(..drop);
                // Keep the paused viewport anchored on the same content; if it
                // was within the dropped window, clamp to the new top.
                self.scroll = self.scroll.saturating_sub(drop);
            }
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.follow = false;
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport: usize) {
        let max = self.rendered.len().saturating_sub(viewport);
        self.scroll = (self.scroll + n).min(max);
        if self.scroll >= max {
            self.follow = true;
        }
    }

    pub fn scroll_top(&mut self) {
        self.follow = false;
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.follow = true;
    }
}
