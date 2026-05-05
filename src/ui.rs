use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
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
        InputMode::Normal => draw_status(f, app, chunks[2]),
    }
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = std::iter::once(Line::from("0:all"))
        .chain(app.categories.iter().enumerate().map(|(i, c)| {
            Line::from(format!("{}:{}", i + 1, truncate(&c.name, 20)))
        }))
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
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
    let (mut lines, total): (Vec<Line<'static>>, usize) = if app.selected == 0 {
        (app.rendered.clone(), app.rendered.len())
    } else {
        let cat = &app.categories[app.selected - 1];
        (
            cat.indices.iter().map(|&i| app.rendered[i].clone()).collect(),
            cat.indices.len(),
        )
    };

    apply_search_highlights(&mut lines, app);

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
    s.push_str("· q quit · / search · n/N next/prev · ↑/↓ PgUp/PgDn scroll · End follow · Tab/0-9 panes ");
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}
