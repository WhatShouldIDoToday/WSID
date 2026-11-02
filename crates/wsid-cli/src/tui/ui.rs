use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, HighlightSpacing, List, ListItem, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Tabs, Wrap,
};
use wsid_core::{EisenhowerQuadrant, TaskStatus, default_global_db_path, get_about_metadata};

use crate::tui::app::{ActiveTab, DataEntryForm, DatabaseSortColumn, DetailPromptKind, TuiApp};

const SELECTED_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

fn quadrant_title(q: EisenhowerQuadrant) -> &'static str {
    match q {
        EisenhowerQuadrant::Q1DoFirst => "Q1: Do First (Urgent & Important)",
        EisenhowerQuadrant::Q2Schedule => "Q2: Schedule (Important, Not Urgent)",
        EisenhowerQuadrant::Q3DelegateQuick => "Q3: Delegate / Quick (Urgent, Not Important)",
        EisenhowerQuadrant::Q4Later => "Q4: Later (Neither Urgent Nor Important)",
    }
}

fn format_minutes(mins: i64) -> String {
    if mins <= 0 {
        return "0m".to_string();
    }
    let h = mins / 60;
    let m = mins % 60;
    if h > 0 && m > 0 {
        format!("{h}h{m}m")
    } else if h > 0 {
        format!("{h}h")
    } else {
        format!("{m}m")
    }
}

pub fn render(frame: &mut Frame, app: &mut TuiApp) {
    let [header_area, content_area, footer_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(2)]).areas(frame.area());

    render_header(frame, header_area, app);

    match app.active_tab {
        ActiveTab::Focus => render_focus_view(frame, content_area, app),
        ActiveTab::Kanban => render_kanban_view(frame, content_area, app),
        ActiveTab::Eisenhower => render_eisenhower_view(frame, content_area, app),
        ActiveTab::Database => render_database_view(frame, content_area, app),
        ActiveTab::DataEntry => render_data_entry_view(frame, content_area, app),
    }

    render_footer(frame, footer_area, app);

    if app.detail_modal.is_some() {
        render_detail_modal(frame, frame.area(), app);
    }

    if app.about_modal_open {
        render_about_modal(frame, frame.area(), app);
    }
}

fn panel_borders(borders: Borders, accessible: bool) -> Borders {
    if accessible { Borders::NONE } else { borders }
}

fn panel_block(title: impl Into<Line<'static>>, borders: Borders, accessible: bool) -> Block<'static> {
    let mut block = Block::default()
        .title(title.into())
        .borders(panel_borders(borders, accessible));
    if !accessible {
        block = block.border_type(BorderType::Rounded);
    }
    block
}

fn render_empty_state(frame: &mut Frame, area: Rect, block: Block<'_>, message: &str) {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let [centered_row] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(inner);
    let paragraph = Paragraph::new(Line::from(Span::styled(message.to_string(), Style::default().fg(Color::DarkGray)))).centered();
    frame.render_widget(paragraph, centered_row);
}

fn render_header(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let titles: Vec<Line> = ActiveTab::ALL
        .iter()
        .map(|t| Line::from(t.title()))
        .collect();

    let mut block = Block::default().borders(panel_borders(Borders::BOTTOM, app.accessible_mode));
    if !app.accessible_mode {
        block = block.border_style(Style::default().fg(Color::DarkGray));
    }

    let tabs = Tabs::new(titles)
        .block(block)
        .select(app.active_tab.index())
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(if app.accessible_mode { "  " } else { " │ " });

    frame.render_widget(tabs, area);
}

fn render_focus_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    if app.focus_queue.actionable_tasks.is_empty() {
        app.focus_state.select(None);
    } else {
        let clamped = app
            .focus_selected
            .min(app.focus_queue.actionable_tasks.len() - 1);
        app.focus_selected = clamped;
        app.focus_state.select(Some(clamped));
    }

    let has_blocked_summary = app.show_blocked_banner && !app.focus_queue.blocked_summary.is_empty();
    let show_secondary_pane = !app.accessible_mode && (!app.focus_queue.actionable_tasks.is_empty() || has_blocked_summary);

    let (left_area, right_area) = if show_secondary_pane {
        let [left, right] = Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)]).areas(area);
        (left, Some(right))
    } else {
        (area, None)
    };

    let list_borders = if right_area.is_some() { Borders::RIGHT } else { Borders::NONE };
    let list_block = panel_block(
        Span::styled(
            " Focus Queue ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        list_borders,
        app.accessible_mode,
    );

    if app.focus_queue.actionable_tasks.is_empty() {
        render_empty_state(frame, left_area, list_block, "All caught up. No pending tasks in focus.");
    } else {
        let items: Vec<ListItem> = app
            .focus_queue
            .actionable_tasks
            .iter()
            .enumerate()
            .map(|(idx, scored)| {
                let t = &scored.task;
                let est = t
                    .time_estimate_minutes
                    .map(format_minutes)
                    .unwrap_or_else(|| "-".to_string());
                let due = t.due_date.as_deref().unwrap_or("-");
                let sgdb = t.sgdb_date.as_deref().unwrap_or("-");
                let line1 = format!(
                    "#{rank} [{id}] {title} ({status}) — Score {score:.2} | Q{quadrant}",
                    rank = idx + 1,
                    id = t.id,
                    title = t.title,
                    status = t.status,
                    score = scored.breakdown.total_score,
                    quadrant = scored.quadrant.number(),
                );
                let line2 = format!(
                    "   Imp:{}/5 Diff:{}/5 Prio:{}/5 Est:{} SGDB:{} Due:{} Unblocks:{}",
                    t.importance, t.difficulty, t.manual_priority, est, sgdb, due, scored.breakdown.unblocks_count,
                );
                ListItem::new(vec![Line::from(line1), Line::from(line2)])
            })
            .collect();

        let list = List::new(items)
            .block(list_block)
            .highlight_style(SELECTED_STYLE)
            .highlight_symbol(">> ")
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, left_area, &mut app.focus_state);
    }

    if let Some(right_rect) = right_area {
        let mut right_lines: Vec<Line> = Vec::new();
        if let Some(selected) = app.focus_queue.actionable_tasks.get(app.focus_selected) {
            let b = &selected.breakdown;
            right_lines.push(Line::from(Span::styled(
                format!("Selected: [{}] {}", selected.task.id, selected.task.title),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            right_lines.push(Line::from(format!("Quadrant: {}", quadrant_title(selected.quadrant))));
            right_lines.push(Line::from(format!("Total Score: {:.2}", b.total_score)));
            right_lines.push(Line::from(format!("  Urgency component:    {:+.2}", b.urgency_component)));
            right_lines.push(Line::from(format!("  Importance component: {:+.2}", b.importance_component)));
            right_lines.push(Line::from(format!("  Unblocks component:   {:+.2} ({} tasks)", b.unblocks_component, b.unblocks_count)));
            right_lines.push(Line::from(format!("  Effort component:     {:+.2}", b.effort_component)));
            right_lines.push(Line::from(format!("  Manual component:     {:+.2}", b.manual_priority_component)));
            if let Some(slack) = b.slack_days {
                right_lines.push(Line::from(format!("  Effective slack:      {slack:.1} day(s)")));
            }
            right_lines.push(Line::from(""));
        }

        if has_blocked_summary {
            right_lines.push(Line::from(Span::styled(
                "Blocked / Waiting Summary:",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            for group in &app.focus_queue.blocked_summary {
                right_lines.push(Line::from(format!(
                    "• [{}] {} task(s) waiting on @{} ({}): {}",
                    group.project_or_tag,
                    group.count,
                    group.waiting_on_assignee,
                    group.blocker_kind,
                    group.task_ids.join(", ")
                )));
            }
        }

        let right_panel = Paragraph::new(right_lines)
            .block(panel_block(" Score Breakdown ", Borders::NONE, app.accessible_mode))
            .wrap(Wrap { trim: true });
        frame.render_widget(right_panel, right_rect);
    }
}

fn render_kanban_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    app.clamp_kanban_selection();

    if app.accessible_mode {
        let status = TaskStatus::ALL[app.kanban_col];
        let tasks: Vec<_> = app
            .kanban_tasks_in_col(app.kanban_col)
            .into_iter()
            .cloned()
            .collect();
        let title = format!(" Kanban — Column {}/{}: {} ({}) ", app.kanban_col + 1, TaskStatus::ALL.len(), status, tasks.len());
        let block = panel_block(title, Borders::NONE, true);

        if tasks.is_empty() {
            render_empty_state(frame, area, block, "No tasks in this column.");
        } else {
            let items: Vec<ListItem> = tasks
                .iter()
                .map(|t| {
                    let due = t.due_date.as_deref().unwrap_or("-");
                    ListItem::new(Line::from(format!("[{}] {} (@{}, Imp:{}/5, Due:{})", t.id, t.title, t.assignee, t.importance, due)))
                })
                .collect();

            let list = List::new(items)
                .block(block)
                .highlight_style(SELECTED_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);
            frame.render_stateful_widget(list, area, &mut app.kanban_states[app.kanban_col]);
        }
        return;
    }

    let cols: [Rect; 6] = Layout::horizontal([Constraint::Fill(1); 6]).areas(area);

    for (col_idx, status) in TaskStatus::ALL.iter().enumerate() {
        let is_active_col = col_idx == app.kanban_col;
        let tasks: Vec<_> = app
            .kanban_tasks_in_col(col_idx)
            .into_iter()
            .cloned()
            .collect();

        let title_style = if is_active_col {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let col_borders = if col_idx + 1 < TaskStatus::ALL.len() { Borders::RIGHT } else { Borders::NONE };
        let block = Block::default()
            .title(Span::styled(format!(" {} ({}) ", status, tasks.len()), title_style))
            .borders(col_borders)
            .border_style(Style::default().fg(Color::DarkGray));

        if tasks.is_empty() {
            render_empty_state(frame, cols[col_idx], block, "Empty");
            continue;
        }

        let items: Vec<ListItem> = tasks
            .iter()
            .map(|t| {
                let line1 = format!("[{}] {}", t.id, t.title);
                let line2 = format!("  @{} I:{} D:{}", t.assignee, t.importance, t.difficulty);
                ListItem::new(vec![Line::from(line1), Line::from(line2)])
            })
            .collect();

        let mut list = List::new(items).block(block);
        if is_active_col {
            list = list
                .highlight_style(SELECTED_STYLE)
                .highlight_symbol("> ")
                .highlight_spacing(HighlightSpacing::Always);
        }
        frame.render_stateful_widget(list, cols[col_idx], &mut app.kanban_states[col_idx]);
    }
}

fn render_eisenhower_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    app.clamp_eisenhower_selection();

    let quadrants =
        [EisenhowerQuadrant::Q1DoFirst, EisenhowerQuadrant::Q2Schedule, EisenhowerQuadrant::Q3DelegateQuick, EisenhowerQuadrant::Q4Later];

    if app.accessible_mode {
        let q = quadrants[app.eisenhower_quadrant];
        let items_in_q: Vec<_> = app
            .eisenhower_tasks_in_quadrant(app.eisenhower_quadrant)
            .into_iter()
            .cloned()
            .collect();
        let title = format!(" Eisenhower — {} ({}) ", quadrant_title(q), items_in_q.len());
        let block = panel_block(title, Borders::NONE, true);

        if items_in_q.is_empty() {
            render_empty_state(frame, area, block, "No active tasks in this quadrant.");
        } else {
            let items: Vec<ListItem> = items_in_q
                .iter()
                .map(|c| {
                    let t = &c.task;
                    let slack = c
                        .slack_days
                        .map(|s| format!("{s:.1}d"))
                        .unwrap_or_else(|| "-".to_string());
                    ListItem::new(Line::from(format!(
                        "[{}] {} ({}) — Imp:{}/5, Diff:{}/5, Urgent:{}, Slack:{}",
                        t.id, t.title, t.status, t.importance, t.difficulty, c.is_urgent, slack
                    )))
                })
                .collect();

            let list = List::new(items)
                .block(block)
                .highlight_style(SELECTED_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);
            frame.render_stateful_widget(list, area, &mut app.eisenhower_states[app.eisenhower_quadrant]);
        }
        return;
    }

    // 2x2 Grid layout recipe
    let [top_row, bottom_row] = Layout::vertical([Constraint::Fill(1); 2]).areas(area);
    let [q1_area, q2_area] = Layout::horizontal([Constraint::Fill(1); 2]).areas(top_row);
    let [q3_area, q4_area] = Layout::horizontal([Constraint::Fill(1); 2]).areas(bottom_row);
    let rects = [q1_area, q2_area, q3_area, q4_area];

    let quadrant_borders = [Borders::BOTTOM | Borders::RIGHT, Borders::BOTTOM, Borders::RIGHT, Borders::NONE];

    for (q_idx, q) in quadrants.iter().enumerate() {
        let is_active_q = q_idx == app.eisenhower_quadrant;
        let items_in_q: Vec<_> = app
            .eisenhower_tasks_in_quadrant(q_idx)
            .into_iter()
            .cloned()
            .collect();

        let title_style = if is_active_q {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let block = Block::default()
            .title(Span::styled(format!(" {} ({}) ", quadrant_title(*q), items_in_q.len()), title_style))
            .borders(quadrant_borders[q_idx])
            .border_style(Style::default().fg(Color::DarkGray));

        if items_in_q.is_empty() {
            render_empty_state(frame, rects[q_idx], block, "No active tasks in this quadrant.");
            continue;
        }

        let items: Vec<ListItem> = items_in_q
            .iter()
            .map(|c| {
                let t = &c.task;
                let slack = c
                    .slack_days
                    .map(|s| format!("slack:{s:.1}d"))
                    .unwrap_or_else(|| "no-deadline".to_string());
                ListItem::new(Line::from(format!("[{}] {} (I:{}/5 D:{}/5 {})", t.id, t.title, t.importance, t.difficulty, slack)))
            })
            .collect();

        let mut list = List::new(items).block(block);
        if is_active_q {
            list = list
                .highlight_style(SELECTED_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);
        }
        frame.render_stateful_widget(list, rects[q_idx], &mut app.eisenhower_states[q_idx]);
    }
}

fn render_database_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    if app.search_hits.is_empty() {
        app.search_table_state.select(None);
        app.search_list_state.select(None);
    } else {
        let clamped = app.search_selected.min(app.search_hits.len() - 1);
        app.search_selected = clamped;
        app.search_table_state.select(Some(clamped));
        app.search_list_state.select(Some(clamped));
    }

    let [search_bar_area, results_area] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(area);

    let status_filter_label = app
        .search_status_filter
        .map_or_else(|| "All".to_string(), |s| s.to_string());
    let mine_label = if app.search_only_mine { format!("@{}", app.user) } else { "All".to_string() };
    let sort_dir = if app.search_sort_asc { "ASC" } else { "DESC" };

    let query_prefix = " Search: ";
    let query_display = if app.search_editing {
        format!("{}{} ", query_prefix, app.search_query)
    } else if app.search_query.is_empty() {
        format!(
            " Search: <All tasks> | Status: {} | Assignee: {} | Sort: {} ({})",
            status_filter_label,
            mine_label,
            app.search_sort_col.label(),
            sort_dir
        )
    } else {
        format!(
            " Search: '{}' | Status: {} | Assignee: {} | Sort: {} ({})",
            app.search_query,
            status_filter_label,
            mine_label,
            app.search_sort_col.label(),
            sort_dir
        )
    };

    let search_bar = Paragraph::new(query_display).block(
        Block::default()
            .borders(panel_borders(Borders::BOTTOM, app.accessible_mode))
            .border_style(if app.search_editing { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) }),
    );
    frame.render_widget(search_bar, search_bar_area);

    if app.search_editing {
        let cursor_x = search_bar_area
            .x
            .saturating_add((query_prefix.len() + app.search_query.chars().count()) as u16)
            .min(search_bar_area.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, search_bar_area.y));
    }

    let results_title = format!(" Results ({}) ", app.search_hits.len());
    let results_block = panel_block(results_title, Borders::NONE, app.accessible_mode);

    if app.search_hits.is_empty() {
        render_empty_state(frame, results_area, results_block, "No tasks match the current search and filters.");
        return;
    }

    if app.accessible_mode {
        let items: Vec<ListItem> = app
            .search_hits
            .iter()
            .map(|hit| {
                let t = &hit.task;
                let due = t.due_date.as_deref().unwrap_or("-");
                let sgdb = t.sgdb_date.as_deref().unwrap_or("-");
                let tags = if t.tags.is_empty() { "-".to_string() } else { t.tags.join(",") };
                ListItem::new(Line::from(format!(
                    "[{}] {} ({}) Assignee:@{} Imp:{} Diff:{} Prio:{} SGDB:{} Due:{} Tags:{}",
                    t.id, t.title, t.status, t.assignee, t.importance, t.difficulty, t.manual_priority, sgdb, due, tags
                )))
            })
            .collect();

        let list = List::new(items)
            .block(results_block)
            .highlight_style(SELECTED_STYLE)
            .highlight_symbol(">> ")
            .highlight_spacing(HighlightSpacing::Always);
        frame.render_stateful_widget(list, results_area, &mut app.search_list_state);
        return;
    }

    let arrow = if app.search_sort_asc { "▲" } else { "▼" };
    let col_header = |col: DatabaseSortColumn, label: &str| -> Cell<'static> {
        if app.search_sort_col == col {
            Cell::from(format!("{label} {arrow}")).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Cell::from(label.to_string()).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        }
    };

    let header = Row::new(vec![
        col_header(DatabaseSortColumn::Id, "ID"),
        col_header(DatabaseSortColumn::Title, "Title"),
        col_header(DatabaseSortColumn::Status, "Status"),
        col_header(DatabaseSortColumn::Assignee, "Assignee"),
        col_header(DatabaseSortColumn::Importance, "Imp"),
        col_header(DatabaseSortColumn::Difficulty, "Diff"),
        col_header(DatabaseSortColumn::SgdbDate, "SGDB"),
        col_header(DatabaseSortColumn::DueDate, "Due"),
        Cell::from("Tags").style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .bottom_margin(0);

    let rows: Vec<Row> = app
        .search_hits
        .iter()
        .map(|hit| {
            let t = &hit.task;
            let sgdb = t
                .sgdb_date
                .as_deref()
                .and_then(|s| s.get(..10))
                .unwrap_or("-");
            let due = t
                .due_date
                .as_deref()
                .and_then(|s| s.get(..10))
                .unwrap_or("-");
            let tags = if t.tags.is_empty() { "-".to_string() } else { t.tags.join(",") };
            Row::new(vec![
                Cell::from(t.id.clone()),
                Cell::from(t.title.clone()),
                Cell::from(t.status.to_string()),
                Cell::from(format!("@{}", t.assignee)),
                Cell::from(format!("{}/5", t.importance)),
                Cell::from(format!("{}/5", t.difficulty)),
                Cell::from(sgdb.to_string()),
                Cell::from(due.to_string()),
                Cell::from(tags),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Fill(1),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Percentage(15),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(results_block)
        .row_highlight_style(SELECTED_STYLE)
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, results_area, &mut app.search_table_state);
}

fn render_data_entry_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    app.entry_form
        .state
        .select(Some(app.entry_form.active_index));

    let mut items: Vec<ListItem> = Vec::new();
    for (idx, label) in DataEntryForm::LABELS.iter().enumerate() {
        let is_active = app.entry_form.active_index == idx;
        let val = &app.entry_form.fields[idx];
        let text = format!("{label:<38}: {val}");
        let style = if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        items.push(ListItem::new(Line::from(Span::styled(text, style))));
    }

    let is_submit = app.entry_form.active_index == DataEntryForm::SUBMIT_INDEX;
    let submit_style = if is_submit {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    };
    items.push(ListItem::new(Line::from(Span::styled("[ Submit New Task ]", submit_style))));

    let form_block = panel_block(" Add New Task ", Borders::NONE, app.accessible_mode);
    let inner_area = form_block.inner(area);

    let form = List::new(items)
        .block(form_block)
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(form, area, &mut app.entry_form.state);

    if app.entry_form.active_index < DataEntryForm::SUBMIT_INDEX && inner_area.height > 0 {
        let offset = app.entry_form.state.offset();
        if app.entry_form.active_index >= offset {
            let visible_row = (app.entry_form.active_index - offset) as u16;
            if visible_row < inner_area.height {
                let val_len = app.entry_form.fields[app.entry_form.active_index]
                    .chars()
                    .count() as u16;
                // ">> " (3) + "{label:<38}: " (40) = 43 chars
                let cursor_x = inner_area
                    .x
                    .saturating_add(43 + val_len)
                    .min(inner_area.right().saturating_sub(1));
                let cursor_y = inner_area.y.saturating_add(visible_row);
                frame.set_cursor_position(Position::new(cursor_x, cursor_y));
            }
        }
    }
}

fn render_footer(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let hints = match app.active_tab {
        ActiveTab::Focus => {
            "[1-5] Views | [j/k] Select | [Enter] Inspect | [p] Preset | [s] Start | [d] Done | [x] Stuck | [b] Blocked | [?] Help & Status | [q] Quit"
        }
        ActiveTab::Kanban => "[1-5] Views | [h/l] Column | [j/k] Task | [</>] Move Status | [Enter] Inspect | [?] Help & Status | [q] Quit",
        ActiveTab::Eisenhower => {
            "[1-5] Views | [h/l/Tab] Quadrant | [j/k] Task | [+/-] Imp | [u] Urgent | [s] Start | [d] Done | [Enter] Inspect | [?] Help"
        }
        ActiveTab::Database => {
            "[1-5] Views | [/] Search | [f] Status | [m] Mine | [o/O] Sort | [j/k] Select | [Enter] Inspect | [?] Help & Status | [q] Quit"
        }
        ActiveTab::DataEntry => "[Tab/Up/Down] Field | [Ctrl+S] Submit | [Esc] Cancel | [?] Help & Status",
    };

    let text = if app.status_message.is_empty() { format!(" {hints}") } else { format!(" {}  │  {hints}", app.status_message) };

    let footer = Paragraph::new(text)
        .style(Style::default().fg(Color::Cyan))
        .block(
            Block::default()
                .borders(panel_borders(Borders::TOP, app.accessible_mode))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(footer, area);
}

fn render_detail_modal(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let popup_area = centered_rect(82, 84, area);
    frame.render_widget(Clear, popup_area);

    let Some(detail) = app.detail_modal.as_mut() else {
        return;
    };

    let t = &detail.task;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            format!("[{}] {} ", t.id, t.title),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("({})", t.status), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(format!("Reporter: @{} | Assignee: @{} | Evaluator: @{}", t.reporter, t.assignee, t.evaluator)));

    let est = t
        .time_estimate_minutes
        .map(format_minutes)
        .unwrap_or_else(|| "None".to_string());
    let sgdb = t.sgdb_date.as_deref().unwrap_or("None");
    let due = t.due_date.as_deref().unwrap_or("None");
    lines.push(Line::from(format!(
        "Importance: {}/5 | Difficulty: {}/5 | Priority: {}/5 | Urgent Override: {}",
        t.importance, t.difficulty, t.manual_priority, t.urgent_override
    )));
    lines.push(Line::from(format!("Time Estimate: {est} | SGDB Date: {sgdb} | Hard Due Date: {due}")));
    lines.push(Line::from(format!("Tags: {}", if t.tags.is_empty() { "None".to_string() } else { t.tags.join(", ") })));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Description:", Style::default().add_modifier(Modifier::BOLD))));
    if t.description.is_empty() {
        lines.push(Line::from("  (No description)"));
    } else {
        for l in t.description.lines() {
            lines.push(Line::from(format!("  {l}")));
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        format!("Attachments ({}):", t.attachments.len()),
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    )));
    if t.attachments.is_empty() {
        lines.push(Line::from("  (None — press [f] to attach a file or [v] to paste from clipboard)"));
    } else {
        for att in &t.attachments {
            let kind_badge = if att.is_image() { "IMG" } else { "FILE" };
            let note_tag = att
                .note_id
                .map(|nid| format!(" (Comment #{nid})"))
                .unwrap_or_default();
            let alt = if att.description.is_empty() { "(no alt text)" } else { att.description.as_str() };
            lines.push(Line::from(format!(
                "  • #{id} [{kind_badge}] {name} ({mime}){note_tag} — Alt: {alt}",
                id = att.id,
                name = att.file_name,
                mime = att.media_type,
            )));
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Relationships & Dependencies (press [g] to jump by ID):",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!("  Blocked By (must finish first): {}", format_task_brief_list(&detail.blockers))));
    lines.push(Line::from(format!("  Blocks (downstream waiting):    {}", format_task_brief_list(&detail.blocking))));
    lines.push(Line::from(format!("  Parent Task(s):                 {}", format_task_brief_list(&detail.parents))));
    lines.push(Line::from(format!("  Sub-tasks:                      {}", format_task_brief_list(&detail.subtasks))));
    lines.push(Line::from(format!("  Parallel Peers:                 {}", format_task_brief_list(&detail.parallel))));
    lines.push(Line::from(format!("  Related Tasks:                  {}", format_task_brief_list(&detail.related))));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        format!("Comments & Notes ({}):", detail.notes.len()),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    if detail.notes.is_empty() {
        lines.push(Line::from("  (No comments yet — press 'c' to add one)"));
    } else {
        for n in &detail.notes {
            lines.push(Line::from(format!("  • [{}] @{}: {}", n.created_at.format("%Y-%m-%d %H:%M"), n.author, n.body)));
            for att in &n.attachments {
                let alt = if att.description.is_empty() { "(no alt text)" } else { att.description.as_str() };
                lines.push(Line::from(format!("      ↳ Attachment #{}: {} ({}) — Alt: {}", att.id, att.file_name, att.media_type, alt)));
            }
        }
    }

    let modal_block = panel_block(format!(" Task Detail — [{}] ", t.id), Borders::ALL, app.accessible_mode)
        .border_style(Style::default().fg(Color::Yellow));
    let inner_area = modal_block.inner(popup_area);
    frame.render_widget(modal_block, popup_area);

    let [body_area, prompt_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]).areas(inner_area);

    let max_scroll = (lines.len() as u16).saturating_sub(body_area.height);
    detail.scroll = detail.scroll.min(max_scroll);

    let body = Paragraph::new(lines.clone())
        .scroll((detail.scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, body_area);

    if !app.accessible_mode && lines.len() as u16 > body_area.height {
        let mut scrollbar_state = ScrollbarState::new(lines.len()).position(detail.scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, body_area, &mut scrollbar_state);
    }

    let (prompt_line, prompt_prefix_len) = match app.detail_prompt_kind {
        DetailPromptKind::None => (
            Line::from(Span::styled(
                "[c] Comment | [f] Attach File | [v] Paste Clipboard | [e] Edit Alt | [o] Open Att | [g] Jump ID | [u] Unblock Subtask | [s/d/x] Status | [Esc] Close",
                Style::default().fg(Color::Cyan),
            )),
            None,
        ),
        DetailPromptKind::AddComment => {
            let prefix = "New Comment (@user, @takenote:user, #ID): ";
            (
                Line::from(Span::styled(
                    format!("{prefix}{}", app.detail_prompt_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Some(prefix.len()),
            )
        }
        DetailPromptKind::AttachFile => {
            let prefix = "Attach File (path | optional alt description): ";
            (
                Line::from(Span::styled(
                    format!("{prefix}{}", app.detail_prompt_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Some(prefix.len()),
            )
        }
        DetailPromptKind::AttachClipboard => {
            let prefix = "Clipboard Alt Description (optional): ";
            (
                Line::from(Span::styled(
                    format!("{prefix}{}", app.detail_prompt_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Some(prefix.len()),
            )
        }
        DetailPromptKind::EditAttachmentDesc => {
            let prefix = "Edit Attachment Alt (attachment_id | new alt description): ";
            (
                Line::from(Span::styled(
                    format!("{prefix}{}", app.detail_prompt_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Some(prefix.len()),
            )
        }
        DetailPromptKind::JumpToTask => {
            let prefix = "Jump to Task ID (e.g. AB1234): ";
            (
                Line::from(Span::styled(
                    format!("{prefix}{}", app.detail_prompt_input),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Some(prefix.len()),
            )
        }
    };

    let prompt_block = Block::default()
        .borders(panel_borders(Borders::TOP, app.accessible_mode))
        .border_style(Style::default().fg(Color::DarkGray));
    let prompt_inner = prompt_block.inner(prompt_area);
    let prompt_widget = Paragraph::new(prompt_line).block(prompt_block);
    frame.render_widget(prompt_widget, prompt_area);

    if let Some(prefix_len) = prompt_prefix_len
        && prompt_inner.height > 0
    {
        let input_len = app.detail_prompt_input.chars().count();
        let cursor_x = prompt_inner
            .x
            .saturating_add((prefix_len + input_len) as u16)
            .min(prompt_inner.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(cursor_x, prompt_inner.y));
    }
}

fn render_about_modal(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let popup_area = centered_rect(84, 86, area);
    frame.render_widget(Clear, popup_area);

    let about = get_about_metadata();
    let mut lines: Vec<Line> = Vec::new();

    let mode_label = if app.accessible_mode { "ACCESSIBLE" } else { "STANDARD" };
    let db_moniker = if app.db_path == default_global_db_path() {
        "[global]".to_string()
    } else {
        let name = app
            .db_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace.db");
        format!("[workspace: {name}]")
    };

    lines.push(Line::from(Span::styled(
        "Session Status",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!("  Active User: @{}  |  Focus Preset: {}  |  Mode: {}", app.user, app.preset, mode_label)));
    lines.push(Line::from(format!("  Database:    {} ({})", db_moniker, app.db_path.display())));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled(
            format!("{} v{} ", about.app_name, about.version),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("— {}", about.app_full_name), Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(format!("Repository: {}", about.repository_url)));
    lines.push(Line::from(format!("License:    {}", about.license_spdx)));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        format!("Third-Party Rust Dependencies ({} unique licenses):", about.rust_licenses.licenses.len()),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(
        "Use [j/k] or [Up/Down] to scroll line-by-line, [PgUp/PgDn] for pages, [g/G] for top/bottom, [Esc/?/q] to close.",
    ));
    lines.push(Line::from(""));

    for group in &about.rust_licenses.licenses {
        let pkgs: Vec<String> = group
            .used_by
            .iter()
            .map(|u| format!("{} {}", u.name, u.version))
            .collect();
        lines.push(Line::from(Span::styled(
            format!("=== {} ({}) — {} crate(s) ===", group.name, group.id, pkgs.len()),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!("Crates: {}", pkgs.join(", "))));
        lines.push(Line::from(""));
        for text_line in group.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        format!("=== WSID Project License ({}) ===", about.license_spdx),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    for l in about.license_text.lines() {
        lines.push(Line::from(format!("  {l}")));
    }

    let modal_block = panel_block(" Help, Status & Licenses — [Esc/?/q] Close ", Borders::ALL, app.accessible_mode)
        .border_style(Style::default().fg(Color::Cyan));
    let inner_area = modal_block.inner(popup_area);
    frame.render_widget(modal_block, popup_area);

    let max_scroll = (lines.len() as u16).saturating_sub(inner_area.height);
    app.about_scroll = app.about_scroll.min(max_scroll);

    let modal = Paragraph::new(lines.clone())
        .scroll((app.about_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, inner_area);

    if !app.accessible_mode && lines.len() as u16 > inner_area.height {
        let mut scrollbar_state = ScrollbarState::new(lines.len()).position(app.about_scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, inner_area, &mut scrollbar_state);
    }
}

fn format_task_brief_list(tasks: &[wsid_core::Task]) -> String {
    if tasks.is_empty() {
        return "None".to_string();
    }
    tasks
        .iter()
        .map(|t| format!("[{}] {} ({})", t.id, t.title, t.status))
        .collect::<Vec<_>>()
        .join("; ")
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [area] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    area
}
