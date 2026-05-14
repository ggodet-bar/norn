use std::collections::{HashMap, VecDeque};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::{InputMode, app::App};

const MATCH_STYLE: Style = Style::new()
    .bg(Color::Gray)
    .fg(Color::Black)
    .add_modifier(Modifier::BOLD);
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
        InputMode::Search {
            buffer,
            is_regex,
            error,
        } => {
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
        .chain(
            app.log_view_state
                .category_indices_and_names()
                .map(|(i, name)| format!("{}:{}", i + 1, truncate(name, 20))),
        )
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
    for (i, label) in labels.iter().enumerate().take(end).skip(start) {
        spans.push(divider_span());
        spans.extend(tab_spans(label, app.selected == i));
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
        format!(
            " {} ",
            app.log_view_state.get_category(app.selected - 1).name()
        )
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    let viewport = inner.height as usize;

    let total = if app.selected == 0 {
        app.rendered.len()
    } else {
        app.log_view_state
            .get_category(app.selected - 1)
            .indices_count()
    };

    // Reconcile follow / clamp first so we know the visible window before
    // slicing. Doing this up front lets us clone only viewport-sized
    // chunks and skip the per-frame full-buffer copy.
    let max_scroll = total.saturating_sub(viewport);
    let scroll = {
        let (view, _) = app.active_view_mut();
        view.follow_or_clamp(max_scroll)
    };
    let visible_end = (scroll + viewport).min(total);

    // Clone only the visible window. `render_rows` keeps the absolute
    // index into `app.rendered` for each visible pane row so the gutter
    // and goto highlight can look up input line numbers.
    let (mut lines, render_rows): (Vec<Line<'static>>, Vec<usize>) = if app.selected == 0 {
        (
            app.rendered.range(scroll..visible_end).cloned().collect(),
            (scroll..visible_end).collect(),
        )
    } else {
        let cat = app.log_view_state.get_category(app.selected - 1);
        let slice = &cat.indices()[scroll..visible_end];
        (
            slice.iter().map(|&i| app.rendered[i].clone()).collect(),
            slice.to_vec(),
        )
    };

    apply_search_highlights(&mut lines, app, scroll);
    let goto_mask: Vec<bool> = match app.goto_highlight {
        Some(target) => render_rows
            .iter()
            .map(|&r| app.log_view_state.line_numbers().get(r).copied() == Some(target))
            .collect(),
        None => Vec::new(),
    };
    let mut gutter_lines: Vec<Line<'static>> = if app.show_line_numbers {
        // Use the largest input line number currently in the pane (not
        // just the visible window) so the gutter width stays stable as
        // the user scrolls.
        let max_line_no = if app.selected == 0 {
            app.log_view_state
                .line_numbers()
                .back()
                .copied()
                .unwrap_or(0)
        } else {
            app.log_view_state
                .get_category(app.selected - 1)
                .last_index()
                .and_then(|&i| app.log_view_state.line_numbers().get(i).copied())
                .unwrap_or(0)
        };
        build_line_number_gutter(
            &render_rows,
            app.log_view_state.line_numbers(),
            &goto_mask,
            max_line_no,
        )
    } else {
        Vec::new()
    };
    apply_goto_highlight(&mut lines, &goto_mask);
    apply_goto_highlight(&mut gutter_lines, &goto_mask);

    // Sticky gutter sits in its own column so horizontal scroll only
    // shifts the body. Width comes from the first gutter row — all rows
    // are built to the same width.
    let gutter_width = gutter_lines.first().map(line_width).unwrap_or(0) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(gutter_width), Constraint::Min(0)])
        .split(inner);
    let body_area = chunks[1];

    // Clamp horizontal scroll to the longest visible line — past the end
    // of every line there's nothing left to reveal.
    let max_body_width = lines.iter().map(line_width).max().unwrap_or(0);
    let max_hscroll = max_body_width.saturating_sub(body_area.width as usize);
    let hscroll = {
        let (view, _) = app.active_view_mut();
        view.clamp_hscroll(max_hscroll)
    } as u16;

    f.render_widget(&block, area);
    if !gutter_lines.is_empty() {
        f.render_widget(Paragraph::new(gutter_lines), chunks[0]);
    }
    f.render_widget(Paragraph::new(lines).scroll((0, hscroll)), body_area);
}

fn line_width(line: &Line<'static>) -> usize {
    line.spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Build a sticky right-aligned line-number gutter as one `Line` per visible
/// pane row. Width is sized to the largest pane line number so columns stay
/// aligned as the user scrolls. Repeated numbers (multiple rendered rows from
/// one input line) only print on the first occurrence; later rows show a
/// blank gutter.
fn build_line_number_gutter(
    render_rows: &[usize],
    numbers: &VecDeque<usize>,
    goto_mask: &[bool],
    max_line_no: usize,
) -> Vec<Line<'static>> {
    if render_rows.is_empty() {
        return Vec::new();
    }
    let width = max_line_no.to_string().len().max(1);
    let normal = Style::default().fg(Color::DarkGray);
    let highlight = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut out = Vec::with_capacity(render_rows.len());
    let mut prev: Option<usize> = None;
    for (i, &r) in render_rows.iter().enumerate() {
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
        out.push(Line::from(Span::styled(label, style)));
    }
    out
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

/// Splice highlight styling into the visible pane lines for every match
/// in the active search. `scroll` is the pane-local row of the first
/// element of `lines`; matches outside `[scroll, scroll + lines.len())`
/// are skipped. Match rows are translated to viewport-local indices.
/// Relies on `SearchState::matches` being sorted by `(row, start)`.
fn apply_search_highlights(lines: &mut [Line<'static>], app: &App, scroll: usize) {
    let search = app.active_search();
    if search.is_inactive() {
        return;
    }
    let visible_end = scroll + lines.len();
    let (lo, hi) = search.matches_range(scroll, visible_end);
    if lo == hi {
        return;
    }
    let mut by_row: HashMap<usize, Vec<(usize, usize, Style)>> = HashMap::new();
    for idx in lo..hi {
        let m = search.r#match(idx);
        let style = if Some(idx) == search.current() {
            CURRENT_MATCH_STYLE
        } else {
            MATCH_STYLE
        };
        by_row
            .entry(m.row - scroll)
            .or_default()
            .push((m.start, m.end, style));
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
        ("all", app.rendered.len(), app.log_view_state.main_view())
    } else {
        let cat = &app.log_view_state.get_category(app.selected - 1);
        (cat.name(), cat.indices_count(), cat.view())
    };
    let mut s = format!(
        " {label}: {total} lines {}",
        if view.display_follow() {
            if view.is_following() {
                "· FOLLOW "
            } else {
                "· PAUSED "
            }
        } else {
            ""
        }
    );
    let search = app.active_search();
    if let Some(q) = &search.query() {
        let total = search.matches_count();
        let pos = match search.current() {
            Some(i) if total > 0 => i + 1,
            _ => 0,
        };
        let prefix = if q.is_regex() { "re/" } else { "/" };
        s.push_str(&format!("· {prefix}{} {pos}/{total} ", q.raw_query()));
    }
    s.push_str("· q quit · / search · : goto · n/N next/prev · hjkl/↑↓ PgUp/PgDn scroll · g/G top/bottom · End follow · Tab/0-9 panes · c promote search · Ctrl-X hide ");
    f.render_widget(
        Paragraph::new(s).style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
}

fn draw_search_bar(f: &mut Frame, buffer: &str, is_regex: bool, error: Option<&str>, area: Rect) {
    let prefix = if is_regex { "re/" } else { "/" };
    let mut spans = vec![
        Span::styled(
            format!(" {prefix}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
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
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
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
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::{InputMode, app::App, capture::LogLine};

    /// Build a widths vector where `widths[0]` is the "0:all" body width
    /// and the remaining entries are uniform-width category tabs.
    fn uniform(n_cats: usize, cat_width: usize) -> Vec<usize> {
        let mut v = Vec::with_capacity(n_cats + 1);
        v.push(7); // " 0:all "
        v.extend(std::iter::repeat(cat_width).take(n_cats));
        v
    }

    fn push_lines(app: &mut App, lines: &[&str]) {
        for raw in lines {
            app.push(LogLine {
                raw: (*raw).to_string(),
            });
        }
    }

    fn render(app: &mut App, mode: &InputMode, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, app, mode)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    fn line_text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
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

    // ---------- truncate ----------

    #[test]
    fn truncate_keeps_strings_at_or_below_limit() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcde", 5), "abcde");
    }

    #[test]
    fn truncate_replaces_overflow_with_ellipsis() {
        assert_eq!(truncate("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn truncate_counts_unicode_chars_not_bytes() {
        // Multi-byte glyphs must count as one each — otherwise the budget
        // would be measured in bytes and clip multi-byte text early.
        assert_eq!(truncate("αβγδε", 5), "αβγδε");
        assert_eq!(truncate("αβγδεζ", 5), "αβγδ…");
    }

    // ---------- line_width / style_at ----------

    #[test]
    fn line_width_sums_char_counts_across_spans() {
        let line = Line::from(vec![Span::raw("abc"), Span::raw(""), Span::raw("de")]);
        assert_eq!(line_width(&line), 5);
    }

    #[test]
    fn line_width_counts_unicode_as_chars() {
        let line = Line::from(vec![Span::raw("héllo")]);
        assert_eq!(line_width(&line), 5);
    }

    #[test]
    fn style_at_returns_span_style_for_offset() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![Span::styled("abc", red), Span::styled("defg", blue)]);
        assert_eq!(style_at(&line, 0).fg, Some(Color::Red));
        assert_eq!(style_at(&line, 2).fg, Some(Color::Red));
        assert_eq!(style_at(&line, 3).fg, Some(Color::Blue));
        assert_eq!(style_at(&line, 6).fg, Some(Color::Blue));
    }

    #[test]
    fn style_at_past_end_returns_default() {
        let line = Line::from(vec![Span::raw("abc")]);
        assert_eq!(style_at(&line, 99), Style::default());
    }

    // ---------- tab_spans / cue_span / divider_span ----------

    #[test]
    fn tab_spans_uses_selected_style_when_selected() {
        let spans = tab_spans("0:all", true);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, " 0:all ");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tab_spans_uses_dim_style_when_not_selected() {
        let spans = tab_spans("1:db", false);
        assert_eq!(spans[0].content, " 1:db ");
        assert_eq!(spans[0].style.fg, Some(Color::DarkGray));
        assert!(!spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn cue_span_pads_character_with_spaces() {
        assert_eq!(cue_span("‹").content, " ‹ ");
        assert_eq!(cue_span("›").content, " › ");
    }

    #[test]
    fn divider_span_is_single_vertical_bar() {
        assert_eq!(divider_span().content, "│");
    }

    // ---------- highlight_line ----------

    #[test]
    fn highlight_line_splits_single_span_at_range_boundaries() {
        let base = Style::default().fg(Color::White);
        let line = Line::from(vec![Span::styled("hello world", base)]);
        let hi = Style::default().bg(Color::Yellow);
        let out = highlight_line(&line, &[(6, 11, hi)]);
        let texts: Vec<&str> = out.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, vec!["hello ", "world"]);
        assert_eq!(out.spans[0].style.fg, Some(Color::White));
        assert_eq!(out.spans[0].style.bg, None);
        assert_eq!(out.spans[1].style.fg, Some(Color::White));
        assert_eq!(out.spans[1].style.bg, Some(Color::Yellow));
    }

    #[test]
    fn highlight_line_supports_multiple_disjoint_ranges() {
        let line = Line::from(vec![Span::raw("aaaa bbbb cccc")]);
        let hi = Style::default().bg(Color::Yellow);
        let out = highlight_line(&line, &[(0, 4, hi), (10, 14, hi)]);
        let parts: Vec<(String, Option<Color>)> = out
            .spans
            .iter()
            .map(|s| (s.content.to_string(), s.style.bg))
            .collect();
        assert_eq!(
            parts,
            vec![
                ("aaaa".into(), Some(Color::Yellow)),
                (" bbbb ".into(), None),
                ("cccc".into(), Some(Color::Yellow)),
            ]
        );
    }

    #[test]
    fn highlight_line_preserves_existing_span_breaks() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![Span::styled("abc", red), Span::styled("def", blue)]);
        let hi = Style::default().bg(Color::Yellow);
        let out = highlight_line(&line, &[(1, 5, hi)]);
        // Boundaries at 0, 1, 3, 5, 6 → 4 output spans.
        assert_eq!(out.spans.len(), 4);
        assert_eq!(out.spans[0].content, "a");
        assert_eq!(out.spans[1].content, "bc");
        assert_eq!(out.spans[2].content, "de");
        assert_eq!(out.spans[3].content, "f");
        assert_eq!(out.spans[0].style.bg, None);
        assert_eq!(out.spans[0].style.fg, Some(Color::Red));
        assert_eq!(out.spans[1].style.bg, Some(Color::Yellow));
        assert_eq!(out.spans[1].style.fg, Some(Color::Red));
        assert_eq!(out.spans[2].style.bg, Some(Color::Yellow));
        assert_eq!(out.spans[2].style.fg, Some(Color::Blue));
        assert_eq!(out.spans[3].style.bg, None);
        assert_eq!(out.spans[3].style.fg, Some(Color::Blue));
    }

    // ---------- build_line_number_gutter ----------

    #[test]
    fn build_line_number_gutter_empty_returns_empty() {
        let out = build_line_number_gutter(&[], &VecDeque::new(), &[], 0);
        assert!(out.is_empty());
    }

    #[test]
    fn build_line_number_gutter_blanks_consecutive_repeats() {
        // Two rendered rows from input line 1, then one from line 2.
        let numbers: VecDeque<usize> = vec![1, 1, 2].into();
        let render_rows = vec![0, 1, 2];
        let out = build_line_number_gutter(&render_rows, &numbers, &[], 2);
        let texts: Vec<String> = out.iter().map(line_text).collect();
        assert_eq!(texts, vec!["1 │ ", "  │ ", "2 │ "]);
    }

    #[test]
    fn build_line_number_gutter_sizes_width_from_max_line_no() {
        // max_line_no determines width, not the visible numbers.
        let numbers: VecDeque<usize> = vec![1].into();
        let out = build_line_number_gutter(&[0], &numbers, &[], 1000);
        assert_eq!(line_text(&out[0]), "   1 │ ");
    }

    #[test]
    fn build_line_number_gutter_highlights_goto_rows() {
        let numbers: VecDeque<usize> = vec![1, 2].into();
        let mask = vec![false, true];
        let out = build_line_number_gutter(&[0, 1], &numbers, &mask, 2);
        assert_eq!(out[0].spans[0].style.fg, Some(Color::DarkGray));
        assert_eq!(out[1].spans[0].style.fg, Some(Color::Yellow));
        assert!(out[1].spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    // ---------- apply_goto_highlight ----------

    #[test]
    fn apply_goto_highlight_empty_mask_is_noop() {
        let mut lines = vec![Line::from(Span::raw("abc"))];
        apply_goto_highlight(&mut lines, &[]);
        assert_eq!(lines[0].spans[0].style.bg, None);
    }

    #[test]
    fn apply_goto_highlight_sets_bg_on_masked_rows_only() {
        let mut lines = vec![Line::from(Span::raw("a")), Line::from(Span::raw("b"))];
        apply_goto_highlight(&mut lines, &[false, true]);
        assert_eq!(lines[0].spans[0].style.bg, None);
        assert_eq!(lines[1].spans[0].style.bg, Some(Color::DarkGray));
        assert_eq!(lines[1].style.bg, Some(Color::DarkGray));
    }

    #[test]
    fn apply_goto_highlight_does_not_override_existing_bg() {
        // patch() leaves already-set fields alone, so a span with its own
        // background (e.g. a search match) keeps that colour.
        let red_bg = Style::default().bg(Color::Red);
        let mut lines = vec![Line::from(Span::styled("a", red_bg))];
        apply_goto_highlight(&mut lines, &[true]);
        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Red));
    }

    // ---------- apply_search_highlights ----------

    #[test]
    fn apply_search_highlights_no_query_is_noop() {
        let app = App::new(None, true);
        let mut lines = vec![Line::from(Span::raw("hello"))];
        apply_search_highlights(&mut lines, &app, 0);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, "hello");
    }

    #[test]
    fn apply_search_highlights_styles_current_match_with_light_yellow() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["hello world"]);
        app.commit_search("world", false).unwrap();
        let mut lines: Vec<Line<'static>> = app.rendered.iter().cloned().collect();
        apply_search_highlights(&mut lines, &app, 0);
        let hit = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "world")
            .expect("match text should appear as its own span");
        assert_eq!(hit.style.bg, Some(Color::LightYellow));
    }

    #[test]
    fn apply_search_highlights_styles_non_current_with_gray() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        // Visible window covers rows 1..3 — neither is the current match
        // (row 0 is current), so both should get the dim Gray style.
        let mut lines: Vec<Line<'static>> = app.rendered.range(1..3).cloned().collect();
        apply_search_highlights(&mut lines, &app, 1);
        for line in &lines {
            let hit = line
                .spans
                .iter()
                .find(|s| s.content.as_ref() == "foo")
                .expect("match span should be present");
            assert_eq!(hit.style.bg, Some(Color::Gray));
        }
    }

    #[test]
    fn apply_search_highlights_skips_matches_outside_window() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["foo", "foo", "foo"]);
        app.commit_search("foo", false).unwrap();
        // Pass only one line as the visible window, well past the matches.
        let mut lines = vec![Line::from(Span::raw("unrelated"))];
        apply_search_highlights(&mut lines, &app, 99);
        // Nothing in this line matched the search rows → untouched.
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, "unrelated");
        assert_eq!(lines[0].spans[0].style.bg, None);
    }

    // ---------- draw (integration via TestBackend) ----------

    #[test]
    fn draw_normal_mode_shows_tab_strip_and_status_bar() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["hello"]);
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("0:all"), "tab strip missing\n{text}");
        assert!(text.contains("all: 1 lines"), "status bar missing\n{text}");
    }

    #[test]
    fn draw_search_mode_renders_buffer_and_help() {
        let mut app = App::new(None, true);
        let mode = InputMode::Search {
            buffer: "abc".into(),
            is_regex: false,
            error: None,
        };
        let buf = render(&mut app, &mode, 80, 10);
        let text = buffer_text(&buf);
        assert!(
            text.contains("/abc"),
            "search prefix+buffer missing\n{text}"
        );
        assert!(text.contains("Enter: apply"), "help hint missing\n{text}");
    }

    #[test]
    fn draw_search_mode_in_regex_uses_re_prefix() {
        let mut app = App::new(None, true);
        let mode = InputMode::Search {
            buffer: "foo".into(),
            is_regex: true,
            error: None,
        };
        let buf = render(&mut app, &mode, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("re/foo"), "regex prefix missing\n{text}");
    }

    #[test]
    fn draw_search_mode_shows_error_when_present() {
        let mut app = App::new(None, true);
        let mode = InputMode::Search {
            buffer: "(".into(),
            is_regex: true,
            error: Some("bad regex".into()),
        };
        let buf = render(&mut app, &mode, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("[bad regex]"), "error not shown\n{text}");
        // Help hint should NOT appear when an error is displayed.
        assert!(
            !text.contains("Enter: apply"),
            "help should be replaced by error\n{text}"
        );
    }

    #[test]
    fn draw_goto_mode_renders_prefix_buffer_and_help() {
        let mut app = App::new(None, true);
        let mode = InputMode::Goto {
            buffer: "42".into(),
            error: None,
        };
        let buf = render(&mut app, &mode, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains(":42"), "goto prefix+buffer missing\n{text}");
        assert!(text.contains("go to line"), "help hint missing\n{text}");
    }

    #[test]
    fn draw_goto_mode_shows_error_when_present() {
        let mut app = App::new(None, true);
        let mode = InputMode::Goto {
            buffer: "abc".into(),
            error: Some("not a number".into()),
        };
        let buf = render(&mut app, &mode, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("[not a number]"), "error not shown\n{text}");
    }

    #[test]
    fn draw_does_not_panic_on_empty_app() {
        let mut app = App::new(None, false);
        let _ = render(&mut app, &InputMode::Normal, 40, 8);
    }

    #[test]
    fn draw_does_not_panic_on_very_narrow_terminal() {
        // Narrow widths exercise the compute_tab_window fallback path and
        // hscroll clamping — make sure rendering still completes.
        let mut app = App::new(None, true);
        push_lines(
            &mut app,
            &[
                "[db] 1", "[db] 2", "[db] 3", "[auth] 1", "[auth] 2", "[auth] 3",
            ],
        );
        let _ = render(&mut app, &InputMode::Normal, 8, 6);
    }

    #[test]
    fn draw_renders_promoted_category_tab_label() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["[db] a", "[db] b", "[db] c"]);
        assert_eq!(app.log_view_state.category_count(), 1);
        let name = app.log_view_state.get_category(0).name().to_owned();
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(
            text.contains(&format!("1:{name}")),
            "category tab missing\n{text}"
        );
    }

    #[test]
    fn draw_category_pane_uses_category_name_in_title() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["[db] a", "[db] b", "[db] c"]);
        let name = app.log_view_state.get_category(0).name().to_owned();
        app.selected = 1;
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains(&format!(" {name} ")), "title missing\n{text}");
        assert!(
            text.contains(&format!("{name}:")),
            "status label missing\n{text}"
        );
    }

    #[test]
    fn draw_status_bar_shows_search_position_and_total() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["foo", "bar foo"]);
        app.commit_search("foo", false).unwrap();
        // Generous width so the status text isn't clipped.
        let buf = render(&mut app, &InputMode::Normal, 200, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("/foo 1/2"), "match counter missing\n{text}");
    }

    #[test]
    fn draw_renders_line_number_gutter_when_enabled() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["one", "two", "three"]);
        app.show_line_numbers = true;
        let buf = render(&mut app, &InputMode::Normal, 40, 10);
        let text = buffer_text(&buf);
        assert!(
            text.contains("1 │"),
            "gutter row for line 1 missing\n{text}"
        );
        assert!(
            text.contains("2 │"),
            "gutter row for line 2 missing\n{text}"
        );
        assert!(
            text.contains("3 │"),
            "gutter row for line 3 missing\n{text}"
        );
    }

    #[test]
    fn draw_omits_line_number_gutter_when_disabled() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["one", "two", "three"]);
        // show_line_numbers defaults to false.
        let buf = render(&mut app, &InputMode::Normal, 40, 10);
        let text = buffer_text(&buf);
        assert!(!text.contains("1 │"), "gutter should not appear\n{text}");
    }

    #[test]
    fn draw_follow_status_shows_follow_when_following() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["x"]);
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("FOLLOW"), "FOLLOW indicator missing\n{text}");
    }

    #[test]
    fn draw_follow_status_shows_paused_when_not_following() {
        let mut app = App::new(None, true);
        push_lines(&mut app, &["x"]);
        app.log_view_state.main_view_mut().scroll_top(); // disable follow
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(text.contains("PAUSED"), "PAUSED indicator missing\n{text}");
    }

    #[test]
    fn draw_omits_follow_indicator_when_display_follow_is_off() {
        // File mode (no --follow) constructs App with display_follow = false,
        // so neither FOLLOW nor PAUSED should appear in the status bar.
        let mut app = App::new(None, false);
        push_lines(&mut app, &["x"]);
        let buf = render(&mut app, &InputMode::Normal, 80, 10);
        let text = buffer_text(&buf);
        assert!(!text.contains("FOLLOW"), "FOLLOW should be hidden\n{text}");
        assert!(!text.contains("PAUSED"), "PAUSED should be hidden\n{text}");
    }
}
