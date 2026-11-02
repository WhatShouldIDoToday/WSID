pub mod app;
pub mod ui;

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Terminal;
use wsid_core::AppConfig;

use crate::tui::app::TuiApp;

pub fn run_tui(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<()> {
    let mut app = TuiApp::new(db_path, config, user, accessible_mode)?;

    let mut terminal = ratatui::init();
    let res = run_event_loop(&mut terminal, &mut app);
    ratatui::restore();

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
