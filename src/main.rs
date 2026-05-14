mod app;
mod capture;
mod ui;

use std::{
    fs::File,
    io::{self, IsTerminal},
    path::PathBuf,
    str::FromStr,
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::App,
    capture::{LogLine, pipe_into, tail_into},
};

const DEFAULT_MAX_LINES: usize = 10_000;

struct Args {
    /// `None` when the user passes `0` to waive the limit
    max_lines: Option<usize>,
    path: Option<PathBuf>,
    no_line_numbers: bool,
    follow: bool,
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;

    let (tx, rx) = mpsc::channel::<LogLine>();
    // A file path overrides stdin; in that mode the buffer is unbounded
    // because the file is finite and the user expects to scroll all of it.
    let file_mode = args.path.is_some();
    let (max_lines, display_follow) = match &args.path {
        Some(path) => {
            let file = File::open(path)?;
            if args.follow {
                tail_into(file, tx);
                // A growing file under --follow needs the same buffer cap as
                // stdin or it can grow without bound.
                (args.max_lines, true)
            } else {
                pipe_into(file, tx);
                // A finite file is shown in full; --max-lines is rejected at
                // parse time for this mode.
                (None, false)
            }
        }
        None => {
            if io::stdin().is_terminal() {
                eprintln!("Missing filename. Run `splog --help` for usage.");
                std::process::exit(2);
            }
            pipe_into(io::stdin(), tx);
            (args.max_lines, true)
        }
    };

    install_panic_hook();
    let (_guard, mut terminal) = TerminalGuard::new()?;

    let mut app = App::new(max_lines, display_follow);
    app.show_line_numbers = file_mode && !args.no_line_numbers;
    run(&mut terminal, &mut app, &rx, file_mode)
}

/// RAII guard for the terminal's raw-mode + alternate-screen state.
/// `Drop` restores the terminal even on panic or early-return error
/// paths so the user's shell isn't left mid-altscreen with input
/// echo disabled.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> anyhow::Result<(Self, Terminal<CrosstermBackend<io::Stdout>>)> {
        enable_raw_mode()?;
        // Build the guard before any further fallible setup so a
        // failure in EnterAlternateScreen / Terminal::new still
        // triggers cleanup via Drop.
        let guard = Self;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok((guard, terminal))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// Wrap the default panic hook so the terminal is restored before the
/// panic message is printed. Without this, panic output lands in the
/// alternate screen and disappears when the shell takes over again.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        original(info);
    }));
}

/// Parse CLI args. Returns the configured max-lines cap: `Some(n)` for a
/// bounded buffer, `None` when the user passes `0` to waive the limit.
fn parse_args() -> anyhow::Result<Args> {
    let mut args = std::env::args().enumerate().skip(1);
    let mut max_lines: Option<usize> = Some(DEFAULT_MAX_LINES);
    let mut max_lines_explicit = false;
    let mut path: Option<PathBuf> = None;
    let mut no_line_numbers = false;
    let mut follow = false;
    while let Some((idx, arg)) = args.next() {
        match arg.as_str() {
            "-n" | "--max-lines" => {
                let (_, value) = args
                    .next()
                    .ok_or_else(|| anyhow!("{arg} requires a value"))?;
                let n: usize = value
                    .parse()
                    .map_err(|e| anyhow!("{arg}: invalid integer {value:?}: {e}"))?;
                max_lines = if n == 0 { None } else { Some(n) };
                max_lines_explicit = true;
            }
            "-N" | "--no-line-numbers" => {
                no_line_numbers = true;
            }
            "-f" | "--follow" => {
                follow = true;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("splog {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if idx == 1 => {
                let path_arg = PathBuf::from_str(&arg)
                    .map_err(|e| anyhow!("{arg}: invalid file path: {e}"))?;
                path = Some(path_arg);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    if follow && path.is_none() {
        return Err(anyhow!(
            "--follow/-f requires a file path; cannot tail stdin"
        ));
    }
    if max_lines_explicit && path.is_some() && !follow {
        return Err(anyhow!(
            "--max-lines/-n only applies to stdin or --follow; a finite file is shown in full"
        ));
    }
    Ok(Args {
        max_lines,
        path,
        no_line_numbers,
        follow,
    })
}

fn print_help() {
    println!(
        "splog — TUI log viewer that splits lines into categories\n\
         \n\
         Usage: splog [FILEPATH] [OPTIONS]\n\
         \n\
         Options:\n  \
           -n, --max-lines N   retain at most N display rows; 0 = unlimited \
           (default: {DEFAULT_MAX_LINES})\n  \
           -N, --no-line-numbers   hide the line-number gutter (file mode only)\n  \
           -f, --follow        keep reading the file as new lines are appended (file mode only)\n  \
           -V, --version       print version and exit\n  \
           -h, --help          show this help"
    );
}

/// Editing state for the search bar. `Normal` is the default mode where
/// scroll/tab/quit keys apply. `Search` opens on `/`; while it's active all
/// non-control character keys go into the buffer instead of triggering
/// app-wide actions.
pub enum InputMode {
    Normal,
    Search {
        buffer: String,
        is_regex: bool,
        error: Option<String>,
    },
    Goto {
        buffer: String,
        error: Option<String>,
    },
}

fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &mpsc::Receiver<LogLine>,
    file_mode: bool,
) -> anyhow::Result<()> {
    let tick = Duration::from_millis(50);
    // Producers that emit cursor-control sequences in tight bursts can leave
    // the terminal with stale cells ratatui's diff renderer won't repaint.
    // The bursts overwhelmingly come from warmup/compile output, so a single
    // forced clear ~2s after the first input is enough — after that, the
    // diff renderer keeps up on its own. File input doesn't have this
    // problem (no live producer, no cursor-control bursts), so the clear
    // is suppressed there to avoid a visible flicker.
    let warmup_redraw_delay = Duration::from_secs(2);
    let mut last_draw = Instant::now() - tick;
    let mut first_input_at: Option<Instant> = None;
    let mut warmup_redraw_done = file_mode;
    let mut input_mode = InputMode::Normal;

    loop {
        let mut got_data = false;
        while let Ok(line) = rx.try_recv() {
            app.push(line);
            got_data = true;
        }
        if got_data && first_input_at.is_none() {
            first_input_at = Some(Instant::now());
        }

        if got_data || last_draw.elapsed() >= tick {
            if !warmup_redraw_done
                && first_input_at.is_some_and(|t| t.elapsed() >= warmup_redraw_delay)
            {
                terminal
                    .clear()
                    .map_err(|e| anyhow!("failed to clear terminal: {e}"))?;
                warmup_redraw_done = true;
            }
            terminal
                .draw(|f| ui::draw(f, app, &input_mode))
                .map_err(|e| anyhow!("failed to draw terminal: {e}"))?;
            last_draw = Instant::now();
        }

        if event::poll(tick)?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            // The goto highlight is one-shot: any new keypress clears
            // the previous goto's flag before the handler runs, so this
            // very keypress's goto-Enter (if any) can re-set it.
            app.clear_goto_highlight();
            let viewport = terminal
                .size()
                .map(|s| s.height.saturating_sub(3) as usize)
                .unwrap_or(0);
            // Scroll keys always pass through, even mid-search edit, so
            // the user can keep their bearings while composing a query.
            if handle_scroll_key(&key, app, viewport) {
                continue;
            }
            let prev = std::mem::replace(&mut input_mode, InputMode::Normal);
            match handle_key(prev, key, app, viewport) {
                Some(next) => input_mode = next,
                None => return Ok(()),
            }
        }
    }
}

/// Scroll-family keys behave the same way in every input mode so the user
/// can keep their bearings while composing a search query. Returns `true`
/// when a key was consumed.
fn handle_scroll_key(key: &KeyEvent, app: &mut App, viewport: usize) -> bool {
    match key.code {
        KeyCode::Up => app.scroll_up(1),
        KeyCode::Down => app.scroll_down(1, viewport),
        KeyCode::Left => app.scroll_left(1),
        KeyCode::Right => app.scroll_right(1),
        KeyCode::PageUp => app.scroll_up(viewport.max(1)),
        KeyCode::PageDown => app.scroll_down(viewport.max(1), viewport),
        KeyCode::Home => app.scroll_top(),
        KeyCode::End => app.scroll_bottom(),
        _ => return false,
    }
    true
}

/// Dispatch a key to the appropriate per-mode handler. Returns `None` to
/// quit the run loop or `Some(next)` for the next mode.
fn handle_key(mode: InputMode, key: KeyEvent, app: &mut App, viewport: usize) -> Option<InputMode> {
    match mode {
        InputMode::Search {
            buffer,
            is_regex,
            error,
        } => handle_search_key(buffer, is_regex, error, key, app, viewport),
        InputMode::Goto { buffer, error } => handle_goto_key(buffer, error, key, app, viewport),
        InputMode::Normal => handle_normal_key(key, app, viewport),
    }
}

fn handle_search_key(
    mut buffer: String,
    mut is_regex: bool,
    error: Option<String>,
    key: KeyEvent,
    app: &mut App,
    viewport: usize,
) -> Option<InputMode> {
    let stay = |buffer, is_regex, error| {
        Some(InputMode::Search {
            buffer,
            is_regex,
            error,
        })
    };
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(InputMode::Normal),
        (KeyCode::Enter, _) => match app.commit_search(&buffer, is_regex) {
            Ok(_) => {
                if let Some(row) = current_match_row(app) {
                    scroll_to_row(app, row, viewport);
                }
                Some(InputMode::Normal)
            }
            Err(e) => stay(buffer, is_regex, Some(e.to_string())),
        },
        (KeyCode::Backspace, _) => {
            buffer.pop();
            stay(buffer, is_regex, None)
        }
        (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
            is_regex = !is_regex;
            stay(buffer, is_regex, None)
        }
        // Bare characters (and shifted ones; SHIFT comes through as a
        // modifier alongside the uppercased Char) extend the buffer. Any
        // other modifier combination is reserved for future bindings.
        (KeyCode::Char(c), m) if (m - KeyModifiers::SHIFT).is_empty() => {
            buffer.push(c);
            stay(buffer, is_regex, None)
        }
        _ => stay(buffer, is_regex, error),
    }
}

fn handle_goto_key(
    mut buffer: String,
    error: Option<String>,
    key: KeyEvent,
    app: &mut App,
    viewport: usize,
) -> Option<InputMode> {
    let stay = |buffer, error| Some(InputMode::Goto { buffer, error });
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(InputMode::Normal),
        (KeyCode::Enter, _) => {
            if buffer.is_empty() {
                return Some(InputMode::Normal);
            }
            match buffer.parse::<usize>() {
                Ok(target) => {
                    if let Some(row) = app.goto_input_line(target) {
                        scroll_to_row(app, row, viewport);
                    }
                    Some(InputMode::Normal)
                }
                Err(e) => stay(buffer, Some(e.to_string())),
            }
        }
        (KeyCode::Backspace, _) => {
            buffer.pop();
            stay(buffer, None)
        }
        (KeyCode::Char(c), m) if c.is_ascii_digit() && (m - KeyModifiers::SHIFT).is_empty() => {
            buffer.push(c);
            stay(buffer, None)
        }
        _ => stay(buffer, error),
    }
}

fn handle_normal_key(key: KeyEvent, app: &mut App, viewport: usize) -> Option<InputMode> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => None,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => None,
        // Esc clears an active search before falling back to quit, so
        // pressing it once "exits" search mode the way users expect.
        (KeyCode::Esc, _) => {
            if app.active_search().query().is_some() {
                app.clear_search();
                Some(InputMode::Normal)
            } else {
                None
            }
        }
        (KeyCode::Char('/'), _) => Some(InputMode::Search {
            buffer: String::new(),
            is_regex: false,
            error: None,
        }),
        (KeyCode::Char(':'), _) => Some(InputMode::Goto {
            buffer: String::new(),
            error: None,
        }),
        (KeyCode::Char('g'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.scroll_top();
            Some(InputMode::Normal)
        }
        (KeyCode::Char('G'), _) => {
            app.scroll_bottom();
            Some(InputMode::Normal)
        }
        (KeyCode::Char('j'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.scroll_down(1, viewport);
            Some(InputMode::Normal)
        }
        (KeyCode::Char('k'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.scroll_up(1);
            Some(InputMode::Normal)
        }
        (KeyCode::Char('h'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.scroll_left(1);
            Some(InputMode::Normal)
        }
        (KeyCode::Char('l'), m) if !m.contains(KeyModifiers::CONTROL) => {
            app.scroll_right(1);
            Some(InputMode::Normal)
        }
        (KeyCode::Char('n'), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(row) = app.search_next() {
                scroll_to_row(app, row, viewport);
            }
            Some(InputMode::Normal)
        }
        (KeyCode::Char('N'), _) => {
            if let Some(row) = app.search_prev() {
                scroll_to_row(app, row, viewport);
            }
            Some(InputMode::Normal)
        }
        (KeyCode::Char('x'), KeyModifiers::CONTROL) => {
            app.ignore_active_category();
            Some(InputMode::Normal)
        }
        // Promote the active search to its own category pane. The pane is
        // backed by the search regex so future matching pushes flow in.
        // No-op when no search is active.
        (KeyCode::Char('c'), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(q) = app.active_search().query() {
                let raw = q.raw_query().to_owned();
                let is_regex = q.is_regex();
                let _ = app.promote_search_to_category(&raw, is_regex);
            }
            Some(InputMode::Normal)
        }
        (KeyCode::Tab, _) => {
            app.next_tab();
            Some(InputMode::Normal)
        }
        (KeyCode::BackTab, _) => {
            app.prev_tab();
            Some(InputMode::Normal)
        }
        (KeyCode::Char(c), _) if c.is_ascii_digit() => {
            let d = c.to_digit(10).unwrap() as usize;
            app.select_tab(d);
            Some(InputMode::Normal)
        }
        _ => Some(InputMode::Normal),
    }
}

fn current_match_row(app: &App) -> Option<usize> {
    let s = app.active_search();
    s.current_match_row()
}

fn scroll_to_row(app: &mut App, row: usize, viewport: usize) {
    let (view, total) = app.active_view_mut();
    let half = viewport / 2;
    let max_scroll = total.saturating_sub(viewport);
    view.scroll_to_row(row.saturating_sub(half).min(max_scroll));
}
