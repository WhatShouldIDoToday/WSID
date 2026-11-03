pub mod app;
pub mod form;
pub mod ui;

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use ratatui::Terminal;
use wsid_core::AppConfig;

use crate::tui::app::TuiApp;

pub fn run_tui(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<()> {
    let mut app = TuiApp::new(db_path, config, user, accessible_mode)?;

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let res = run_event_loop(&mut terminal, &mut app);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    res
}

fn run_event_loop<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut TuiApp) -> wsid_core::Result<()> {
    let tick_rate = Duration::from_millis(250);
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(tick_rate)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Paste(text) => app.handle_paste(&text),
                _ => {}
            }
        } else {
            let _ = app.on_tick();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
