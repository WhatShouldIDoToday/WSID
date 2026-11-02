mod args;
mod cli_runner;
mod tui;

use std::io::{self, IsTerminal};

use chrono::Utc;
use clap::Parser;
use wsid_core::{AppConfig, Database, compute_focus_queue, default_config_path, normalize_user, resolve_active_db_path};

use crate::args::{Cli, Commands};
use crate::cli_runner::{OutputMode, execute_command, render_focus_queue};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> wsid_core::Result<()> {
    let cli = Cli::parse();

    let cfg_path = default_config_path();
    let mut config = AppConfig::load_from_path(&cfg_path).unwrap_or_default();

    let db_path = resolve_active_db_path(cli.db.as_deref(), cli.local);
    config.record_recent_workspace(db_path.clone());
    let _ = config.save_to_path(&cfg_path);

    let user = match cli.user.as_deref() {
        Some(u) if !u.trim().is_empty() => normalize_user(u)?,
        _ => config.default_user.clone(),
    };

    let output_mode = OutputMode::from_flags(cli.accessible, cli.json);

    match cli.command {
        None => {
            if io::stdout().is_terminal() && !cli.json {
                tui::run_tui(&db_path, config, user, cli.accessible)
            } else {
                let db = Database::open(&db_path)?;
                let today = Utc::now().date_naive();
                let focus = compute_focus_queue(
                    &db,
                    &user,
                    config.focus_preset,
                    config.custom_weights,
                    Some(config.focus_limit),
                    today,
                    config.scoring,
                )?;
                let mut stdout = io::stdout();
                render_focus_queue(&focus, output_mode, &mut stdout)
            }
        }
        Some(Commands::Tui) => tui::run_tui(&db_path, config, user, cli.accessible),
        Some(Commands::Mcp { verbose }) => {
            let db = Database::open(&db_path)?;
            let mut server = wsid_mcp::McpServer::new(db, config).with_stderr_logging(verbose);
            server.run_stdio()?;
            Ok(())
        }
        Some(cmd) => {
            let mut stdout = io::stdout();
            execute_command(cmd, &db_path, &config, &user, output_mode, &mut stdout)
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tempfile::tempdir;

    use super::*;
    use crate::args::{AddArgs, NextArgs, NoteArgs, SearchArgs};
    use crate::tui::app::{ActiveTab, TuiApp};

    #[test]
    fn cli_commands_execute_and_support_accessible_and_json_modes() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cli_test.db");
        let ext_img = dir.path().join("architecture.png");
        std::fs::write(&ext_img, b"fake-png").unwrap();

        let config = AppConfig::default();

        // 1. Add a task with an attached file and SGDB/Due dates
        let mut buf = Vec::new();
        execute_command(
            Commands::Add(Box::new(AddArgs {
                title: "Implement Kanban view".to_string(),
                description: Some("Support keyboard navigation".to_string()),
                status: None,
                due: Some("2026-11-10".to_string()),
                sgdb: Some("2026-11-05".to_string()),
                estimate: Some("1h30m".to_string()),
                difficulty: Some(3),
                importance: Some(5),
                urgent: true,
                priority: Some(2),
                tags: vec!["ui".to_string(), "rust".to_string()],
                reporter: Some("alice".to_string()),
                assignee: Some("alice".to_string()),
                evaluator: Some("human".to_string()),
                blocks: Vec::new(),
                blocked_by: Vec::new(),
                subtask_of: Vec::new(),
                parent_of: Vec::new(),
                parallel: Vec::new(),
                related: Vec::new(),
                attachments: vec![ext_img.clone()],
                attachment_descriptions: vec!["Kanban architecture diagram".to_string()],
            })),
            &db_path,
            &config,
            "alice",
            OutputMode::Accessible,
            &mut buf,
        )
        .unwrap();

        let add_output = String::from_utf8(buf).unwrap();
        assert!(add_output.contains("Created task "));
        assert!(add_output.contains("Implement Kanban view"));

        // Extract task ID from JSON search
        let mut search_buf = Vec::new();
        execute_command(
            Commands::Search(SearchArgs {
                query: "Kanban".to_string(),
                status: None,
                tag: Some("ui".to_string()),
                assignee: None,
                reporter: None,
                evaluator: None,
                limit: None,
            }),
            &db_path,
            &config,
            "alice",
            OutputMode::Json,
            &mut search_buf,
        )
        .unwrap();
        let hits: serde_json::Value = serde_json::from_slice(&search_buf).unwrap();
        let task_id = hits[0]["task"]["id"].as_str().unwrap().to_string();

        // 2. Check `wsid next --accessible` output (linear screen-reader format, no box-drawing chars)
        let mut next_buf = Vec::new();
        execute_command(
            Commands::Next(NextArgs { preset: None, limit: Some(5) }),
            &db_path,
            &config,
            "alice",
            OutputMode::Accessible,
            &mut next_buf,
        )
        .unwrap();
        let next_str = String::from_utf8(next_buf).unwrap();
        assert!(next_str.contains(&format!("Task {task_id}")));
        assert!(!next_str.contains('╭'));
        assert!(!next_str.contains('│'));

        // 3. Add a note with `@bob` mention and verify accessible show output includes attachments & notes
        let mut note_buf = Vec::new();
        execute_command(
            Commands::Note(NoteArgs {
                id: task_id.clone(),
                body: "Waiting on @bob for tokens".to_string(),
                author: None,
                no_spawn: false,
                attachments: Vec::new(),
                attachment_descriptions: Vec::new(),
            }),
            &db_path,
            &config,
            "alice",
            OutputMode::Accessible,
            &mut note_buf,
        )
        .unwrap();
        let note_str = String::from_utf8(note_buf).unwrap();
        assert!(note_str.contains("Spawned mention task"));

        // 4. Show task in accessible mode and verify attachment alt text and disk path are present
        let mut show_buf = Vec::new();
        execute_command(Commands::Show { id: task_id.clone() }, &db_path, &config, "alice", OutputMode::Accessible, &mut show_buf).unwrap();
        let show_str = String::from_utf8(show_buf).unwrap();
        assert!(show_str.contains("Kanban architecture diagram"));
        assert!(show_str.contains(&ext_img.display().to_string()));

        // 5. Verify `wsid about` outputs version, Apache-2.0 license, repository URL, and third-party crates
        let mut about_buf = Vec::new();
        execute_command(Commands::About { full: true }, &db_path, &config, "alice", OutputMode::Accessible, &mut about_buf).unwrap();
        let about_str = String::from_utf8(about_buf).unwrap();
        assert!(about_str.contains("Apache-2.0"));
        assert!(about_str.contains("https://github.com/dmbeta/WSID"));
        assert!(about_str.contains("ratatui"));
    }

    #[test]
    fn tui_renders_all_five_views_and_detail_modal_and_detects_external_db_changes() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_test.db");
        let config = AppConfig::default();

        // Seed a task
        let mut buf = Vec::new();
        execute_command(
            Commands::Add(Box::new(AddArgs {
                title: "Initial Focus Task".to_string(),
                description: Some("First task in TUI".to_string()),
                status: None,
                due: Some("2026-11-03".to_string()),
                sgdb: None,
                estimate: Some("30m".to_string()),
                difficulty: Some(2),
                importance: Some(4),
                urgent: false,
                priority: None,
                tags: vec!["core".to_string()],
                reporter: Some("alice".to_string()),
                assignee: Some("alice".to_string()),
                evaluator: Some("alice".to_string()),
                blocks: Vec::new(),
                blocked_by: Vec::new(),
                subtask_of: Vec::new(),
                parent_of: Vec::new(),
                parallel: Vec::new(),
                related: Vec::new(),
                attachments: Vec::new(),
                attachment_descriptions: Vec::new(),
            })),
            &db_path,
            &config,
            "alice",
            OutputMode::Standard,
            &mut buf,
        )
        .unwrap();

        let mut app = TuiApp::new(&db_path, config.clone(), "alice".to_string(), false).unwrap();
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Focus View
        assert_eq!(app.active_tab, ActiveTab::Focus);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // Open Detail Modal on selected task with Enter, then close with Esc
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.detail_modal.is_some());
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.detail_modal.is_none());

        // 2. Switch to Kanban View ('2')
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Kanban);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // 3. Switch to Eisenhower View ('3')
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Eisenhower);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // 4. Switch to Database Search View ('4')
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Database);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // 5. Switch to Data Entry View ('5')
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::DataEntry);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // Toggle accessible mode ('a' back in Focus view)
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(app.accessible_mode);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();

        // Open About & Third-Party Licenses modal ('?'), scroll ('j'), and close ('Esc')
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.about_modal_open);
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.about_scroll, 1);
        terminal.draw(|f| crate::tui::ui::render(f, &app)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.about_modal_open);

        // External write via a second Database connection should be detected on `on_tick()`
        let mut external_db = Database::open(&db_path).unwrap();
        wsid_core::create_task(
            &mut external_db,
            wsid_core::CreateTaskInput {
                title: "External MCP Created Task".to_string(),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();

        assert!(app.on_tick().unwrap());
        assert_eq!(app.focus_queue.actionable_tasks.len(), 2);
    }
}
