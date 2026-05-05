mod app;
mod capture;
mod ui;

use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::app::App;
use crate::capture::{LogLine, pipe_into};

fn main() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<LogLine>();

    // Stdin is the upstream log pipe. crossterm opens /dev/tty itself for key
    // input when stdin isn't a tty, so we can hand stdin to the reader thread
    // unmodified.
    pipe_into(io::stdin(), tx);

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let res = run(&mut terminal, &mut app, &rx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &mpsc::Receiver<LogLine>,
) -> anyhow::Result<()> {
    let tick = Duration::from_millis(50);
    let mut last_draw = Instant::now() - tick;

    loop {
        let mut got_data = false;
        while let Ok(line) = rx.try_recv() {
            app.push(line);
            got_data = true;
        }

        if got_data || last_draw.elapsed() >= tick {
            terminal.draw(|f| ui::draw(f, app))?;
            last_draw = Instant::now();
        }

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let viewport = terminal
                    .size()
                    .map(|s| s.height.saturating_sub(3) as usize)
                    .unwrap_or(0);
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => return Ok(()),
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                    (KeyCode::Up, _) => app.scroll_up(1),
                    (KeyCode::Down, _) => app.scroll_down(1, viewport),
                    (KeyCode::PageUp, _) => app.scroll_up(viewport.max(1)),
                    (KeyCode::PageDown, _) => app.scroll_down(viewport.max(1), viewport),
                    (KeyCode::Home, _) => app.scroll_top(),
                    (KeyCode::End, _) => app.scroll_bottom(),
                    _ => {}
                }
            }
        }
    }
}
