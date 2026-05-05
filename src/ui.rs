use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
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
    draw_status(f, app, chunks[2]);
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
    let (lines, total): (Vec<Line<'static>>, usize) = if app.selected == 0 {
        (app.rendered.clone(), app.rendered.len())
    } else {
        let cat = &app.categories[app.selected - 1];
        (
            cat.indices.iter().map(|&i| app.rendered[i].clone()).collect(),
            cat.indices.len(),
        )
    };

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

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let (label, total, view) = if app.selected == 0 {
        ("all", app.rendered.len(), &app.main)
    } else {
        let cat = &app.categories[app.selected - 1];
        (cat.name.as_str(), cat.indices.len(), &cat.view)
    };
    let s = format!(
        " {label}: {total} lines · {} · q quit · ↑/↓ PgUp/PgDn scroll · End follow · Tab/0-9 panes ",
        if view.follow { "FOLLOW" } else { "PAUSED" }
    );
    f.render_widget(
        Paragraph::new(s).style(Style::default().fg(Color::Black).bg(Color::Cyan)),
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
