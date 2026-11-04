pub mod app;
pub mod form;
pub mod text_edit;
pub mod ui;

use std::path::Path;
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal;
use ratatui::Terminal;
use wsid_core::AppConfig;

use crate::tui::app::TuiApp;

pub fn run_tui(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<()> {
    let mut app = TuiApp::new(db_path, config, user, accessible_mode)?;

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    // Kitty keyboard protocol reports `Cmd`-based chords (undo/redo on macOS) and unambiguous
    // modifiers. Terminals that do not support it keep their legacy encodings, which
    // `text_edit::apply_text_shortcut` already accepts.
    let enhanced_keys = is_keyboard_enhanced();
    if enhanced_keys {
        let _ = execute!(std::io::stdout(), PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }
    let res = run_event_loop(&mut terminal, &mut app);
    if enhanced_keys {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    res
}

/// Whether the terminal understands the kitty keyboard protocol.
fn is_keyboard_enhanced() -> bool {
    terminal::supports_keyboard_enhancement().unwrap_or(false)
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
