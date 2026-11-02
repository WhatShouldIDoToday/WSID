use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap};

use crate::args::format_duration_estimate;
use crate::tui::app::{ActiveTab, DataEntryForm, DetailPromptKind, TaskDetailSnapshot, TuiApp};
use wsid_core::TaskStatus;

pub fn render(frame: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(10), Constraint::Length(2)])
        .split(frame.area());

    render_header(frame, app, chunks[0]);

    match app.active_tab {
        ActiveTab::Focus => render_focus_view(frame, app, chunks[1]),
        ActiveTab::Kanban => render_kanban_view(frame, app, chunks[1]),
        ActiveTab::Eisenhower => render_eisenhower_view(frame, app, chunks[1]),
        ActiveTab::Database => render_database_view(frame, app, chunks[1]),
        ActiveTab::DataEntry => render_data_entry_view(frame, app, chunks[1]),
    }

    render_footer(frame, app, chunks[2]);

    if let Some(ref detail) = app.detail_modal {
        render_detail_modal(frame, app, detail, frame.area());
    }
}

fn border_type(accessible: bool) -> BorderType {
    if accessible { BorderType::Plain } else { BorderType::Rounded }
}

fn render_header(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let titles: Vec<Line> = ActiveTab::ALL
        .iter()
        .map(|tab| {
            let style = if *tab == app.active_tab {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(tab.title(), style))
        })
        .collect();

    let title_bar = format!(
        " WSID | Persona: @{} | Preset: {} | Mode: {} | DB: {} ",
        app.persona,
        app.preset.as_str(),
        if app.accessible_mode { "Accessible" } else { "Standard" },
        app.db_path.display()
    );

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(app.accessible_mode))
                .title(title_bar),
        )
        .select(app.active_tab.index())
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

fn render_focus_view(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if app.accessible_mode {
            [Constraint::Percentage(100), Constraint::Percentage(0)]
        } else {
            [Constraint::Percentage(65), Constraint::Percentage(35)]
        })
        .split(area);

    let items: Vec<ListItem> = if app.focus_queue.actionable_tasks.is_empty() {
        vec![ListItem::new(Line::from(
            "No actionable tasks in your Focus queue right now. Press [5] to create a task or check Blocked summary.",
        ))]
    } else {
        app.focus_queue
            .actionable_tasks
            .iter()
            .enumerate()
            .map(|(idx, st)| {
                let is_sel = idx == app.focus_selected;
                let prefix = if is_sel { ">> " } else { "   " };
                let est = format_duration_estimate(st.task.time_estimate_minutes);
                let due = st
                    .breakdown
                    .effective_deadline
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "none".to_string());
                let tags = if st.task.tags.is_empty() { String::new() } else { format!(" [{}]", st.task.tags.join(", ")) };
                let attach_badge =
                    if st.task.attachments.is_empty() { String::new() } else { format!(" [{} file(s)]", st.task.attachments.len()) };

                let line1 = format!(
                    "{prefix}{rank}. [{id}] {title} ({status}, Q{q}, score {score:.1}){tags}{attach_badge}",
                    rank = idx + 1,
                    id = st.task.id,
                    title = st.task.title,
                    status = st.task.status.as_str(),
                    q = st.quadrant.number(),
                    score = st.breakdown.total_score,
                );
                let line2 = format!(
                    "      Imp: {}/5 | Diff: {}/5 | Est: {} | Deadline: {} | Unblocks: {}",
                    st.task.importance, st.task.difficulty, est, due, st.breakdown.unblocks_count
                );

                let style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(vec![Line::from(line1), Line::from(line2)]).style(style)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .title(format!(
                " Focus Queue (Top {}, [p] cycle preset: {}, [space] start/done, [Enter] details) ",
                app.focus_queue.actionable_tasks.len(),
                app.preset.as_str()
            )),
    );
    frame.render_widget(list, chunks[0]);

    if !app.accessible_mode {
        let mut right_lines = Vec::new();
        if let Some(st) = app.focus_queue.actionable_tasks.get(app.focus_selected) {
            right_lines.push(Line::from(Span::styled(
                format!("Why [{}] is ranked #{}:", st.task.id, app.focus_selected + 1),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            right_lines.push(Line::from(format!("  Total Score:     {:.2}", st.breakdown.total_score)));
            right_lines.push(Line::from(format!("  Urgency:        +{:.2}", st.breakdown.urgency_component)));
            right_lines.push(Line::from(format!("  Importance:     +{:.2}", st.breakdown.importance_component)));
            right_lines
                .push(Line::from(format!("  Unblocks ({}):   +{:.2}", st.breakdown.unblocks_count, st.breakdown.unblocks_component)));
            right_lines.push(Line::from(format!("  Effort fit:     +{:.2}", st.breakdown.effort_component)));
            right_lines.push(Line::from(format!("  Manual Boost:   {:+.2}", st.breakdown.manual_priority_component)));
            right_lines.push(Line::from(format!("  Parallel Boost: +{:.2}", st.breakdown.parallel_component)));
            right_lines.push(Line::from(format!("  In-Progress:    +{:.2}", st.breakdown.in_progress_component)));
            right_lines.push(Line::from(""));
        }

        right_lines.push(Line::from(Span::styled(
            "Blocked / Waiting Summary:",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )));
        if app.focus_queue.blocked_summary.is_empty() {
            right_lines.push(Line::from("  None! No blocked tasks."));
        } else {
            for grp in &app.focus_queue.blocked_summary {
                right_lines.push(Line::from(format!(
                    "  • [{}] {} task(s) waiting on @{} ({})",
                    grp.project_or_tag,
                    grp.count,
                    grp.waiting_on_assignee,
                    grp.blocker_kind.as_str()
                )));
            }
        }

        let breakdown_panel = Paragraph::new(right_lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(border_type(app.accessible_mode))
                    .title(" Score Breakdown & Situational Awareness "),
            );
        frame.render_widget(breakdown_panel, chunks[1]);
    }
}

fn render_kanban_view(frame: &mut Frame, app: &TuiApp, area: Rect) {
    if app.accessible_mode {
        // Accessible linear single-column view of the active Kanban column
        let status = TaskStatus::ALL[app.kanban_col];
        let col_tasks = app.kanban_tasks_in_col(app.kanban_col);
        let items: Vec<ListItem> = if col_tasks.is_empty() {
            vec![ListItem::new(Line::from("  (Empty column)"))]
        } else {
            col_tasks
                .iter()
                .enumerate()
                .map(|(idx, t)| {
                    let is_sel = idx == app.kanban_row;
                    let prefix = if is_sel { ">> " } else { "   " };
                    let style = if is_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(format!("{prefix}[{}] {} (@{}, imp {}/5)", t.id, t.title, t.assignee, t.importance)))
                        .style(style)
                })
                .collect()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(format!(
                    " Kanban Column {}/{}: {} ({} tasks) — [h/l] switch column, [H/L] move card ",
                    app.kanban_col + 1,
                    TaskStatus::ALL.len(),
                    status.as_str(),
                    col_tasks.len()
                )),
        );
        frame.render_widget(list, area);
        return;
    }

    let constraints = [Constraint::Ratio(1, 6); 6];
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (col_idx, status) in TaskStatus::ALL.iter().enumerate() {
        let is_active_col = col_idx == app.kanban_col;
        let col_tasks = app.kanban_tasks_in_col(col_idx);

        let items: Vec<ListItem> = col_tasks
            .iter()
            .enumerate()
            .map(|(row_idx, t)| {
                let is_sel = is_active_col && row_idx == app.kanban_row;
                let prefix = if is_sel { ">" } else { " " };
                let style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(vec![
                    Line::from(format!("{prefix}[{}] {}", t.id, t.title)),
                    Line::from(format!("  @{} (I:{})", t.assignee, t.importance)),
                ])
                .style(style)
            })
            .collect();

        let title_style = if is_active_col {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .border_style(title_style)
            .title(format!(" {} ({}) ", status.as_str(), col_tasks.len()));

        frame.render_widget(List::new(items).block(block), cols[col_idx]);
    }
}

fn render_eisenhower_view(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let quadrant_titles = [
        "Q1: Urgent & Important (Do First)",
        "Q2: Not Urgent & Important (Schedule)",
        "Q3: Urgent & Not Important (Delegate/Quick)",
        "Q4: Not Urgent & Not Important (Later)",
    ];

    if app.accessible_mode {
        let q_tasks = app.eisenhower_tasks_in_quadrant(app.eisenhower_quadrant);
        let items: Vec<ListItem> = if q_tasks.is_empty() {
            vec![ListItem::new(Line::from("  (No tasks in this quadrant)"))]
        } else {
            q_tasks
                .iter()
                .enumerate()
                .map(|(idx, c)| {
                    let is_sel = idx == app.eisenhower_row;
                    let prefix = if is_sel { ">> " } else { "   " };
                    let slack = c
                        .slack_days
                        .map(|s| format!("{s:.1}d slack"))
                        .unwrap_or_else(|| "no deadline".to_string());
                    let style = if is_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(format!("{prefix}[{}] {} (imp {}/5, {})", c.task.id, c.task.title, c.task.importance, slack)))
                        .style(style)
                })
                .collect()
        };

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(format!(
                    " {} ({} tasks) — [h/l] switch quadrant, [+/-] importance, [u] urgent ",
                    quadrant_titles[app.eisenhower_quadrant],
                    q_tasks.len()
                )),
        );
        frame.render_widget(list, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    let rects = [top_cols[0], top_cols[1], bot_cols[0], bot_cols[1]];

    for q_idx in 0..4 {
        let is_active = q_idx == app.eisenhower_quadrant;
        let q_tasks = app.eisenhower_tasks_in_quadrant(q_idx);
        let items: Vec<ListItem> = q_tasks
            .iter()
            .enumerate()
            .map(|(row_idx, c)| {
                let is_sel = is_active && row_idx == app.eisenhower_row;
                let prefix = if is_sel { ">> " } else { "   " };
                let slack = c
                    .slack_days
                    .map(|s| format!("{s:.1}d slack"))
                    .unwrap_or_else(|| "undated".to_string());
                let style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(format!("{prefix}[{}] {} (Imp:{}/5, {})", c.task.id, c.task.title, c.task.importance, slack)))
                    .style(style)
            })
            .collect();

        let border_style = if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .border_style(border_style)
            .title(format!(" {} ({}) ", quadrant_titles[q_idx], q_tasks.len()));

        frame.render_widget(List::new(items).block(block), rects[q_idx]);
    }
}

fn render_database_view(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6)])
        .split(area);

    let search_title = if app.search_editing {
        " Fuzzy Search Query (TYPING — press [Enter] or [Esc] to navigate results) "
    } else {
        " Fuzzy Search Query (press [/] to type query, [s] sort column, [S] reverse order) "
    };

    let query_box = Paragraph::new(format!("Query: {}", app.search_query)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .border_style(if app.search_editing {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            })
            .title(search_title),
    );
    frame.render_widget(query_box, chunks[0]);

    let items: Vec<ListItem> = if app.search_hits.is_empty() {
        vec![ListItem::new(Line::from("  No matching tasks found."))]
    } else {
        app.search_hits
            .iter()
            .enumerate()
            .map(|(idx, hit)| {
                let t = &hit.task;
                let is_sel = idx == app.search_selected;
                let prefix = if is_sel { ">> " } else { "   " };
                let due = t
                    .due_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let sgdb = t
                    .sgdb_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let tags = if t.tags.is_empty() { "-".to_string() } else { t.tags.join(",") };
                let style = if is_sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(format!(
                    "{prefix}[{}] {:<28} | {:<11} | @{} | Imp:{}/5 | Diff:{}/5 | SGDB:{} | Due:{} | Tags:{}",
                    t.id,
                    truncate_str(&t.title, 28),
                    t.status.as_str(),
                    t.assignee,
                    t.importance,
                    t.difficulty,
                    sgdb,
                    due,
                    tags
                )))
                .style(style)
            })
            .collect()
    };

    let sort_label = app.search_sort_col.label();

    let results_list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .title(format!(
                " Database Results ({} tasks, sorted by {} {}) ",
                app.search_hits.len(),
                sort_label,
                if app.search_sort_asc { "ASC" } else { "DESC" }
            )),
    );
    frame.render_widget(results_list, chunks[1]);
}

fn render_data_entry_view(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let mut items = Vec::new();
    for idx in 0..=DataEntryForm::SUBMIT_INDEX {
        let is_sel = idx == app.entry_form.active_index;
        let prefix = if is_sel { ">> " } else { "   " };
        let style = if is_sel {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let line = if idx == DataEntryForm::SUBMIT_INDEX {
            format!("{prefix}[ SUBMIT NEW TASK (Press Enter or Ctrl+S) ]")
        } else {
            format!("{prefix}{:<34}: {}", DataEntryForm::LABELS[idx], app.entry_form.fields[idx])
        };
        items.push(ListItem::new(Line::from(line)).style(style));
    }

    let form = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(border_type(app.accessible_mode))
            .title(" Create New Task — [Tab/Down] next field, [Shift+Tab/Up] prev, [Ctrl+S] submit "),
    );
    frame.render_widget(form, area);
}

fn render_footer(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let status = if app.status_message.trim().is_empty() {
        "Keys: [1-5] Views | [j/k] Navigate | [Enter] Task Details & Notes | [a] Toggle Accessible Mode | [q] Quit"
    } else {
        app.status_message.as_str()
    };
    let footer = Paragraph::new(Line::from(Span::styled(format!(" {status}"), Style::default().fg(Color::Cyan))));
    frame.render_widget(footer, area);
}

fn render_detail_modal(frame: &mut Frame, app: &TuiApp, detail: &TaskDetailSnapshot, area: Rect) {
    let popup_area = centered_rect(85, 85, area);
    frame.render_widget(Clear, popup_area);

    let t = &detail.task;
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("[{}] {} ({})", t.id, t.title, t.status.as_str()),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!(
        "Origin: {} | Reporter: @{} | Assignee: @{} | Evaluator: @{}",
        t.origin_kind.as_str(),
        t.reporter,
        t.assignee,
        t.evaluator
    )));
    lines.push(Line::from(format!(
        "Importance: {}/5 | Difficulty: {}/5 | Manual Priority: {:+} | Urgent Override: {}",
        t.importance, t.difficulty, t.manual_priority, t.urgent_override
    )));
    lines.push(Line::from(format!(
        "SGDB: {} | Due: {} | Estimate: {} | Tags: {}",
        t.sgdb_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "none".to_string()),
        t.due_date
            .map(|d| d.to_string())
            .unwrap_or_else(|| "none".to_string()),
        format_duration_estimate(t.time_estimate_minutes),
        if t.tags.is_empty() { "none".to_string() } else { t.tags.join(", ") }
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Description:", Style::default().add_modifier(Modifier::BOLD))));
    if t.description.trim().is_empty() {
        lines.push(Line::from("  (no description)"));
    } else {
        for l in t.description.lines() {
            lines.push(Line::from(format!("  {l}")));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("Attachments ({}):", t.attachments.len()), Style::default().add_modifier(Modifier::BOLD))));
    if t.attachments.is_empty() {
        lines.push(Line::from("  (none — press [f] to attach a file or [v] to paste clipboard)"));
    } else {
        for att in &t.attachments {
            lines.push(Line::from(format!(
                "  • [Attachment #{}] {} ({}) — alt=\"{}\"",
                att.id,
                att.file_name,
                att.media_type,
                att.alt_text()
            )));
            lines.push(Line::from(format!(
                "    Disk link: {} ({})",
                att.file_path,
                if att.is_managed { "app-data managed" } else { "external file" }
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Relations:", Style::default().add_modifier(Modifier::BOLD))));
    let relation_groups = [
        ("Blocked By", &detail.blockers),
        ("Blocks", &detail.blocking),
        ("Parents", &detail.parents),
        ("Subtasks", &detail.subtasks),
        ("Parallel", &detail.parallel),
        ("Related", &detail.related),
    ];
    let mut any_relations = false;
    for (label, list) in relation_groups {
        if !list.is_empty() {
            any_relations = true;
            let formatted: Vec<String> = list
                .iter()
                .map(|peer| format!("{} ({}, {})", peer.id, peer.status.as_str(), peer.title))
                .collect();
            lines.push(Line::from(format!("  • {label}: {}", formatted.join(", "))));
        }
    }
    if !any_relations {
        lines.push(Line::from("  (none)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Notes / Comments ({}):", detail.notes.len()),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for note in &detail.notes {
        lines.push(Line::from(format!("  • [#{} @{} {}]: {}", note.id, note.author, note.created_at.format("%Y-%m-%d %H:%M"), note.body)));
        for att in &note.attachments {
            lines.push(Line::from(format!(
                "      [Note Attachment #{}] {} ({}) — alt=\"{}\" -> {}",
                att.id,
                att.file_name,
                att.media_type,
                att.alt_text(),
                att.file_path
            )));
        }
    }

    lines.push(Line::from(""));
    match app.detail_prompt_kind {
        DetailPromptKind::None => {
            lines.push(Line::from(Span::styled(
                "Actions: [c] Comment (@persona / @takenote:p / #ID) | [f] Attach File (path|alt) | [v] Paste Clipboard | [e] Edit Alt (id|alt) | [o] Open Attachment | [g] Jump to Task | [u] Unblock/Dismiss | [s] Start/Done | [Esc] Close",
                Style::default().fg(Color::Green),
            )));
        }
        DetailPromptKind::AddComment => {
            lines.push(Line::from(Span::styled(
                format!("New Comment (Enter to submit, Esc to cancel): {}", app.detail_prompt_input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        DetailPromptKind::AttachFile => {
            lines.push(Line::from(Span::styled(
                format!("Attach File `path | optional alt text` (Enter to attach, Esc to cancel): {}", app.detail_prompt_input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        DetailPromptKind::AttachClipboard => {
            lines.push(Line::from(Span::styled(
                format!("Attach Clipboard `optional alt text` (Enter to paste, Esc to cancel): {}", app.detail_prompt_input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        DetailPromptKind::EditAttachmentDesc => {
            lines.push(Line::from(Span::styled(
                format!("Edit Attachment Alt `attachment_id | alt text` (Enter to save, Esc to cancel): {}", app.detail_prompt_input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        DetailPromptKind::JumpToTask => {
            lines.push(Line::from(Span::styled(
                format!("Jump to Task ID (Enter to open, Esc to cancel): {}", app.detail_prompt_input),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    let modal = Paragraph::new(lines)
        .scroll((detail.scroll, 0))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(border_type(app.accessible_mode))
                .border_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .title(" Task Detail & Collaboration Drawer "),
        );
    frame.render_widget(modal, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
