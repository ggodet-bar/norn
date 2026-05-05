mod app;
mod capture;
mod categorize;
mod ui;

use std::io::{self, IsTerminal};
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

const DEFAULT_MAX_LINES: usize = 10_000;

fn main() -> anyhow::Result<()> {
    let max_lines = parse_args()?;

    if io::stdin().is_terminal() {
        eprintln!("Missing filename. Run `norn --help` for usage.");
        std::process::exit(2);
    }

    let (tx, rx) = mpsc::channel::<LogLine>();
    pipe_into(io::stdin(), tx);

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(max_lines);
    let res = run(&mut terminal, &mut app, &rx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

/// Parse CLI args. Returns the configured max-lines cap: `Some(n)` for a
/// bounded buffer, `None` when the user passes `0` to waive the limit.
fn parse_args() -> anyhow::Result<Option<usize>> {
    let mut args = std::env::args().skip(1);
    let mut max_lines: Option<usize> = Some(DEFAULT_MAX_LINES);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-n" | "--max-lines" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{arg} requires a value"))?;
                let n: usize = value
                    .parse()
                    .map_err(|e| anyhow::anyhow!("{arg}: invalid integer {value:?}: {e}"))?;
                max_lines = if n == 0 { None } else { Some(n) };
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(max_lines)
}

fn print_help() {
    println!(
        "norn — TUI log viewer that reads from stdin\n\
         \n\
         Usage: norn [OPTIONS]\n\
         \n\
         Options:\n  \
           -n, --max-lines N   retain at most N display rows; 0 = unlimited \
           (default: {DEFAULT_MAX_LINES})\n  \
           -h, --help          show this help"
    );
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
                    (KeyCode::Tab, _) => app.next_tab(),
                    (KeyCode::BackTab, _) => app.prev_tab(),
                    (KeyCode::Char(c), _) if c.is_ascii_digit() => {
                        let d = c.to_digit(10).unwrap() as usize;
                        app.select_tab(d);
                    }
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
