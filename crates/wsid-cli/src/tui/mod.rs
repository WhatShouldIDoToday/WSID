pub mod app;
pub mod ui;

use std::io;
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use wsid_core::AppConfig;

use crate::tui::app::TuiApp;

pub fn run_tui(db_path: &Path, config: AppConfig, persona: String, accessible_mode: bool) -> wsid_core::Result<()> {
    let mut app = TuiApp::new(db_path, config, persona, accessible_mode)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_event_loop<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut TuiApp) -> wsid_core::Result<()> {
    let tick_rate = Duration::from_millis(250);
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.handle_key(key);
            }
        } else {
            let _ = app.on_tick();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
