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

        // Toggle accessible mode ('a' back in Focus view)
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
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

        // Switch active user (registers it and stays valid-only)
        press(&mut app, KeyCode::Char('U'));
        type_str(&mut app, "bad name");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.user, "alice");
        press(&mut app, KeyCode::Char('U'));
        type_str(&mut app, "Bob");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.user, "bob");

        // Edit a field from the detail view
        app.open_detail_for(&id);
        press(&mut app, KeyCode::Char('E'));
        for _ in 0.."title: Original".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "importance: 5");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.detail_modal.as_ref().unwrap().task.importance, 5, "status: {}", app.status_message);

        // Past deadlines are rejected via the shared core validation
        press(&mut app, KeyCode::Char('E'));
        for _ in 0.."title: Original".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "due: 2000-01-01");
        press(&mut app, KeyCode::Enter);
        assert!(app.status_message.contains("past"), "status: {}", app.status_message);

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
}
