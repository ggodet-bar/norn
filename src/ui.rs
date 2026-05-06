use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::InputMode;
use crate::app::App;

const MATCH_STYLE: Style = Style::new().bg(Color::Yellow).fg(Color::Black);
const CURRENT_MATCH_STYLE: Style = Style::new()
    .bg(Color::LightYellow)
    .fg(Color::Black)
    .add_modifier(Modifier::BOLD);

pub fn draw(f: &mut Frame, app: &mut App, mode: &InputMode) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_tabs(f, app, chunks[0]);
    draw_log(f, app, chunks[1]);
    match mode {
        InputMode::Search { buffer, is_regex, error } => {
            draw_search_bar(f, buffer, *is_regex, error.as_deref(), chunks[2]);
        }
        InputMode::Goto { buffer, error } => {
            draw_goto_bar(f, buffer, error.as_deref(), chunks[2]);
        }
        InputMode::Normal => draw_status(f, app, chunks[2]),
    }
}

/// Render the tab strip. The "0:all" tab is pinned at the left edge; the
/// remaining category tabs slide so the active one stays as close to the
/// center as possible, with `‹` / `›` cues when there are hidden tabs in
/// either direction.
fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let labels: Vec<String> = std::iter::once("0:all".to_string())
        .chain(app.categories.iter().enumerate().map(|(i, c)| {
            format!("{}:{}", i + 1, truncate(&c.name, 20))
        }))
        .collect();
    // Each tab body is rendered as ` label ` so width is `chars + 2`.
    let widths: Vec<usize> = labels.iter().map(|l| l.chars().count() + 2).collect();

    let (start, end, show_left, show_right) =
        compute_tab_window(&widths, app.selected, area.width as usize);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.extend(tab_spans(&labels[0], app.selected == 0));
    if show_left {
        spans.push(divider_span());
        spans.push(cue_span("‹"));
    }
    for i in start..end {
        spans.push(divider_span());
        spans.extend(tab_spans(&labels[i], app.selected == i));
    }
    if show_right {
        spans.push(divider_span());
        spans.push(cue_span("›"));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Pick which category tabs to show given a width budget and the active
/// tab's index. `widths[0]` is the "0:all" tab body width; `widths[1..]`
/// are category bodies. Returns `(start, end, show_left, show_right)`,
/// where the visible category range is `start..end` (exclusive end). The
/// "0:all" tab is always rendered, so when only it is visible the range
/// is `(1, 1)` with no cues.
fn compute_tab_window(
    widths: &[usize],
    selected: usize,
    avail: usize,
) -> (usize, usize, bool, bool) {
    let n = widths.len();
    if n <= 1 {
        return (1, 1, false, false);
    }

    // Body width of a left/right cue (` ‹ ` / ` › `).
    const CUE_BODY: usize = 3;
    // Cost of having `[start..=end]` visible alongside "0:all", with cue
    // bodies added on each side that has hidden tabs. Each visible piece
    // (cue or tab) is preceded by a 1-char divider.
    let cost = |start: usize, end: usize| -> usize {
        let mut c = widths[0];
        if start > 1 {
            c += 1 + CUE_BODY;
        }
        for w in &widths[start..=end] {
            c += 1 + w;
        }
        if end < n - 1 {
            c += 1 + CUE_BODY;
        }
        c
    };

    // Fast path: everything fits.
    if cost(1, n - 1) <= avail {
        return (1, n, false, false);
    }

    // Center on the active category. When "0:all" is selected we anchor at
    // the first category so the window stays left-aligned.
    let active = selected.max(1).min(n - 1);

    let mut start = active;
    let mut end = active;
    if cost(start, end) > avail {
        // Even the active tab plus cues doesn't fit; show it anyway and let
        // the terminal clip. This only triggers on absurdly narrow widths.
        return (active, active + 1, active > 1, active < n - 1);
    }

    // Grow outward, alternating right then left, keeping `active` centered.
    let mut prefer_right = true;
    loop {
        let can_right = end < n - 1;
        let can_left = start > 1;
        if !can_right && !can_left {
            break;
        }
        let try_right = prefer_right && can_right || !can_left;
        let (ns, ne) = if try_right {
            (start, end + 1)
        } else {
            (start - 1, end)
        };
        if cost(ns, ne) <= avail {
            start = ns;
            end = ne;
            prefer_right = !prefer_right;
            continue;
        }
        // Couldn't grow that way; try the other side once before giving up.
        let other_right = !try_right && can_right;
        let other_left = try_right && can_left;
        if other_right {
            let (ns, ne) = (start, end + 1);
            if cost(ns, ne) <= avail {
                start = ns;
                end = ne;
                prefer_right = false;
                continue;
            }
        } else if other_left {
            let (ns, ne) = (start - 1, end);
            if cost(ns, ne) <= avail {
                start = ns;
                end = ne;
                prefer_right = true;
                continue;
            }
        }
        break;
    }

    (start, end + 1, start > 1, end < n - 1)
}

fn tab_spans(label: &str, selected: bool) -> Vec<Span<'static>> {
    let style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    vec![Span::styled(format!(" {label} "), style)]
}

fn cue_span(c: &str) -> Span<'static> {
    Span::styled(format!(" {c} "), Style::default().fg(Color::DarkGray))
}

fn divider_span() -> Span<'static> {
    Span::styled("│", Style::default().fg(Color::DarkGray))
}

fn draw_log(f: &mut Frame, app: &mut App, area: Rect) {
    let title = if app.selected == 0 {
        " all ".to_string()
    } else {
        format!(" {} ", app.categories[app.selected - 1].name)
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    let viewport = inner.height as usize;

    // We clone Lines here only for the rendered frame; storage stays
    // index-based per pane, so categories never duplicate the buffer.
    // `render_rows` keeps the absolute index into `app.rendered` for each
    // pane row so the line-number gutter can fetch input numbers below.
    let (mut lines, render_rows, total): (Vec<Line<'static>>, Vec<usize>, usize) =
        if app.selected == 0 {
            let n = app.rendered.len();
            (app.rendered.clone(), (0..n).collect(), n)
        } else {
            let cat = &app.categories[app.selected - 1];
            (
                cat.indices.iter().map(|&i| app.rendered[i].clone()).collect(),
                cat.indices.clone(),
                cat.indices.len(),
            )
        };

    apply_search_highlights(&mut lines, app);
    let goto_mask: Vec<bool> = match app.goto_highlight {
        Some(target) => render_rows
            .iter()
            .map(|&r| app.line_numbers.get(r).copied() == Some(target))
            .collect(),
        None => Vec::new(),
    };
    if app.show_line_numbers {
        prepend_line_number_gutter(&mut lines, &render_rows, &app.line_numbers, &goto_mask);
    }
    apply_goto_highlight(&mut lines, &goto_mask);

    let max_scroll = total.saturating_sub(viewport);
    let (view, _) = app.active_view_mut();
    if view.follow {
        view.scroll = max_scroll;
    } else {
        view.scroll = view.scroll.min(max_scroll);
    }
    let scroll = view.scroll as u16;

    let p = Paragraph::new(lines).block(block).scroll((scroll, 0));
    f.render_widget(p, area);
}

/// Prepend a right-aligned line-number gutter to each pane line. Width is
/// sized to the largest visible number so columns stay aligned. Repeated
/// numbers (multiple rendered rows from one input line) only print on the
/// first occurrence; later rows show a blank gutter.
fn prepend_line_number_gutter(
    lines: &mut [Line<'static>],
    render_rows: &[usize],
    numbers: &[usize],
    goto_mask: &[bool],
) {
    if lines.is_empty() {
        return;
    }
    let max = render_rows
        .iter()
        .filter_map(|&r| numbers.get(r).copied())
        .max()
        .unwrap_or(0);
    let width = max.to_string().len().max(1);
    let normal = Style::default().fg(Color::DarkGray);
    let highlight = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut prev: Option<usize> = None;
    for (i, (line, &r)) in lines.iter_mut().zip(render_rows.iter()).enumerate() {
        let n = numbers.get(r).copied();
        let label = match n {
            Some(num) if Some(num) != prev => format!("{:>width$} │ ", num, width = width),
            _ => format!("{:>width$} │ ", "", width = width),
        };
        prev = n;
        let style = if goto_mask.get(i).copied().unwrap_or(false) {
            highlight
        } else {
            normal
        };
        let mut spans = Vec::with_capacity(line.spans.len() + 1);
        spans.push(Span::styled(label, style));
        spans.extend(line.spans.drain(..));
        *line = Line::from(spans);
    }
}

/// Patch a discrete background colour onto every span of a row whose
/// `goto_mask` entry is true. Existing span backgrounds (e.g. search
/// match highlights) keep precedence — `Style::patch` only fills in
/// fields that are unset on the span.
fn apply_goto_highlight(lines: &mut [Line<'static>], goto_mask: &[bool]) {
    if goto_mask.is_empty() {
        return;
    }
    let bg = Style::default().bg(Color::DarkGray);
    for (i, line) in lines.iter_mut().enumerate() {
        if !goto_mask.get(i).copied().unwrap_or(false) {
            continue;
        }
        for span in &mut line.spans {
            span.style = bg.patch(span.style);
        }
        line.style = bg.patch(line.style);
    }
}

/// Splice highlight styling into pane lines for every match in the active
/// search. Lines without matches are left untouched.
fn apply_search_highlights(lines: &mut [Line<'static>], app: &App) {
    let search = app.active_search();
    if search.query.is_none() || search.matches.is_empty() {
        return;
    }
    let mut by_row: HashMap<usize, Vec<(usize, usize, Style)>> = HashMap::new();
    for (idx, m) in search.matches.iter().enumerate() {
        let style = if Some(idx) == search.current {
            CURRENT_MATCH_STYLE
        } else {
            MATCH_STYLE
        };
        by_row.entry(m.row).or_default().push((m.start, m.end, style));
    }
    for (row, ranges) in &by_row {
        if let Some(line) = lines.get_mut(*row) {
            *line = highlight_line(line, ranges);
        }
    }
}

/// Re-emit `line` with `ranges` (byte offsets in the line's plain text)
/// restyled by overlay. Spans are split at every range boundary so the
/// overlay sits on top of the original styling instead of replacing it.
fn highlight_line(line: &Line<'static>, ranges: &[(usize, usize, Style)]) -> Line<'static> {
    // Collect breakpoints from span boundaries and from each range. Walking
    // the deduped boundaries pairwise emits one Span per (style, slice).
    let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let mut points: Vec<usize> = Vec::with_capacity(line.spans.len() + 1 + 2 * ranges.len());
    let mut cursor = 0usize;
    points.push(0);
    for span in &line.spans {
        cursor += span.content.len();
        points.push(cursor);
    }
    let total_len = cursor;
    for &(s, e, _) in ranges {
        if s < total_len {
            points.push(s);
        }
        if e <= total_len {
            points.push(e);
        }
    }
    points.sort_unstable();
    points.dedup();

    let mut out = Vec::with_capacity(points.len());
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a == b {
            continue;
        }
        let base = style_at(line, a);
        let overlay = ranges
            .iter()
            .find(|(s, e, _)| *s <= a && *e >= b)
            .map(|(_, _, st)| *st);
        let style = match overlay {
            Some(o) => base.patch(o),
            None => base,
        };
        out.push(Span::styled(plain[a..b].to_string(), style));
    }
    Line::from(out)
}

fn style_at(line: &Line<'static>, offset: usize) -> Style {
    let mut cursor = 0;
    for span in &line.spans {
        let end = cursor + span.content.len();
        if offset < end {
            return span.style;
        }
        cursor = end;
    }
    Style::default()
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let (label, total, view) = if app.selected == 0 {
        ("all", app.rendered.len(), &app.main)
    } else {
        let cat = &app.categories[app.selected - 1];
        (cat.name.as_str(), cat.indices.len(), &cat.view)
    };
    let mut s = format!(
        " {label}: {total} lines · {} ",
        if view.follow { "FOLLOW" } else { "PAUSED" }
    );
    let search = app.active_search();
    if let Some(q) = &search.query {
        let total = search.matches.len();
        let pos = match search.current {
            Some(i) if total > 0 => i + 1,
            _ => 0,
        };
        let prefix = if q.is_regex { "re/" } else { "/" };
        s.push_str(&format!("· {prefix}{} {pos}/{total} ", q.raw));
    }
    s.push_str("· q quit · / search · : goto · n/N next/prev · ↑/↓ PgUp/PgDn scroll · End follow · Tab/0-9 panes · Ctrl-X hide ");
    f.render_widget(
        Paragraph::new(s).style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
}

fn draw_search_bar(
    f: &mut Frame,
    buffer: &str,
    is_regex: bool,
    error: Option<&str>,
    area: Rect,
) {
    let prefix = if is_regex { "re/" } else { "/" };
    let mut spans = vec![
        Span::styled(
            format!(" {prefix}"),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled(buffer.to_string(), Style::default().fg(Color::White)),
        // A trailing block stands in for a cursor.
        Span::styled("▏", Style::default().fg(Color::White)),
    ];
    if let Some(err) = error {
        spans.push(Span::styled(
            format!("  [{err}]"),
            Style::default().fg(Color::LightRed),
        ));
    } else {
        spans.push(Span::styled(
            "  Enter: apply · Esc: cancel · Ctrl-R: regex".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let line = Line::from(spans);
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn draw_goto_bar(f: &mut Frame, buffer: &str, error: Option<&str>, area: Rect) {
    let mut spans = vec![
        Span::styled(
            " :".to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled(buffer.to_string(), Style::default().fg(Color::White)),
        Span::styled("▏", Style::default().fg(Color::White)),
    ];
    if let Some(err) = error {
        spans.push(Span::styled(
            format!("  [{err}]"),
            Style::default().fg(Color::LightRed),
        ));
    } else {
        spans.push(Span::styled(
            "  Enter: go to line · Esc: cancel".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let line = Line::from(spans);
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::compute_tab_window;

    /// Build a widths vector where `widths[0]` is the "0:all" body width
    /// and the remaining entries are uniform-width category tabs.
    fn uniform(n_cats: usize, cat_width: usize) -> Vec<usize> {
        let mut v = Vec::with_capacity(n_cats + 1);
        v.push(7); // " 0:all "
        v.extend(std::iter::repeat(cat_width).take(n_cats));
        v
    }

    #[test]
    fn shows_everything_when_it_fits() {
        let w = uniform(3, 8);
        let (start, end, l, r) = compute_tab_window(&w, 0, 200);
        assert_eq!((start, end, l, r), (1, 4, false, false));
    }

    #[test]
    fn no_categories_renders_only_all() {
        let w = uniform(0, 8);
        assert_eq!(compute_tab_window(&w, 0, 80), (1, 1, false, false));
    }

    #[test]
    fn centers_active_when_window_slides() {
        // 20 cats of width 8; total far exceeds budget — expect a window
        // around the active tab with both cues.
        let w = uniform(20, 8);
        let (start, end, l, r) = compute_tab_window(&w, 10, 60);
        assert!(l && r, "expected both cues for a centered window");
        assert!(start < 10 && end > 10, "active tab must be inside window");
        // Roughly centered: distance to either end of the window should be
        // within one tab of equal.
        let left_dist = 10 - start;
        let right_dist = end - 1 - 10;
        assert!(left_dist.abs_diff(right_dist) <= 1);
    }

    #[test]
    fn drops_left_cue_when_active_near_start() {
        let w = uniform(20, 8);
        let (start, _, l, r) = compute_tab_window(&w, 1, 60);
        assert_eq!(start, 1, "window must start at first category");
        assert!(!l, "no left cue when window touches the start");
        assert!(r, "right cue still needed");
    }

    #[test]
    fn drops_right_cue_when_active_near_end() {
        let w = uniform(20, 8);
        let n = w.len();
        let (_, end, l, r) = compute_tab_window(&w, n - 1, 60);
        assert_eq!(end, n, "window must reach the last category");
        assert!(l);
        assert!(!r);
    }

    #[test]
    fn all_pane_selected_anchors_window_left() {
        // When "0:all" is active, the category window stays left-aligned
        // (no left cue), since centering on "all" doesn't make sense.
        let w = uniform(20, 8);
        let (start, _, l, _) = compute_tab_window(&w, 0, 60);
        assert_eq!(start, 1);
        assert!(!l);
    }

    #[test]
    fn graceful_fallback_when_active_alone_overflows() {
        // Width too narrow for "0:all" + active + cues. We still return a
        // single-tab window so something renders.
        let w = uniform(5, 30);
        let (start, end, _, _) = compute_tab_window(&w, 3, 10);
        assert_eq!(end - start, 1);
        assert_eq!(start, 3);
    }
}
