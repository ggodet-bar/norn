use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let block = Block::default().borders(Borders::ALL).title(" log ");
    let inner = block.inner(chunks[0]);
    let viewport = inner.height as usize;

    let total = app.rendered.len();
    let max_scroll = total.saturating_sub(viewport);
    if app.follow {
        app.scroll = max_scroll;
    } else {
        app.scroll = app.scroll.min(max_scroll);
    }

    let paragraph = Paragraph::new(app.rendered.clone())
        .block(block)
        .scroll((app.scroll as u16, 0));
    f.render_widget(paragraph, chunks[0]);

    let status = format!(
        " {} lines · {} · q quit · ↑/↓ PgUp/PgDn scroll · End follow · Home top ",
        total,
        if app.follow { "FOLLOW" } else { "PAUSED" },
    );
    let status_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    f.render_widget(
        Paragraph::new(status).style(status_style),
        chunks[1],
    );
}
