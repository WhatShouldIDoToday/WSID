mod args;
mod cli_runner;
mod tui;

use std::io::{self, IsTerminal};

use chrono::Utc;
use clap::Parser;
use wsid_core::{AppConfig, Database, compute_focus_queue_in_project, default_config_path, normalize_user, resolve_active_db_path};

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
        _ => normalize_user(&config.default_user)?,
    };

    let output_mode = OutputMode::from_flags(cli.accessible, cli.json);

    match cli.command {
        None => {
            if io::stdout().is_terminal() && !cli.json {
                tui::run_tui(&db_path, config, user, cli.accessible)
            } else {
                let db = Database::open(&db_path)?;
                let today = Utc::now().date_naive();
                let focus = compute_focus_queue_in_project(
                    &db,
                    &user,
                    config.focus_preset,
                    config.custom_weights,
                    Some(config.focus_limit),
                    today,
                    config.scoring,
                    Some(wsid_core::ALL_PROJECTS),
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
    use crate::tui::app::{ActiveTab, PickerMode, SettingsAction, SettingsMode, TuiApp};
    use wsid_core::{CreateTaskInput, EisenhowerQuadrant};

    #[test]
    fn tui_edit_opens_prefilled_form_and_saves_through_update() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("edit.db");
        let task_id = {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(
                &mut db,
                CreateTaskInput {
                    title: "Original title".to_string(),
                    description: Some("Original description".to_string()),
                    importance: Some(4),
                    tags: vec!["backend".to_string()],
                    due_date: Some("2099-01-05".to_string()),
                    time_estimate_minutes: Some(200),
                    ..Default::default()
                },
                "alice",
            )
            .unwrap()
            .id
        };

        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        app.open_detail_for(&task_id);
        press(&mut app, KeyCode::Char('E'));
        assert_eq!(app.active_tab, ActiveTab::DataEntry);
        assert_eq!(app.editing_task_id.as_deref(), Some(task_id.as_str()));
        assert!(app.detail_modal.is_none());

        let form = &app.entry_form;
        assert_eq!(form.fields[0], "Original title");
        assert_eq!(form.fields[1], "Original description");
        assert_eq!(form.fields[6], "4");
        assert_eq!(form.fields[9], "backend");
        assert_eq!(form.fields[4], "3h 20m");
        assert!(form.fields[2].starts_with("2099-01-05 20:00"), "due prefilled in local time: {}", form.fields[2]);

        // Save the edited values through the same Ctrl+S path as creation.
        app.entry_form.fields[0] = "Renamed title".to_string();
        app.entry_form.fields[6] = "5".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(app.editing_task_id.is_none());
        assert!(app.form_errors.is_none());
        assert_eq!(app.active_tab, ActiveTab::Focus, "returns to the tab the edit started from");
        assert_eq!(app.detail_modal.as_ref().map(|d| d.task.id.clone()), Some(task_id.clone()));

        let db = Database::open(&db_path).unwrap();
        let saved = wsid_core::get_task(&db, &task_id).unwrap();
        assert_eq!(saved.title, "Renamed title");
        assert_eq!(saved.importance, 5);
        assert_eq!(saved.description, "Original description");
        assert_eq!(saved.tags, vec!["backend".to_string()]);

        // Esc cancels editing and reopens the task detail without writing anything.
        app.open_detail_for(&task_id);
        press(&mut app, KeyCode::Char('E'));
        app.entry_form.fields[0] = "Discarded".to_string();
        press(&mut app, KeyCode::Esc);
        assert!(app.editing_task_id.is_none());
        assert_eq!(app.detail_modal.as_ref().map(|d| d.task.title.clone()), Some("Renamed title".to_string()));
        let db = Database::open(&db_path).unwrap();
        assert_eq!(wsid_core::get_task(&db, &task_id).unwrap().title, "Renamed title");
    }

    #[test]
    fn tui_eisenhower_shift_tab_wraps_and_urgency_keys_move_columns() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("eisen.db");
        let task_id = {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(
                &mut db,
                CreateTaskInput {
                    title: "Quadrant mover".to_string(),
                    importance: Some(4),
                    urgent_override: Some(true),
                    ..Default::default()
                },
                "alice",
            )
            .unwrap()
            .id
        };

        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        // `3` opens the Eisenhower matrix; Shift+Tab steps back, Tab steps forward.
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.active_tab, ActiveTab::Eisenhower);
        assert_eq!(app.eisenhower_quadrant, 0);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.eisenhower_quadrant, 3, "Shift+Tab moves to the previous quadrant");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.eisenhower_quadrant, 0);

        let quadrant_of = |app: &TuiApp, id: &str| {
            app.eisenhower_items
                .iter()
                .find(|c| c.task.id == id)
                .map(|c| c.quadrant)
        };
        let focus_quadrant = |app: &mut TuiApp, target: usize| {
            while app.eisenhower_quadrant != target {
                press(app, KeyCode::Tab);
            }
        };
        assert_eq!(quadrant_of(&app, &task_id), Some(EisenhowerQuadrant::Q1DoFirst));

        // `<` forces not-urgent (left column), `>` forces urgent (right column).
        press(&mut app, KeyCode::Char('<'));
        assert_eq!(quadrant_of(&app, &task_id), Some(EisenhowerQuadrant::Q2Schedule), "{}", app.status_message);

        focus_quadrant(&mut app, 1);
        press(&mut app, KeyCode::Char('>'));
        assert_eq!(quadrant_of(&app, &task_id), Some(EisenhowerQuadrant::Q1DoFirst), "{}", app.status_message);

        // Importance still moves between rows, so every quadrant is reachable.
        focus_quadrant(&mut app, 0);
        press(&mut app, KeyCode::Char('-'));
        press(&mut app, KeyCode::Char('-'));
        assert_eq!(quadrant_of(&app, &task_id), Some(EisenhowerQuadrant::Q3DelegateQuick));

        focus_quadrant(&mut app, 2);
        press(&mut app, KeyCode::Char('<'));
        assert_eq!(quadrant_of(&app, &task_id), Some(EisenhowerQuadrant::Q4Later), "{}", app.status_message);
    }

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
                due: Some("2099-11-10".to_string()),
                sgdb: Some("2099-11-05".to_string()),
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
                project: None,
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
                project: None,
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
            Commands::Next(NextArgs { preset: None, limit: Some(5), project: None }),
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
    fn cli_edit_users_kanban_and_multi_delete() {
        use crate::args::{EditArgs, UsersAction};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cli_bulk.db");
        let config = AppConfig { primary_user: "alice".to_string(), default_user: "alice".to_string(), ..AppConfig::default() };
        let run = |cmd: Commands, user: &str| -> wsid_core::Result<String> {
            let mut buf = Vec::new();
            execute_command(cmd, &db_path, &config, user, OutputMode::Json, &mut buf)?;
            Ok(String::from_utf8(buf).unwrap())
        };
        let add = |title: &str| {
            Commands::Add(Box::new(AddArgs {
                title: title.to_string(),
                description: None,
                status: None,
                due: None,
                sgdb: None,
                estimate: None,
                difficulty: None,
                importance: Some(1),
                urgent: false,
                priority: None,
                tags: Vec::new(),
                project: None,
                reporter: None,
                assignee: Some("bob".to_string()),
                evaluator: None,
                blocks: Vec::new(),
                blocked_by: Vec::new(),
                subtask_of: Vec::new(),
                parent_of: Vec::new(),
                parallel: Vec::new(),
                related: Vec::new(),
                attachments: Vec::new(),
                attachment_descriptions: Vec::new(),
            }))
        };
        let id_of = |json: &str| -> String {
            let v: serde_json::Value = serde_json::from_str(json).unwrap();
            v["task"]["id"]
                .as_str()
                .or_else(|| v["id"].as_str())
                .unwrap()
                .to_string()
        };
        let a = id_of(&run(add("A"), "alice").unwrap());
        let b = id_of(&run(add("B"), "alice").unwrap());

        let edited = run(
            Commands::Edit(EditArgs {
                ids: vec![a.clone(), b.clone()],
                status: Some("todo".to_string()),
                assignee: None,
                importance: Some(4),
                difficulty: None,
                urgent: false,
                not_urgent: false,
                add_tags: vec!["batch".to_string()],
                remove_tags: Vec::new(),
                project: None,
            }),
            "alice",
        )
        .unwrap();
        assert!(edited.contains("\"importance\": 4"));

        let kanban = run(Commands::Kanban { all: true }, "alice").unwrap();
        assert!(kanban.contains(&a) && kanban.contains(&b));

        // Only the main user may delete users; others get an error.
        assert!(
            run(Commands::Users { action: UsersAction::Delete { user: "bob".to_string(), delete_authored: false, yes: true } }, "bob")
                .is_err()
        );
        let done =
            run(Commands::Users { action: UsersAction::Delete { user: "bob".to_string(), delete_authored: false, yes: true } }, "alice")
                .unwrap();
        assert!(done.contains("reassigned_task_count"));
        let users = run(Commands::Users { action: UsersAction::List }, "alice").unwrap();
        assert!(!users.contains("bob"));

        run(Commands::Delete { ids: vec![a, b] }, "alice").unwrap();
    }

    #[test]
    fn tui_renders_all_six_views_and_detail_modal_and_detects_external_db_changes() {
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
                due: Some("2099-11-03".to_string()),
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
                project: None,
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
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();

        // Open Detail Modal on selected task with Enter, then close with Esc
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.detail_modal.is_some());
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.detail_modal.is_none());

        // 2. Switch to Kanban View ('2')
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Kanban);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();

        // 3. Switch to Eisenhower View ('3')
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Eisenhower);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();

        // 4. Switch to Database Search View ('4')
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Database);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();

        // 5. Switch to Data Entry View ('5')
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::DataEntry);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();

        // 6. Switch to Settings View ('6')
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::NONE));
        assert_eq!(app.active_tab, ActiveTab::Settings);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Active Context") && rendered.contains("Focus Queue"), "settings sections render: {rendered}");

        // Toggle accessible mode ('a') while Settings is open.
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(app.accessible_mode);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        for cell in terminal.backend().buffer().content() {
            assert!(
                !matches!(cell.symbol(), "╭" | "╮" | "╰" | "╯" | "┌" | "┐" | "└" | "┘" | "─"),
                "unexpected box-drawing symbol '{}' in accessible mode",
                cell.symbol()
            );
        }

        // Open About & Third-Party Licenses modal ('?'), scroll ('j'), and close ('Esc')
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.about_modal_open);
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.about_scroll, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
        assert_eq!(app.about_scroll, u16::MAX, "G asks for the last row");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(app.about_scroll > 1 && app.about_scroll < u16::MAX, "G clamps to the wrapped bottom: {}", app.about_scroll);
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.about_scroll, 0, "g jumps back to the top");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
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

    #[test]
    fn tui_multi_select_and_bulk_edit() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_bulk.db");
        let config = AppConfig::default();
        {
            let mut db = Database::open(&db_path).unwrap();
            for title in ["One", "Two"] {
                wsid_core::create_task(
                    &mut db,
                    wsid_core::CreateTaskInput { title: title.to_string(), importance: Some(1), ..Default::default() },
                    "alice",
                )
                .unwrap();
            }
        }
        let mut app = TuiApp::new(&db_path, config, "alice".to_string(), false).unwrap();
        let key = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        key(&mut app, KeyCode::Char('4'));
        key(&mut app, KeyCode::Char('A'));
        assert_eq!(app.selected_ids.len(), 2);
        key(&mut app, KeyCode::Char('b'));
        for ch in "importance=5 +batch".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);

        assert!(app.selected_ids.is_empty(), "status: {}", app.status_message);
        assert!(
            app.all_tasks
                .iter()
                .all(|t| t.importance == 5 && t.tags == vec!["batch".to_string()])
        );
    }

    #[test]
    fn tui_detail_edit_delete_comment_and_switch_user() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_detail.db");
        let config = AppConfig::default();
        let id = {
            let mut db = Database::open(&db_path).unwrap();
            let t = wsid_core::create_task(
                &mut db,
                wsid_core::CreateTaskInput { title: "Original".to_string(), ..Default::default() },
                "alice",
            )
            .unwrap();
            wsid_core::add_task_note(&mut db, &t.id, "alice", "first comment", false).unwrap();
            t.id
        };
        let mut app = TuiApp::new(&db_path, config, "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };

        // Switch active user with `Ctrl+U` (registers it and stays valid-only); `U` filters views.
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_str(&mut app, "bad name");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.user, "alice");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_str(&mut app, "Bob");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.user, "bob");

        // Edit a field by opening the pre-filled add/edit form
        app.open_detail_for(&id);
        press(&mut app, KeyCode::Char('E'));
        assert_eq!(app.active_tab, ActiveTab::DataEntry);
        assert_eq!(app.entry_form.fields[0], "Original");
        app.entry_form.fields[6] = "5".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(app.detail_modal.as_ref().unwrap().task.importance, 5, "status: {}", app.status_message);

        // Past deadlines are rejected via the shared core validation
        press(&mut app, KeyCode::Char('E'));
        app.entry_form.fields[2] = "2000-01-01".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let errs = app.form_errors.clone().expect("past deadline rejected");
        assert!(errs.iter().any(|(_, m)| m.contains("past")), "errs: {errs:?}");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.detail_modal.as_ref().unwrap().task.importance, 5, "status: {}", app.status_message);

        // Delete the selected comment (requires typing `yes`)
        press(&mut app, KeyCode::Char('X'));
        type_str(&mut app, "no");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.detail_modal.as_ref().unwrap().notes.len(), 1);
        press(&mut app, KeyCode::Char('X'));
        type_str(&mut app, "yes");
        press(&mut app, KeyCode::Enter);
        assert!(app.detail_modal.as_ref().unwrap().notes.is_empty());

        // Delete the task
        press(&mut app, KeyCode::Char('D'));
        type_str(&mut app, "yes");
        press(&mut app, KeyCode::Enter);
        assert!(app.detail_modal.is_none());
        assert!(app.all_tasks.is_empty());
    }

    #[test]
    fn tui_add_task_form_cursor_mask_validation_and_error_modal() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_form.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let row_text = |terminal: &Terminal<TestBackend>, y: u16| -> String {
            let buf = terminal.backend().buffer();
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };

        press(&mut app, KeyCode::Char('5'));

        // Title, then a masked Due date typed as bare digits, then SGDB: cursor must sit right after the value.
        type_str(&mut app, "Hello");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_str(&mut app, "20990105");
        assert_eq!(app.entry_form.fields[2], "2099-01-05 ");
        press(&mut app, KeyCode::Tab);
        type_str(&mut app, "20990101");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        let line = row_text(&terminal, cursor.y);
        let value = &app.entry_form.fields[3];
        let end_of_value = line.find(value.as_str()).unwrap() + value.chars().count();
        assert_eq!(cursor.x as usize, end_of_value, "cursor should sit at the end of the SGDB value: {line:?}");

        // Leaving the field validates immediately: SGDB 01-01 is fine, but make it invalid and tab away.
        app.entry_form.fields[3] = "2099-01-09".to_string();
        press(&mut app, KeyCode::Tab);
        assert!(app.entry_form.errors[3].is_some());
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let rendered: String = (0..36)
            .map(|y| row_text(&terminal, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("✗"), "inline error line rendered");
        let buf = terminal.backend().buffer();
        let red_cells = (0..36u16)
            .flat_map(|y| (0..120u16).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].symbol() == "✗" && buf[(x, y)].fg == ratatui::style::Color::Red)
            .count();
        assert!(red_cells > 0, "error marker is red");

        // Submitting an invalid form opens the modal and Enter returns focus to the first invalid field.
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let errs = app.form_errors.clone().expect("modal opened");
        assert!(errs.iter().any(|(i, _)| *i == Some(3)));
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let rendered: String = (0..36)
            .map(|y| row_text(&terminal, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Cannot create task"));
        press(&mut app, KeyCode::Enter);
        assert!(app.form_errors.is_none());
        assert_eq!(app.entry_form.active_index, 3);

        // Fix the SGDB and submit successfully.
        app.entry_form.fields[3] = "2099-01-01".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.form_errors.is_none(), "{:?}", app.form_errors);
        assert_eq!(app.all_tasks.len(), 1);
    }

    #[test]
    fn tui_relation_attachment_and_user_pickers() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_pickers.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let target = wsid_core::create_task(
            &mut Database::open(&db_path).unwrap(),
            wsid_core::CreateTaskInput { title: "Fix login".to_string(), ..Default::default() },
            "alice",
        )
        .unwrap();
        app.refresh_all().unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let screen = |terminal: &Terminal<TestBackend>| -> String {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        press(&mut app, KeyCode::Char('5'));
        type_str(&mut app, "Picker task");
        app.entry_form.set_active_index(&app.db, "alice", 10);

        // Kind prefix -> dropdown of kinds -> Enter fills `blocks:`, then title search offers the task.
        type_str(&mut app, "blo");
        assert!(app.entry_form.dropdown.is_some());
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("blocks"), "kind dropdown drawn");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.entry_form.fields[10], "blocks:");
        type_str(&mut app, "login");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains(&format!("{}  Fix login", target.id)), "target dropdown drawn");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.entry_form.relations.len(), 1);
        assert_eq!(app.entry_form.relations[0].relation, "blocks");
        assert!(app.entry_form.fields[10].is_empty());
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("↳ blocks"), "relation sub-row drawn");

        // Nonexistent task and unknown kind are flagged inline, in red, without leaving the field.
        type_str(&mut app, "blocks:AB1333");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.entry_form.errors[10]
                .as_deref()
                .unwrap()
                .contains("No task AB1333")
        );
        assert_eq!(app.entry_form.relations.len(), 1);
        for _ in 0..13 {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "blockz:AB1333");
        press(&mut app, KeyCode::Enter);
        assert!(
            app.entry_form.errors[10]
                .as_deref()
                .unwrap()
                .contains("Unknown relation")
        );
        for _ in 0..13 {
            press(&mut app, KeyCode::Backspace);
        }

        // Manage list: Enter on empty input opens it, Delete removes the highlighted relation.
        press(&mut app, KeyCode::Enter);
        assert!(app.entry_form.dropdown.as_ref().is_some_and(|d| d.manage));
        press(&mut app, KeyCode::Delete);
        assert!(app.entry_form.relations.is_empty());
        assert_eq!(app.entry_form.active_index, 10);

        // Attachments: a pasted quoted path opens the alt-text step prefilled with the file name.
        let file = dir.path().join("a b.png");
        std::fs::write(&file, b"x").unwrap();
        app.entry_form.set_active_index(&app.db, "alice", 11);
        app.handle_paste(&format!("'{}'", file.display()));
        assert_eq!(app.entry_form.pending_attachment.as_deref(), Some(file.to_str().unwrap()));
        assert_eq!(app.entry_form.fields[11], "a b.png");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.entry_form.attachments.len(), 1);
        assert_eq!(app.entry_form.attachments[0].description.as_deref(), Some("a b.png"));

        // A multi-file paste adds each file with its name as alt text; a missing file is an error.
        let second = dir.path().join("c.txt");
        std::fs::write(&second, b"y").unwrap();
        app.handle_paste(&format!("{} {}", second.display(), second.display()));
        assert_eq!(app.entry_form.attachments.len(), 3);
        app.handle_paste("/definitely/missing.txt");
        assert!(
            app.entry_form.errors[11]
                .as_deref()
                .unwrap()
                .contains("File not found")
        );
        app.entry_form.fields[11].clear();

        // Users: a new name shows an info note; a known user is offered in the dropdown.
        app.entry_form.set_active_index(&app.db, "alice", 13);
        for _ in 0..5 {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "quebert");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("@quebert is new"), "new-user note drawn");

        // Submit creates everything atomically, including the new user.
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.form_errors.is_none(), "{:?}", app.form_errors);
        let created = app
            .all_tasks
            .iter()
            .find(|t| t.title == "Picker task")
            .unwrap();
        assert_eq!(created.assignee, "quebert");
    }

    #[test]
    fn tui_time_estimate_normalization_and_timezone_display() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_tz.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let screen = |terminal: &Terminal<TestBackend>| -> String {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let tz = wsid_core::local_tz_abbr();
        press(&mut app, KeyCode::Char('5'));
        type_str(&mut app, "Timezone task");
        app.entry_form.set_active_index(&app.db, "alice", 2);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let form_screen = screen(&terminal);
        assert!(form_screen.contains(&format!("YYYY-MM-DD HH:MM {tz}")), "label has tz: {form_screen}");
        assert!(form_screen.contains(&format!("Local time zone: {tz}")), "due row hint has tz: {form_screen}");

        type_str(&mut app, "20990222");
        app.entry_form.set_active_index(&app.db, "alice", 4);
        type_str(&mut app, "200m");
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("= 3h 20m"), "live duration preview drawn");

        // Blurring field 4 normalizes `200m` -> `3h 20m`.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.entry_form.fields[4], "3h 20m");

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.form_errors.is_none(), "{:?}", app.form_errors);
        let task_id = app.all_tasks[0].id.clone();
        // Store an explicit RFC3339 deadline (as the GUI does) and verify the detail view renders it without raw offset.
        wsid_core::update_task(
            &mut app.db,
            &task_id,
            wsid_core::UpdateTaskInput { sgdb_date: Some(Some("2099-02-20T20:00:00-05:00".to_string())), ..Default::default() },
        )
        .unwrap();

        app.open_detail_for(&task_id);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let detail_screen = screen(&terminal);
        assert!(detail_screen.contains("Time Estimate: 3h 20m"), "detail uses format_duration_minutes: {detail_screen}");
        assert!(detail_screen.contains("2099-02-22 20:00"), "detail renders human deadline: {detail_screen}");
        assert!(!detail_screen.contains("T20:00:00-05:00"), "detail has no raw RFC3339 time: {detail_screen}");
    }

    #[test]
    fn tui_detail_modal_attachments_cursor_mark_inspect_edit_and_remove() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_atts.db");
        let managed_dir = dir.path().join("managed");
        let linked_file = dir.path().join("spec.txt");
        let missing_file = dir.path().join("gone.txt");
        std::fs::write(&linked_file, b"hello world").unwrap();
        std::fs::write(&missing_file, b"soon gone").unwrap();

        let task_id = {
            let mut db = Database::open(&db_path).unwrap();
            let t = wsid_core::create_task(
                &mut db,
                wsid_core::CreateTaskInput { title: "Attachment task".to_string(), ..Default::default() },
                "alice",
            )
            .unwrap();
            wsid_core::attach_existing_file_with_dir(&mut db, &t.id, None, &linked_file, Some("Initial spec"), Some(&managed_dir)).unwrap();
            wsid_core::save_clipboard_attachment_with_dir(
                &mut db,
                &t.id,
                None,
                b"PNGDATA",
                Some("clip.png"),
                Some("Pasted shot"),
                Some(&managed_dir),
            )
            .unwrap();
            let note = wsid_core::add_task_note(&mut db, &t.id, "alice", "See attached", false)
                .unwrap()
                .note;
            wsid_core::attach_existing_file_with_dir(&mut db, &t.id, Some(note.id), &missing_file, None, Some(&managed_dir)).unwrap();
            t.id
        };
        std::fs::remove_file(&missing_file).unwrap();

        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(140, 42)).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let screen = |terminal: &Terminal<TestBackend>| -> String {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .map(|y| {
                    (0..buf.area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        app.open_detail_for(&task_id);
        assert_eq!(app.detail_modal.as_ref().unwrap().all_attachments().len(), 3);

        // Unfocused list has no cursor and no details block for an "implicitly selected" row.
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let unfocused = screen(&terminal);
        assert!(!unfocused.contains("▶ [ ]"), "no attachment cursor before focus: {unfocused}");
        assert!(!unfocused.contains("Linked file | text/plain"), "no details block before focus: {unfocused}");
        assert!(unfocused.contains("Attachments (3):"), "unfocused header: {unfocused}");
        // The modal owns the keyboard, so the footer must show its keys only.
        assert!(unfocused.contains("[Esc] Close"), "detail footer: {unfocused}");
        assert!(!unfocused.contains("[p] Preset"), "no stale Focus hints under the modal: {unfocused}");

        // Focus attachments list with 'a'
        press(&mut app, KeyCode::Char('a'));
        assert!(app.attachment_focused);
        assert_eq!(app.attachment_cursor, 0);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let s0 = screen(&terminal);
        assert!(s0.contains("Attachments (3) [FOCUSED"), "focused header: {s0}");
        assert!(s0.contains("Linked file | text/plain | 11 B"), "first attachment details block: {s0}");

        // Move down to managed copy, mark it with Space, then edit alt text with 'e'
        press(&mut app, KeyCode::Down);
        assert_eq!(app.attachment_cursor, 1);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.attachment_marked.len(), 1);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("Managed copy | image/png"), "second attachment details block");

        press(&mut app, KeyCode::Char('e'));
        for _ in 0.."Pasted shot".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "Updated shot: v2");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.detail_modal.as_ref().unwrap().all_attachments()[1].description, "Updated shot: v2");

        // Move down to note attachment whose file was deleted -> shows (missing), and remove it with 'x'
        press(&mut app, KeyCode::Down);
        assert_eq!(app.attachment_cursor, 2);
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        assert!(screen(&terminal).contains("(missing)"), "missing indicator shown");

        // Accessible mode renders `> 1. name (linked) /path` lines
        let first_path = app.detail_modal.as_ref().unwrap().all_attachments()[0]
            .file_path
            .clone();
        app.accessible_mode = true;
        app.attachment_cursor = 0;
        terminal
            .draw(|f| crate::tui::ui::render(f, &mut app))
            .unwrap();
        let acc = screen(&terminal);
        assert!(acc.contains(&format!("> 1. spec.txt (linked) {first_path}")), "accessible attachment line: {acc}");
        app.accessible_mode = false;

        // Remove the third attachment with 'x', then Esc leaves the list (detail modal stays open)
        app.attachment_cursor = 2;
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.detail_modal.as_ref().unwrap().all_attachments().len(), 2);
        press(&mut app, KeyCode::Esc);
        assert!(!app.attachment_focused);
        assert!(app.detail_modal.is_some());
    }

    #[test]
    fn tui_focus_context_owns_keys_and_footer_in_priority_order() {
        use crate::tui::app::{DetailPromptKind, FocusContext};

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("tui_focus.db");
        let task_id = {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(&mut db, wsid_core::CreateTaskInput { title: "Focus".to_string(), ..Default::default() }, "alice")
                .unwrap()
                .id
        };
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();

        // Plain list view.
        assert_eq!(app.focus_context(), FocusContext::Tab(crate::tui::app::ActiveTab::Focus));

        // A detail prompt outranks the modal and the tab underneath it.
        app.open_detail_for(&task_id);
        assert_eq!(app.focus_context(), FocusContext::DetailModal);
        app.detail_prompt_kind = DetailPromptKind::JumpToTask;
        assert_eq!(app.focus_context(), FocusContext::DetailPrompt(DetailPromptKind::JumpToTask));
        app.detail_prompt_kind = DetailPromptKind::None;
        app.attachment_focused = true;
        assert_eq!(app.focus_context(), FocusContext::DetailAttachments);
        app.attachment_focused = false;
        assert_eq!(app.focus_context(), FocusContext::DetailModal);

        // Modals above the detail modal.
        app.project_modal_open = true;
        assert_eq!(app.focus_context(), FocusContext::ProjectModal);
        app.about_modal_open = true;
        assert_eq!(app.focus_context(), FocusContext::AboutModal);
        app.form_errors = Some(vec![(None, "boom".to_string())]);
        assert_eq!(app.focus_context(), FocusContext::FormErrorModal);

        // The validation modal swallows keys instead of quitting on 'q'.
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.should_quit);
        assert!(app.form_errors.is_none());
    }

    #[test]
    fn tui_prompts_support_word_delete_undo_and_paste() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("prompts.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode, modifiers: KeyModifiers| app.handle_key(KeyEvent::new(code, modifiers));

        // Project filter: Option+Backspace removes a word, Ctrl+Z brings it back.
        press(&mut app, KeyCode::Char('P'), KeyModifiers::NONE);
        assert!(app.project_modal_open);
        for ch in "web api".chars() {
            press(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert_eq!(app.project_modal_query, "web api");
        press(&mut app, KeyCode::Backspace, KeyModifiers::ALT);
        assert_eq!(app.project_modal_query, "web ");
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.project_modal_query, "web api");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.project_modal_open);

        // Database search: a bracketed paste stays on one line and undoes as one step.
        press(&mut app, KeyCode::Char('4'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(app.search_editing);
        app.handle_paste("line one\nline two");
        assert_eq!(app.search_query, "line one line two");
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.search_query, "");
        press(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(app.search_query, "line one line two");

        // The word-delete chord only fires while a prompt owns the keys.
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.search_editing);
        assert_eq!(app.search_query, "line one line two");
    }

    #[test]
    fn tui_eisenhower_edits_follow_focus_and_apply_to_checked_tasks() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("eisenhower.db");
        let (do_first, delegate) = {
            let mut db = Database::open(&db_path).unwrap();
            let do_first = wsid_core::create_task(
                &mut db,
                wsid_core::CreateTaskInput {
                    title: "Do first".to_string(),
                    importance: Some(3),
                    urgent_override: Some(true),
                    ..Default::default()
                },
                "alice",
            )
            .unwrap()
            .id;
            let delegate = wsid_core::create_task(
                &mut db,
                wsid_core::CreateTaskInput {
                    title: "Delegate".to_string(),
                    importance: Some(1),
                    urgent_override: Some(true),
                    ..Default::default()
                },
                "alice",
            )
            .unwrap()
            .id;
            (do_first, delegate)
        };
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode, modifiers: KeyModifiers| app.handle_key(KeyEvent::new(code, modifiers));
        // Quadrant and row the matrix cursor is on.
        let focused = |app: &TuiApp| {
            app.eisenhower_tasks_in_quadrant(app.eisenhower_quadrant)[app.eisenhower_row]
                .task
                .id
                .clone()
        };
        let importance = |app: &TuiApp, id: &str| {
            app.all_tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .importance
        };

        press(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
        assert_eq!(app.eisenhower_quadrant, 0, "Q1 holds the important-and-urgent task");

        // Check the Q3 task, then walk to Q1 and check the other one too.
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(app.eisenhower_quadrant, 0);
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
        assert_eq!(app.selected_ids.len(), 2);

        // Demoting importance moves the focused task from Q1 to Q3; the cursor goes with it and
        // both check marks survive, even though only one task needed the change.
        press(&mut app, KeyCode::Char('-'), KeyModifiers::NONE);
        assert_eq!(app.eisenhower_quadrant, 2, "the cursor follows the edited task into Q3");
        assert_eq!(focused(&app), do_first);
        assert_eq!(importance(&app, &do_first), 2);
        assert_eq!(importance(&app, &delegate), 1, "1 is already the floor");
        assert_eq!(app.selected_ids.len(), 2, "check marks are never cleared by an edit");

        // Raising both at once: the focused task returns to Q1 and the cursor follows it again.
        press(&mut app, KeyCode::Char('+'), KeyModifiers::NONE);
        assert_eq!(importance(&app, &do_first), 3);
        assert_eq!(importance(&app, &delegate), 2);
        assert_eq!(app.eisenhower_quadrant, 0);
        assert_eq!(focused(&app), do_first);
        assert_eq!(app.selected_ids.len(), 2);

        // Completing every checked task empties the matrix; the cursor does not panic or drift
        // onto a stale row, and the checks stay put.
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(app.eisenhower_items.is_empty());
        assert_eq!(app.eisenhower_row, 0);
        assert_eq!(app.selected_ids.len(), 2);
    }

    #[test]
    fn tui_user_filter_modal_scopes_each_view() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("users.db");
        {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(&mut db, wsid_core::CreateTaskInput { title: "Alice task".to_string(), ..Default::default() }, "alice")
                .unwrap();
            wsid_core::create_task(&mut db, wsid_core::CreateTaskInput { title: "Bob task".to_string(), ..Default::default() }, "bob")
                .unwrap();
        }
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let has_all = |app: &TuiApp| {
            app.user_picker_options(crate::tui::app::PickerMode::Filter)
                .iter()
                .any(|(value, _)| value == wsid_core::ALL_USERS)
        };
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        let matrix_titles = |app: &TuiApp| {
            app.eisenhower_items
                .iter()
                .map(|i| i.task.title.clone())
                .collect::<Vec<_>>()
        };

        // Every list view starts scoped to the active user.
        assert_eq!((app.focus_user.as_str(), app.kanban_user.as_str(), app.eisenhower_user.as_str()), ("alice", "alice", "alice"));
        assert_eq!(matrix_titles(&app), vec!["Alice task".to_string()]);

        // Kanban: `U` offers `all` and applying it widens the board.
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('U'));
        assert!(app.user_modal_open);
        assert!(has_all(&app));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert!(!app.user_modal_open);
        assert_eq!(app.kanban_user, wsid_core::ALL_USERS);
        assert_eq!(app.kanban_tasks_in_col(app.kanban_col).len(), 2);

        // Eisenhower: same `all` option, and it reaches the matrix.
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('U'));
        assert!(has_all(&app));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.eisenhower_user, wsid_core::ALL_USERS);
        assert_eq!(matrix_titles(&app), vec!["Alice task".to_string(), "Bob task".to_string()]);

        // Focus: single-user only, and typing filters the list before Enter applies it.
        press(&mut app, KeyCode::Char('1'));
        press(&mut app, KeyCode::Char('U'));
        assert!(!has_all(&app), "Focus shows one user, so `all` is not offered");
        type_str(&mut app, "bo");
        assert_eq!(app.filtered_user_options().first().map(|(v, _)| v.as_str()), Some("bob"), "search narrows the modal to bob");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus_user, "bob");
        let queued: Vec<String> = app
            .focus_queue
            .actionable_tasks
            .iter()
            .map(|f| f.task.title.clone())
            .collect();
        assert_eq!(queued, vec!["Bob task".to_string()]);

        // Esc leaves the filter untouched.
        press(&mut app, KeyCode::Char('U'));
        press(&mut app, KeyCode::Esc);
        assert!(!app.user_modal_open);
        assert_eq!(app.focus_user, "bob");

        // A user nobody has used yet can still be filtered to; `all` stays reserved.
        press(&mut app, KeyCode::Char('U'));
        type_str(&mut app, "@newbie");
        assert_eq!(app.filtered_user_options(), vec![("newbie".to_string(), "@newbie".to_string())]);
        for _ in 0.."@newbie".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "all");
        assert!(app.filtered_user_options().is_empty());
        press(&mut app, KeyCode::Esc);
    }

    #[test]
    fn tui_switching_views_clears_multi_select_but_same_view_keeps_it() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("selection.db");
        {
            let mut db = Database::open(&db_path).unwrap();
            for title in ["First task", "Second task"] {
                wsid_core::create_task(&mut db, CreateTaskInput { title: title.to_string(), ..Default::default() }, "alice").unwrap();
            }
        }

        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('A'));
        assert_eq!(app.selected_ids.len(), 2);

        press(&mut app, KeyCode::Char('3'));
        assert!(app.selected_ids.is_empty(), "leaving a view clears stale check marks");

        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('A'));
        assert_eq!(app.selected_ids.len(), 2);
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(app.selected_ids.len(), 2, "re-selecting the current view keeps check marks");
    }

    #[test]
    fn tui_settings_persist_focus_preset_and_limit_to_config() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("settings.db");
        Database::open(&db_path).unwrap();
        let config_path = dir.path().join("config.toml");
        let config = AppConfig { primary_user: "alice".to_string(), default_user: "alice".to_string(), ..Default::default() };

        let mut app = TuiApp::new_with_config_path(&db_path, config, "alice".to_string(), false, config_path.clone()).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };

        press(&mut app, KeyCode::Char('6'));
        assert_eq!(app.active_tab, ActiveTab::Settings);

        let preset_row = app
            .settings_rows()
            .iter()
            .position(|row| matches!(row.action, SettingsAction::CyclePreset))
            .unwrap();
        app.settings.selected = preset_row;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.preset, wsid_core::FocusPreset::QuickWins);
        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.focus_preset, wsid_core::FocusPreset::QuickWins);

        let limit_row = app
            .settings_rows()
            .iter()
            .position(|row| matches!(row.action, SettingsAction::EditFocusLimit))
            .unwrap();
        app.settings.selected = limit_row;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.settings.mode, SettingsMode::EditFocusLimit);
        let prefilled = app.settings.focus_limit_input.clone();
        for _ in 0..prefilled.chars().count() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "50");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.settings.mode, SettingsMode::Browse);
        assert_eq!(app.config.focus_limit, 50);
        assert_eq!(AppConfig::load_from_path(&config_path).unwrap().focus_limit, 50);

        // Out-of-range values are rejected without persisting or leaving edit mode.
        app.settings.selected = limit_row;
        press(&mut app, KeyCode::Enter);
        for _ in 0..app.settings.focus_limit_input.chars().count() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "51");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.settings.mode, SettingsMode::EditFocusLimit);
        assert_eq!(app.config.focus_limit, 50);
        assert_eq!(AppConfig::load_from_path(&config_path).unwrap().focus_limit, 50);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.settings.mode, SettingsMode::Browse);
    }

    #[test]
    fn tui_switch_database_keeps_state_on_error_and_resets_on_success() {
        let dir = tempdir().unwrap();
        let first_path = dir.path().join("first.db");
        let second_path = dir.path().join("second.db");
        {
            let mut db = Database::open(&first_path).unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "First task".to_string(), ..Default::default() }, "alice").unwrap();
        }
        {
            let mut db = Database::open(&second_path).unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Second task".to_string(), ..Default::default() }, "alice").unwrap();
        }

        let config_path = dir.path().join("config.toml");
        let app_config = AppConfig::default();
        let mut app = TuiApp::new_with_config_path(&first_path, app_config, "alice".to_string(), false, config_path.clone()).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        let before_path = app.db_path.clone();
        let before_titles: Vec<String> = app
            .all_tasks
            .iter()
            .map(|task| task.title.clone())
            .collect();
        // The parent path is a file, so the database cannot be created there.
        let bad_path = first_path.join("bad.db");
        assert!(app.switch_database(&bad_path).is_err());
        assert_eq!(app.db_path, before_path, "a failed switch leaves the open database alone");
        assert_eq!(
            app.all_tasks
                .iter()
                .map(|task| task.title.clone())
                .collect::<Vec<_>>(),
            before_titles
        );
        assert_eq!(app.config.recent_workspaces.first(), None);

        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('A'));
        app.focus_user = "bob".to_string();
        app.kanban_user = "bob".to_string();
        app.search_query = "first".to_string();
        app.search_assignee_filter = Some("bob".to_string());

        app.switch_database(&second_path).unwrap();
        assert_eq!(app.db_path, second_path);
        assert!(app.all_tasks.iter().any(|task| task.title == "Second task"));
        assert!(app.selected_ids.is_empty(), "switching databases resets multi-select");
        assert_eq!(app.focus_user, "alice");
        assert_eq!(app.kanban_user, "alice");
        assert_eq!(app.search_query, "");
        assert_eq!(app.search_assignee_filter, None);
        assert_eq!(app.config.recent_workspaces.first(), Some(&second_path));
        let saved = AppConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.recent_workspaces.first(), Some(&second_path));
    }

    #[test]
    fn tui_settings_delete_user_reassigns_tasks_and_prunes_filters() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("delete-user.db");
        {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Alice task".to_string(), ..Default::default() }, "alice").unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Bob task".to_string(), ..Default::default() }, "bob").unwrap();
        }

        let config_path = dir.path().join("config.toml");
        let config = AppConfig { primary_user: "alice".to_string(), default_user: "alice".to_string(), ..Default::default() };
        let mut app = TuiApp::new_with_config_path(&db_path, config, "alice".to_string(), false, config_path).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        app.focus_user = "bob".to_string();
        app.kanban_user = "bob".to_string();
        app.search_assignee_filter = Some("bob".to_string());

        press(&mut app, KeyCode::Char('6'));
        let bob_row = app
            .settings_rows()
            .iter()
            .position(|row| matches!(&row.action, SettingsAction::DeleteUser(user) if user == "bob"))
            .expect("primary user can delete other known users");
        app.settings.selected = bob_row;
        press(&mut app, KeyCode::Enter);
        assert!(app.settings.delete_confirm.is_some());
        press(&mut app, KeyCode::Enter);

        assert!(app.settings.delete_confirm.is_none());
        let known = wsid_core::list_known_users(&app.db, &[]).unwrap();
        assert!(!known.iter().any(|user| user == "bob"), "deleted user disappears from known users: {known:?}");
        assert_eq!(app.focus_user, "alice");
        assert_eq!(app.kanban_user, "alice");
        assert_eq!(app.search_assignee_filter, None);
        assert!(app.all_tasks.iter().any(|task| task.title == "Bob task"), "authored tasks are reassigned, not dropped");
        assert!(
            app.all_tasks
                .iter()
                .all(|task| task.assignee != "bob" && task.reporter != "bob")
        );
    }

    #[test]
    fn tui_settings_edit_user_kind_and_bulk_mismatch_notice() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("kinds.db");
        {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Alice task".to_string(), ..Default::default() }, "alice").unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Bob task".to_string(), ..Default::default() }, "bob").unwrap();
        }
        let config = AppConfig { primary_user: "alice".to_string(), default_user: "alice".to_string(), ..Default::default() };
        let mut app = TuiApp::new_with_config_path(&db_path, config, "alice".to_string(), false, dir.path().join("config.toml")).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        press(&mut app, KeyCode::Char('6'));
        app.settings.selected = app
            .settings_rows()
            .iter()
            .position(|row| row.label == "@bob")
            .unwrap();
        press(&mut app, KeyCode::Char('t'));
        assert!(app.settings.kind_modal_open);
        for ch in "jira".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.filtered_kind_options(), vec![("jira".to_string(), "jira · new kind".to_string())]);
        press(&mut app, KeyCode::Enter);
        assert!(!app.settings.kind_modal_open, "{}", app.status_message);
        assert_eq!(
            wsid_core::list_user_kinds(&app.db)
                .unwrap()
                .get("bob")
                .map(String::as_str),
            Some("jira")
        );
        assert!(
            app.settings_rows()
                .iter()
                .any(|row| row.label == "@bob" && row.value.starts_with("jira · "))
        );

        // Typing a conflicting kind in the bulk prompt warns live and reports a notice on apply.
        app.selected_ids = vec![app.all_tasks[0].id.clone()];
        app.apply_bulk_input("a=human:bob");
        assert!(app.status_message.contains("warning: @bob is a jira user"), "{}", app.status_message);
    }

    #[test]
    fn tui_settings_hide_delete_actions_from_non_primary_users() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("non-primary.db");
        {
            let mut db = Database::open(&db_path).unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Alice task".to_string(), ..Default::default() }, "alice").unwrap();
            wsid_core::create_task(&mut db, CreateTaskInput { title: "Bob task".to_string(), ..Default::default() }, "bob").unwrap();
        }

        let config_path = dir.path().join("config.toml");
        let config = AppConfig { primary_user: "alice".to_string(), default_user: "bob".to_string(), ..Default::default() };
        let mut app = TuiApp::new_with_config_path(&db_path, config, "bob".to_string(), false, config_path).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        press(&mut app, KeyCode::Char('6'));
        assert!(!app.is_primary_user());
        assert!(
            !app.settings_rows()
                .iter()
                .any(|row| matches!(row.action, SettingsAction::DeleteUser(_))),
            "non-primary users see an informational row instead of delete actions"
        );
        assert!(
            app.settings_rows()
                .iter()
                .any(|row| row.value.contains("only @alice can manage users"))
        );
    }

    #[test]
    fn tui_ctrl_u_and_settings_open_user_switcher_without_all() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("switcher_user.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode, modifiers: KeyModifiers| app.handle_key(KeyEvent::new(code, modifiers));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };

        // Ctrl+U works from any list view and opens the switcher, not the filter.
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert!(app.user_modal_open);
        assert_eq!(app.user_modal_mode, PickerMode::Switch);
        assert!(
            app.filtered_user_options()
                .iter()
                .all(|(value, _)| value != wsid_core::ALL_USERS),
            "switcher never offers all"
        );

        // A name that is not valid selects nothing; the active user stays put.
        type_str(&mut app, "bad name");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.user, "alice");
        assert!(app.user_modal_open);

        // A new, valid name is registered and becomes the active user.
        press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        type_str(&mut app, "Bob");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!app.user_modal_open);
        assert_eq!(app.user, "bob");

        // Settings → Active user opens the same switcher.
        press(&mut app, KeyCode::Char('6'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.user_modal_open);
        assert_eq!(app.user_modal_mode, PickerMode::Switch);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.user_modal_open);
        assert_eq!(app.user, "bob");

        // `U` is still the per-view filter, which offers all where the view allows it.
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('U'), KeyModifiers::NONE);
        assert_eq!(app.user_modal_mode, PickerMode::Filter);
        assert_eq!(app.filtered_user_options().first().map(|(v, _)| v.as_str()), Some(wsid_core::ALL_USERS));
    }

    #[test]
    fn tui_ctrl_p_switches_active_project_and_p_stays_a_filter() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("switcher_project.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode, modifiers: KeyModifiers| app.handle_key(KeyEvent::new(code, modifiers));
        let type_str = |app: &mut TuiApp, s: &str| {
            for ch in s.chars() {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        assert_eq!(app.active_project, wsid_core::DEFAULT_PROJECT);

        // Filter (`P`) offers all first; the switcher (`Ctrl+P`) does not.
        press(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('P'), KeyModifiers::NONE);
        assert_eq!(
            app.filtered_project_options()
                .first()
                .map(|(v, _)| v.as_str()),
            Some(wsid_core::ALL_PROJECTS)
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

        press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(app.project_modal_open);
        assert_eq!(app.project_modal_mode, PickerMode::Switch);
        assert!(
            app.filtered_project_options()
                .iter()
                .all(|(value, _)| value != wsid_core::ALL_PROJECTS)
        );

        // `all` is reserved: typing it leaves nothing to select.
        type_str(&mut app, "all");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.project_modal_open);
        assert_eq!(app.active_project, wsid_core::DEFAULT_PROJECT);

        // A new project name is offered and becomes active; Kanban and Eisenhower move to it.
        for _ in 0..3 {
            press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        }
        type_str(&mut app, "Backend");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!app.project_modal_open);
        assert_eq!(app.active_project, "backend");
        assert_eq!(app.kanban_project, "backend");
        assert_eq!(app.eisenhower_project, "backend");

        // Settings → Active project opens the switcher too.
        press(&mut app, KeyCode::Char('6'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.project_modal_open);
        assert_eq!(app.project_modal_mode, PickerMode::Switch);
    }

    #[test]
    fn tui_view_entry_resets_scope_but_keeps_database_search_state() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("scope_reset.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode| app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));

        app.active_project = "backend".to_string();
        app.kanban_user = "bob".to_string();
        app.kanban_project = "web".to_string();
        app.focus_project = "web".to_string();
        app.search_query = "invoice".to_string();
        app.search_assignee_filter = Some("bob".to_string());
        app.search_project_filter = Some("web".to_string());
        app.search_only_mine = true;

        // Entering Kanban resets its user and project to the active context.
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.kanban_user, "alice");
        assert_eq!(app.kanban_project, "backend");

        // Entering Focus shows all projects for the active user.
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.focus_user, "alice");
        assert_eq!(app.focus_project, wsid_core::ALL_PROJECTS);

        // Database resets user, project, and `m`, but keeps the search text.
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(app.search_assignee_filter, None);
        assert_eq!(app.search_project_filter, None);
        assert!(!app.search_only_mine);
        assert_eq!(app.search_query, "invoice");

        // Returning to the same view keeps its scope.
        app.kanban_user = "bob".to_string();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.kanban_user, "alice", "entering a view from another view resets it");
        app.kanban_user = "bob".to_string();
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.kanban_user, "bob", "same-tab switches leave scope alone");

        // Settings has no view scope.
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(app.kanban_user, "bob");
    }

    #[test]
    fn tui_switching_active_user_moves_only_views_following_the_old_user() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("switch_follow.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();

        app.kanban_user = "alice".to_string();
        app.eisenhower_user = "bob".to_string();
        app.search_assignee_filter = Some("alice".to_string());
        assert!(app.switch_active_user("carol"));
        assert_eq!(app.user, "carol");
        assert_eq!(app.kanban_user, "carol", "a view following the old user follows the new one");
        assert_eq!(app.eisenhower_user, "bob", "an explicit filter on another user stays");
        assert_eq!(app.search_assignee_filter.as_deref(), Some("carol"));
        assert!(!app.switch_active_user("all"), "all is never an active user");
        assert_eq!(app.user, "carol");
    }

    #[test]
    fn tui_add_task_follows_active_context_but_keeps_drafts() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("entry_context.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), false).unwrap();
        let press = |app: &mut TuiApp, code: KeyCode, modifiers: KeyModifiers| app.handle_key(KeyEvent::new(code, modifiers));

        // A pristine form is seeded with the active user and project.
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(app.entry_form.fields[12], "alice");
        assert_eq!(app.entry_form.fields[15], wsid_core::DEFAULT_PROJECT);

        // Switching the active project from Add Task updates the pristine project field.
        press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        for ch in "backend".chars() {
            press(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.entry_form.fields[15], "backend");

        // A draft keeps what the user typed, and only context fields still matching the old context move.
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        app.entry_form.fields[14] = "carol".to_string();
        press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        for ch in "bob".chars() {
            press(&mut app, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.user, "bob");
        assert_eq!(app.entry_form.fields[0], "x", "the draft title survives");
        assert_eq!(app.entry_form.fields[12], "bob");
        assert_eq!(app.entry_form.fields[14], "carol", "a field the user changed is not overwritten");
    }

    #[test]
    fn tui_active_context_switchers_render_titles_in_accessible_mode() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("switcher_render.db");
        let mut app = TuiApp::new(&db_path, AppConfig::default(), "alice".to_string(), true).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let rendered = |terminal: &mut Terminal<TestBackend>, app: &mut TuiApp| {
            terminal.draw(|f| crate::tui::ui::render(f, app)).unwrap();
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };

        app.open_user_switcher();
        let text = rendered(&mut terminal, &mut app);
        assert!(text.contains("User Switcher"), "user switcher title: {text}");
        assert!(text.contains("Switch"), "switch hint: {text}");
        app.user_modal_open = false;

        app.open_project_switcher();
        let text = rendered(&mut terminal, &mut app);
        assert!(text.contains("Project Switcher"), "project switcher title: {text}");
        app.project_modal_open = false;
        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE));
        let text = rendered(&mut terminal, &mut app);
        assert!(text.contains("Project Filter"), "project filter title: {text}");
    }
}
