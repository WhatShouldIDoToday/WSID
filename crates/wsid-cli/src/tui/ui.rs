use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Tabs, Wrap,
};
use wsid_core::{
    EisenhowerQuadrant, TaskStatus, default_global_db_path, format_deadline_date, format_deadline_display, format_duration_minutes,
    get_about_metadata,
};

use crate::tui::app::{ActiveTab, DataEntryForm, DatabaseSortColumn, DetailPromptKind, FocusContext, PickerMode, SettingsMode, TuiApp};
use crate::tui::form;

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

pub fn render(frame: &mut Frame, app: &mut TuiApp) {
    let footer_h = footer_height(app, frame.area().width);
    let [header_area, content_area, footer_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(footer_h)]).areas(frame.area());

    render_header(frame, header_area, app);

    match app.active_tab {
        ActiveTab::Focus => render_focus_view(frame, content_area, app),
        ActiveTab::Kanban => render_kanban_view(frame, content_area, app),
        ActiveTab::Eisenhower => render_eisenhower_view(frame, content_area, app),
        ActiveTab::Database => render_database_view(frame, content_area, app),
        ActiveTab::DataEntry => render_data_entry_view(frame, content_area, app),
        ActiveTab::Settings => render_settings_view(frame, content_area, app),
    }

    render_footer(frame, footer_area, app);

    if app.detail_modal.is_some() {
        render_detail_modal(frame, frame.area(), app);
    }

    if app.settings.delete_confirm.is_some() {
        render_user_delete_modal(frame, frame.area(), app);
    }

    if app.about_modal_open {
        render_about_modal(frame, frame.area(), app);
    }

    if app.project_modal_open {
        render_project_modal(frame, frame.area(), app);
    }

    if app.user_modal_open {
        render_user_modal(frame, frame.area(), app);
    }

    if app.form_errors.is_some() {
        render_form_error_modal(frame, frame.area(), app);
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

    if area.width < 70 || app.active_tab == ActiveTab::Settings {
        frame.render_widget(tabs, area);
        return;
    }

    // Keep the active user and project visible on every list view, not just after pressing U/P.
    let [tabs_area, scope_area] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(46)]).areas(area);
    frame.render_widget(tabs, tabs_area);

    let user = app.current_user_filter();
    let project = app.current_project_filter();
    let scope = Paragraph::new(Line::from(vec![
        Span::styled("User: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            user,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Project: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            project,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]))
    .alignment(Alignment::Right)
    .block(
        Block::default()
            .borders(panel_borders(Borders::BOTTOM, app.accessible_mode))
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(scope, scope_area);
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
                    .map(|m| format_duration_minutes(m.max(0) as u32))
                    .unwrap_or_else(|| "-".to_string());
                let due = t
                    .due_date
                    .as_deref()
                    .map(format_deadline_display)
                    .unwrap_or_else(|| "-".to_string());
                let sgdb = t
                    .sgdb_date
                    .as_deref()
                    .map(format_deadline_display)
                    .unwrap_or_else(|| "-".to_string());
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
        let title = format!(
            " Kanban — Column {}/{}: {} ({}) · project: {} ",
            app.kanban_col + 1,
            TaskStatus::ALL.len(),
            status,
            tasks.len(),
            app.kanban_project
        );
        let block = panel_block(title, Borders::NONE, true);

        if tasks.is_empty() {
            render_empty_state(frame, area, block, "No tasks in this column.");
        } else {
            let items: Vec<ListItem> = tasks
                .iter()
                .map(|t| {
                    let due = t
                        .due_date
                        .as_deref()
                        .map(format_deadline_date)
                        .unwrap_or_else(|| "-".to_string());
                    ListItem::new(Line::from(format!(
                        "{}[{}] {} (@{}, Imp:{}/5, Due:{}, Proj:{})",
                        app.mark(&t.id),
                        t.id,
                        t.title,
                        t.assignee,
                        t.importance,
                        due,
                        t.project
                    )))
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
                let line1 = format!("{}[{}] {}", app.mark(&t.id), t.id, t.title);
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
        let title = format!(" Eisenhower — {} ({}) · project: {} ", quadrant_title(q), items_in_q.len(), app.eisenhower_project);
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
                        "{}[{}] {} ({}) — Imp:{}/5, Diff:{}/5, Urgent:{}, Slack:{}, Proj:{}",
                        app.mark(&t.id),
                        t.id,
                        t.title,
                        t.status,
                        t.importance,
                        t.difficulty,
                        c.is_urgent,
                        slack,
                        t.project
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
                ListItem::new(Line::from(format!(
                    "{}[{}] {} (I:{}/5 D:{}/5 {})",
                    app.mark(&t.id),
                    t.id,
                    t.title,
                    t.importance,
                    t.difficulty,
                    slack
                )))
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
    let mine_label = if app.search_only_mine {
        format!("@{}", app.user)
    } else {
        app.search_assignee_filter
            .as_ref()
            .map_or_else(|| "All".to_string(), |u| format!("@{u}"))
    };
    let tag_label = app
        .search_tag_filter
        .as_ref()
        .map_or_else(String::new, |t| format!(" | Tag: #{t}"));
    let project_label = app
        .search_project_filter
        .as_ref()
        .map_or_else(String::new, |p| format!(" | Project: {p}"));
    let sort_dir = if app.search_sort_asc { "ASC" } else { "DESC" };

    let query_prefix = " Search: ";
    let query_display = if app.search_editing {
        format!("{}{} ", query_prefix, app.search_query)
    } else if app.search_query.is_empty() {
        format!(
            " Search: <All tasks> | Status: {} | Assignee: {}{}{} | Column: {} ({})",
            status_filter_label,
            mine_label,
            tag_label,
            project_label,
            app.search_sort_col.label(),
            sort_dir
        )
    } else {
        format!(
            " Search: '{}' | Status: {} | Assignee: {}{}{} | Column: {} ({})",
            app.search_query,
            status_filter_label,
            mine_label,
            tag_label,
            project_label,
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
                let due = t
                    .due_date
                    .as_deref()
                    .map(format_deadline_date)
                    .unwrap_or_else(|| "-".to_string());
                let sgdb = t
                    .sgdb_date
                    .as_deref()
                    .map(format_deadline_date)
                    .unwrap_or_else(|| "-".to_string());
                let tags = if t.tags.is_empty() { "-".to_string() } else { t.tags.join(",") };
                ListItem::new(Line::from(format!(
                    "{}[{}] {} ({}) Assignee:@{} Imp:{} Diff:{} Prio:{} SGDB:{} Due:{} Proj:{} Tags:{}",
                    app.mark(&t.id),
                    t.id,
                    t.title,
                    t.status,
                    t.assignee,
                    t.importance,
                    t.difficulty,
                    t.manual_priority,
                    sgdb,
                    due,
                    t.project,
                    tags
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
        col_header(DatabaseSortColumn::Project, "Project"),
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
                .map(format_deadline_date)
                .unwrap_or_else(|| "-".to_string());
            let due = t
                .due_date
                .as_deref()
                .map(format_deadline_date)
                .unwrap_or_else(|| "-".to_string());
            let tags = if t.tags.is_empty() { "-".to_string() } else { t.tags.join(",") };
            Row::new(vec![
                Cell::from(format!("{}{}", app.mark(&t.id), t.id)),
                Cell::from(t.title.clone()),
                Cell::from(t.status.to_string()),
                Cell::from(t.project.clone()),
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
        Constraint::Fill(2),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Fill(1),
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

    let label_width = form::label_width();
    let pad = " ".repeat(label_width + 2);
    let error_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let note_style = Style::default().fg(Color::Yellow);
    let sub_style = Style::default().fg(Color::Cyan);
    let mut items: Vec<ListItem> = Vec::new();
    // Rows each item occupies (label row, list sub-rows, error and note lines), for cursor and dropdown placement.
    let mut heights: Vec<u16> = Vec::new();
    for (idx, _spec) in form::FIELDS.iter().enumerate() {
        let form = &app.entry_form;
        let is_active = form.active_index == idx;
        let val = &form.fields[idx];
        let error = form.errors[idx].as_deref();
        let style = if error.is_some() {
            error_style
        } else if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let label = match (idx, &form.pending_attachment) {
            (11, Some(path)) => {
                let name = Path::new(path)
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                format!("Alt text for {name}")
                    .chars()
                    .take(label_width)
                    .collect()
            }
            _ => form::field_label(idx),
        };
        let mut lines = vec![Line::from(Span::styled(format!("{label:<label_width$}: {val}"), style))];
        match idx {
            10 => {
                for (r, title) in form.relations.iter().zip(&form.relation_titles) {
                    lines.push(Line::from(Span::styled(format!("{pad}↳ {}  {}  {title}", r.relation, r.target), sub_style)));
                }
            }
            11 => {
                for a in &form.attachments {
                    let name = Path::new(&a.path)
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    lines.push(Line::from(Span::styled(
                        format!("{pad}↳ {name} - \"{}\"", a.description.as_deref().unwrap_or("")),
                        sub_style,
                    )));
                }
            }
            _ => {}
        }
        if let Some(msg) = error {
            lines.push(Line::from(Span::styled(format!("{pad}✗ {msg}"), error_style)));
        }
        if let Some(note) = form.notes[idx].as_deref() {
            lines.push(Line::from(Span::styled(format!("{pad}+ {note}"), note_style)));
        }
        heights.push(lines.len() as u16);
        items.push(ListItem::new(lines));
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
    let submit_label = if app.editing_task_id.is_some() { "[ Save Changes ]" } else { "[ Submit New Task ]" };
    items.push(ListItem::new(Line::from(Span::styled(submit_label, submit_style))));

    let form_title = match app.editing_task_id.as_deref() {
        Some(id) => format!(" Edit Task [{id}] "),
        None => " Add New Task ".to_string(),
    };
    let form_block = panel_block(form_title, Borders::NONE, app.accessible_mode);
    let inner_area = form_block.inner(area);

    // Background-only highlight so red error text stays red on the selected row.
    let form_widget = List::new(items)
        .block(form_block)
        .highlight_style(Style::new().bg(Color::DarkGray))
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(form_widget, area, &mut app.entry_form.state);

    let active = app.entry_form.active_index;
    if active < DataEntryForm::SUBMIT_INDEX && inner_area.height > 0 {
        let offset = app.entry_form.state.offset();
        if active >= offset {
            let visible_row: u16 = heights[offset..active].iter().sum();
            if visible_row < inner_area.height {
                let val_len = app.entry_form.fields[active].chars().count() as u16;
                let cursor_x = inner_area
                    .x
                    .saturating_add(form::value_column() as u16 + val_len)
                    .min(inner_area.right().saturating_sub(1));
                let cursor_y = inner_area.y.saturating_add(visible_row);
                frame.set_cursor_position(Position::new(cursor_x, cursor_y));
                render_dropdown(frame, inner_area, visible_row, heights[active], app);
            }
        }
    }
}

/// Settings rows are grouped by section; only real rows are navigable, while
/// section headers and the database summary are plain text in the same list.
fn render_settings_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let rows = app.settings_rows();
    app.clamp_settings_selection(rows.len());

    let list_area = if app.settings.mode == SettingsMode::Browse {
        area
    } else {
        let [prompt_area, list_area] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(area);
        render_settings_prompt(frame, prompt_area, app);
        list_area
    };

    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Line::from(Span::styled(
        format!(" Database: {}  ({})", app.db_path.display(), if app.is_global_database() { "global" } else { "workspace" }),
        Style::default().fg(Color::DarkGray),
    ))));
    items.push(ListItem::new(Line::from("")));

    let mut last_section = "";
    let mut selected_display = 0;
    for (index, row) in rows.iter().enumerate() {
        if row.section != last_section {
            if !last_section.is_empty() {
                items.push(ListItem::new(Line::from("")));
            }
            last_section = row.section;
            items.push(ListItem::new(Line::from(Span::styled(
                format!(" {}", row.section),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))));
        }
        if index == app.settings.selected {
            selected_display = items.len();
        }
        let text = if row.value.is_empty() { row.label.clone() } else { format!("{}  {}", row.label, row.value) };
        items.push(ListItem::new(Line::from(text)));
    }

    let mut state = ListState::default();
    if !rows.is_empty() {
        state.select(Some(selected_display));
    }
    let list = List::new(items)
        .block(panel_block(" Settings ", Borders::NONE, app.accessible_mode))
        .highlight_style(SELECTED_STYLE)
        .highlight_symbol(if app.accessible_mode { "> " } else { ">> " })
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn render_settings_prompt(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let (label, value) = match app.settings.mode {
        SettingsMode::EditFocusLimit => ("Focus queue size (1-50): ", app.settings.focus_limit_input.as_str()),
        SettingsMode::EditDatabasePath => ("Database path or folder: ", app.settings.database_path_input.as_str()),
        SettingsMode::Browse => return,
    };
    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(value.to_string()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    let cursor_x = area
        .x
        .saturating_add(label.chars().count() as u16)
        .saturating_add(value.chars().count() as u16)
        .min(area.right().saturating_sub(1));
    if area.height > 0 {
        frame.set_cursor_position(Position::new(cursor_x, area.y));
    }
}

/// Suggestion / manage list under the active row (above it when there is no room below).
fn render_dropdown(frame: &mut Frame, inner: Rect, row: u16, row_height: u16, app: &TuiApp) {
    let Some(dd) = app.entry_form.dropdown.as_ref() else {
        return;
    };
    let x = inner
        .x
        .saturating_add(form::value_column() as u16)
        .min(inner.right().saturating_sub(1));
    let width = inner.right().saturating_sub(x).min(70);
    let rows = dd.labels.len().min(8) as u16;
    let height = rows + if app.accessible_mode { 0 } else { 2 };
    let below = inner.y.saturating_add(row).saturating_add(row_height);
    let y = if below + height <= inner.bottom() || inner.y + row < height { below } else { inner.y + row - height };
    let popup = Rect { x, y, width, height: height.min(inner.bottom().saturating_sub(y)) };
    if popup.width == 0 || popup.height == 0 {
        return;
    }

    let items: Vec<ListItem> = dd
        .labels
        .iter()
        .map(|l| ListItem::new(Line::from(l.as_str())))
        .collect();
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(dd.selected));
    let list = List::new(items)
        .block(panel_block(if dd.manage { " Remove with Delete " } else { "" }, Borders::ALL, app.accessible_mode))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
        .highlight_symbol("> ");
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut state);
}

/// Label without its leading number and trailing hint, e.g. `3. Due Date (YYYY-MM-DD HH:MM)` -> `Due Date`.
fn short_label(label: &str) -> &str {
    let no_num = label.split_once(". ").map_or(label, |(_, rest)| rest);
    no_num
        .split(" (")
        .next()
        .unwrap_or(no_num)
        .trim_end_matches(" *")
}

fn render_form_error_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let Some(errors) = app.form_errors.as_ref() else {
        return;
    };
    let red = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("Fix {} problem(s) before creating the task:", errors.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    for (idx, msg) in errors {
        let name = idx.map_or("Task", |i| short_label(form::FIELDS[i].label));
        lines.push(Line::from(vec![Span::styled("✗ ", red), Span::styled(format!("{name}: "), red), Span::raw(msg.clone())]));
    }

    let height = (lines.len() as u16 + 2).min(area.height);
    let width = (area.width * 7 / 10).max(40).min(area.width);
    let popup =
        Rect { x: area.x + (area.width.saturating_sub(width)) / 2, y: area.y + (area.height.saturating_sub(height)) / 2, width, height };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(Span::styled(" Cannot create task ", red))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Red));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_user_delete_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let Some(confirm) = app.settings.delete_confirm.as_ref() else {
        return;
    };
    let preview = &confirm.preview;
    let red = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Deleting ", Style::default()),
            Span::styled(
                format!("@{}", preview.username),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" reassigns their tasks to @{}.", preview.fallback_user)),
        ]),
        Line::from(""),
        Line::from(format!(
            "{} affected task(s) · authored {} · assigned {} · validator {}",
            preview.affected_tasks.len(),
            preview.authored_task_count,
            preview.assigned_task_count,
            preview.evaluator_task_count
        )),
    ];

    if !preview.affected_tasks.is_empty() {
        lines.push(Line::from(""));
        for task in preview.affected_tasks.iter().take(8) {
            let mut roles = Vec::new();
            if task.is_reporter {
                roles.push("author");
            }
            if task.is_assignee {
                roles.push("assignee");
            }
            if task.is_evaluator {
                roles.push("validator");
            }
            let roles = if roles.is_empty() { "linked".to_string() } else { roles.join(", ") };
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", task.id), Style::default().fg(Color::Cyan)),
                Span::raw(task.title.clone()),
                Span::styled(format!(" ({roles})"), Style::default().fg(Color::DarkGray)),
            ]));
        }
        if preview.affected_tasks.len() > 8 {
            lines.push(Line::from(Span::styled(
                format!("… and {} more", preview.affected_tasks.len() - 8),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(if confirm.delete_authored { "[x] " } else { "[ ] " }, Style::default().fg(Color::Yellow)),
        Span::raw(format!("Also permanently delete {} authored task(s)", preview.authored_task_count)),
    ]));

    let height = (lines.len() as u16 + 2).min(area.height);
    let width = (area.width * 8 / 10).max(50).min(area.width);
    let popup =
        Rect { x: area.x + (area.width.saturating_sub(width)) / 2, y: area.y + (area.height.saturating_sub(height)) / 2, width, height };
    frame.render_widget(Clear, popup);
    let block = panel_block(Span::styled(format!(" Delete User @{} ", preview.username), red), Borders::ALL, app.accessible_mode)
        .border_style(Style::default().fg(Color::Red));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn looks_like_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    ["error", "failed", "cannot", "invalid", "blocked", "unknown", "expected"]
        .iter()
        .any(|w| lower.contains(w))
}

/// Hints for whatever currently owns the keyboard (`TuiApp::focus_context`).
///
/// This is the only place keyboard shortcuts are advertised: modals no longer
/// repeat them in their titles, bodies, or status messages.
fn focus_hints(app: &TuiApp) -> String {
    match app.focus_context() {
        FocusContext::FormErrorModal => "[Enter/Esc/q] Back to the form".to_string(),
        FocusContext::AboutModal => "[j/k] Scroll | [PgUp/PgDn] Page | [g/G] Top/Bottom | [Esc/?/q] Close".to_string(),
        FocusContext::ProjectModal => match app.project_modal_mode {
            PickerMode::Filter => "[type] Filter | [Up/Down] Select | [Enter] Apply | [Ctrl+P] Switch project | [Esc] Cancel".to_string(),
            PickerMode::Switch => "[type] Search or new name | [Up/Down] Select | [Enter] Switch | [Esc] Cancel".to_string(),
        },
        FocusContext::UserModal => match app.user_modal_mode {
            PickerMode::Filter => "[type] Filter | [Up/Down] Select | [Enter] Apply | [Ctrl+U] Switch user | [Esc] Cancel".to_string(),
            PickerMode::Switch => "[type] Search or new name | [Up/Down] Select | [Enter] Switch | [Esc] Cancel".to_string(),
        },
        FocusContext::DetailPrompt(DetailPromptKind::AddComment) => {
            "[Tab] Complete | [Up/Down] Suggestion | [Enter] Post | [Esc] Cancel".to_string()
        }
        FocusContext::DetailPrompt(DetailPromptKind::ConfirmDeleteTask | DetailPromptKind::ConfirmDeleteNote) => {
            "[Enter] Delete | [Esc] Cancel".to_string()
        }
        FocusContext::DetailPrompt(_) => "[Enter] Submit | [Esc] Cancel".to_string(),
        FocusContext::DetailAttachments => {
            "[Up/Down] Move | [Space] Mark | [o] Open | [O] All | [e] Alt | [x] Remove | [f] Attach | [v] Clipboard | [Esc] Leave"
                .to_string()
        }
        FocusContext::DetailModal => {
            "[j/k] Scroll | [a] Attachments | [c] Comment | [E] Edit | [D] Delete | [N/X] Note | [f/v] Attach/Clip | [o/O] Open/All | [g] Jump | [u] Unblock | [s/r/d/x] Status | [Esc] Close"
                .to_string()
        }
        FocusContext::SearchInput => "[Enter/Esc] Finish | [Alt+Bksp] Del word | [Ctrl+Z] Undo".to_string(),
        FocusContext::BulkEditPrompt => "[Enter] Apply | [Esc] Cancel".to_string(),
        FocusContext::UserDeleteModal => "[a] Also delete authored | [Enter] Delete | [Esc] Cancel".to_string(),
        FocusContext::Tab(ActiveTab::Focus) => {
            "[1-6] Views | [P] Project filter | [Ctrl+P] Switch project | [j/k] Select | [Enter] Inspect | [p] Preset | [s] Start | [d] Done | [x] Stuck | [b] Blocked | [U] User filter | [Ctrl+U] Switch user | [?] Help | [q] Quit"
                .to_string()
        }
        FocusContext::Tab(ActiveTab::Kanban) => {
            "[1-6] Views | [P] Project filter | [Ctrl+P] Switch project | [h/l] Column | [j/k] Task | [</>] Move Status | [Space] Select | [b] Bulk edit | [U] User filter | [Ctrl+U] Switch user | [Enter] Inspect | [?] Help | [q] Quit"
                .to_string()
        }
        FocusContext::Tab(ActiveTab::Eisenhower) => {
            "[1-6] Views | [P] Project filter | [Ctrl+P] Switch project | [U] User filter | [Ctrl+U] Switch user | [h/l/Tab/S-Tab] Quadrant | [j/k] Task | [+/-] Imp | [</>] Urgency | [u] Urgent | [Space] Select | [b] Bulk edit | [Enter] Inspect | [?] Help"
                .to_string()
        }
        FocusContext::Tab(ActiveTab::Database) => {
            "[1-6] Views | [P] Project filter | [Ctrl+P] Switch project | [U] User filter | [Ctrl+U] Switch user | [/] Search/filter column | [f] Status | [t] Tag | [u] Cycle user | [m] Mine | [o] Column | [O] Asc/Desc | [Space] Select | [A] All | [b] Bulk edit | [Enter] Inspect | [?] Help"
                .to_string()
        }
        FocusContext::Tab(ActiveTab::DataEntry) => app.entry_form.hint().to_string(),
        FocusContext::Tab(ActiveTab::Settings) => match app.settings.mode {
            SettingsMode::Browse => {
                "[1-6] Views | [j/k] Move | [Enter] Change | [a] Accessible | [?] Help | [q] Quit".to_string()
            }
            SettingsMode::EditFocusLimit => "[0-9] Size | [Enter] Save | [Esc] Cancel".to_string(),
            SettingsMode::EditDatabasePath => "[type] Path | [Enter] Open | [Esc] Cancel".to_string(),
        },
    }
}

fn render_footer(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let footer = Paragraph::new(footer_line(app))
        .style(Style::default().fg(Color::Cyan))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(panel_borders(Borders::TOP, app.accessible_mode))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(footer, area);
}

/// `status │ hints`, or just the hints when there is no status to show.
fn footer_line(app: &TuiApp) -> Line<'static> {
    let hints = focus_hints(app);
    let mut spans: Vec<Span<'static>> = Vec::new();
    if app.status_message.is_empty() {
        spans.push(Span::raw(format!(" {hints}")));
    } else {
        let style = if looks_like_error(&app.status_message) {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(format!(" {}", app.status_message), style));
        spans.push(Span::raw(format!("  │  {hints}")));
    }
    Line::from(spans)
}

/// Rows the footer needs at `width` (including its top border), so long modal
/// hints wrap instead of silently losing their tail.
fn footer_height(app: &TuiApp, width: u16) -> u16 {
    let usable = width.saturating_sub(2).max(1);
    let rows = Paragraph::new(footer_line(app))
        .wrap(Wrap { trim: false })
        .line_count(usable);
    rows.clamp(1, 4) as u16 + 1
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
    lines.push(Line::from(format!("Project: {}", t.project)));

    let est = t
        .time_estimate_minutes
        .map(|m| format_duration_minutes(m.max(0) as u32))
        .unwrap_or_else(|| "None".to_string());
    let sgdb = t
        .sgdb_date
        .as_deref()
        .map(format_deadline_display)
        .unwrap_or_else(|| "None".to_string());
    let due = t
        .due_date
        .as_deref()
        .map(format_deadline_display)
        .unwrap_or_else(|| "None".to_string());
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

    let all_atts = detail.all_attachments();
    let att_cursor = app.attachment_cursor.min(all_atts.len().saturating_sub(1));
    let att_header = if app.attachment_focused {
        format!("Attachments ({}) [FOCUSED]:", all_atts.len())
    } else {
        format!("Attachments ({}):", all_atts.len())
    };
    lines.push(Line::from(Span::styled(
        att_header,
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    )));
    if all_atts.is_empty() {
        lines.push(Line::from("  (None)"));
    } else {
        for (idx, att) in all_atts.iter().enumerate() {
            let selected = app.attachment_focused && idx == att_cursor;
            let marked = app.attachment_marked.contains(&att.id);
            let exists = std::path::Path::new(&att.file_path).exists();
            let missing_suffix = if exists { "" } else { " (missing)" };
            if app.accessible_mode {
                let prefix = if selected { "> " } else { "  " };
                let mark_tag = if marked { "[*] " } else { "" };
                let kind_short = if att.is_managed { "managed" } else { "linked" };
                let note_tag = att
                    .note_id
                    .map(|nid| format!(" (Comment #{nid})"))
                    .unwrap_or_default();
                lines.push(Line::from(format!(
                    "{prefix}{mark_tag}{num}. {name} ({kind_short}){note_tag} {path}{missing_suffix}",
                    num = idx + 1,
                    name = att.file_name,
                    path = att.file_path,
                )));
            } else {
                let cursor_glyph = if selected { "▶ " } else { "  " };
                let mark_box = if marked { "[x] " } else { "[ ] " };
                let kind_badge = if att.is_image() { "IMG" } else { "FILE" };
                let storage_badge = if att.is_managed { "managed" } else { "linked" };
                let note_tag = att
                    .note_id
                    .map(|nid| format!(" (Comment #{nid})"))
                    .unwrap_or_default();
                let alt = if att.description.is_empty() { "(no alt text)" } else { att.description.as_str() };
                let row_style = if selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {cursor_glyph}{mark_box}{num}. #{id} [{kind_badge}/{storage_badge}] {name} ({mime}){note_tag}{missing_suffix} — Alt: {alt}",
                        num = idx + 1,
                        id = att.id,
                        name = att.file_name,
                        mime = att.media_type,
                    ),
                    row_style,
                )));
            }
            if selected {
                let storage_label = if att.is_managed { "Managed copy" } else { "Linked file" };
                let meta_size = std::fs::metadata(&att.file_path)
                    .ok()
                    .map(|m| format_file_size(m.len()))
                    .unwrap_or_else(|| "missing".to_string());
                let added = att
                    .created_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
                let detail_style = Style::default().fg(Color::Gray);
                lines.push(Line::from(Span::styled(format!("      Path: {}{missing_suffix}", att.file_path), detail_style)));
                lines.push(Line::from(Span::styled(
                    format!("      {storage_label} | {} | {meta_size} | Added {added} | Alt: {}", att.media_type, att.alt_text()),
                    detail_style,
                )));
            }
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "Relationships & Dependencies:",
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
        lines.push(Line::from("  (No comments yet)"));
    } else {
        let cursor = app.detail_note_cursor.min(detail.notes.len() - 1);
        for (idx, n) in detail.notes.iter().enumerate() {
            let marker = if idx == cursor { "▶" } else { "•" };
            lines.push(Line::from(format!("  {marker} #{} [{}] @{}: {}", n.id, n.created_at.format("%Y-%m-%d %H:%M"), n.author, n.body)));
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

    // `@` / `#` completions plus resolved-token preview for the comment prompt.
    let suggestion_lines: Vec<Line> = if app.detail_prompt_kind == DetailPromptKind::AddComment {
        let mut lines: Vec<Line> = Vec::new();
        for (idx, suggestion) in app.detail_suggestions.iter().enumerate() {
            let selected = idx == app.detail_suggestion_index;
            let style = if selected { Style::default().fg(Color::Black).bg(Color::Cyan) } else { Style::default().fg(Color::Gray) };
            let marker = if selected { "▶ " } else { "  " };
            lines.push(Line::from(Span::styled(format!("{marker}{}  ({})", suggestion.label, suggestion.hint), style)));
        }
        if !app.detail_comment_preview.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  ↳ {}", app.detail_comment_preview.join("  ·  ")),
                Style::default().fg(Color::Green),
            )));
        }
        lines
    } else {
        Vec::new()
    };
    let suggestion_rows = suggestion_lines.len() as u16;
    // The prompt line is only drawn while a prompt is actually open; its
    // keybindings live in the footer, not inside the modal.
    let prompt_rows = if app.detail_prompt_kind == DetailPromptKind::None { 0 } else { 3 };

    let [body_area, suggestion_area, prompt_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(suggestion_rows), Constraint::Length(prompt_rows)]).areas(inner_area);

    if suggestion_rows > 0 {
        frame.render_widget(Paragraph::new(suggestion_lines), suggestion_area);
    }

    // Count wrapped rows, otherwise long descriptions leave the tail unreachable.
    let wrapped_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(body_area.width.max(1));
    let max_scroll = wrapped_rows
        .saturating_sub(body_area.height as usize)
        .min(u16::MAX as usize) as u16;
    detail.scroll = detail.scroll.min(max_scroll);

    let body = Paragraph::new(lines.clone())
        .scroll((detail.scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, body_area);

    if !app.accessible_mode && wrapped_rows > body_area.height as usize {
        let mut scrollbar_state = ScrollbarState::new(wrapped_rows).position(detail.scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, body_area, &mut scrollbar_state);
    }

    let (prompt_line, prompt_prefix_len) = match app.detail_prompt_kind {
        DetailPromptKind::None => (Line::default(), None),
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
            let prefix = "Edit Attachment Alt Text: ";
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
        DetailPromptKind::ConfirmDeleteTask | DetailPromptKind::ConfirmDeleteNote => {
            let prefix = match app.detail_prompt_kind {
                DetailPromptKind::ConfirmDeleteTask => "Delete this task? Type yes: ",
                _ => "Delete selected comment? Type yes: ",
            };
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

    if prompt_rows > 0 {
        let prompt_block = Block::default()
            .borders(panel_borders(Borders::TOP, app.accessible_mode))
            .border_style(Style::default().fg(Color::DarkGray));
        let prompt_inner = prompt_block.inner(prompt_area);
        let prompt_widget = Paragraph::new(prompt_line)
            .block(prompt_block)
            .wrap(Wrap { trim: false });
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
}

/// Shared layout for the searchable filter popups (` Project Filter `, ` User Filter `).
struct FilterModal<'a> {
    title: &'a str,
    query: &'a str,
    selected: usize,
    options: Vec<(String, String)>,
    current: String,
    empty: &'a str,
}

fn render_filter_modal(frame: &mut Frame, area: Rect, accessible: bool, modal: &FilterModal<'_>) {
    let popup_area = centered_rect(56, 64, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(Span::styled(
            modal.title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(panel_borders(Borders::ALL, accessible))
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    if inner.height < 3 || inner.width < 8 {
        return;
    }

    let [input_area, list_area] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(inner);

    let input = Paragraph::new(Line::from(format!(" Search: {}_", modal.query)).style(Style::default().fg(Color::Yellow))).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(input, input_area);
    let cursor_x = input_area
        .x
        .saturating_add((" Search: ".len() + modal.query.chars().count()) as u16)
        .min(input_area.right().saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, input_area.y));

    let lines: Vec<Line> = if modal.options.is_empty() {
        vec![Line::from(Span::styled(modal.empty, Style::default().fg(Color::DarkGray)))]
    } else {
        modal
            .options
            .iter()
            .enumerate()
            .map(|(idx, (value, label))| {
                let selected = idx == modal.selected;
                let active = value.eq_ignore_ascii_case(&modal.current);
                let marker = if selected { "▶ " } else { "  " };
                let check = if active { "✓ " } else { "  " };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if active {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{marker}{check}{label}"), style))
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), list_area);
}

fn render_project_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    render_filter_modal(
        frame,
        area,
        app.accessible_mode,
        &FilterModal {
            title: match app.project_modal_mode {
                PickerMode::Filter => " Project Filter ",
                PickerMode::Switch => " Project Switcher ",
            },
            query: &app.project_modal_query,
            selected: app.project_modal_selected,
            options: app.filtered_project_options(),
            current: app.project_modal_current(),
            empty: "  No matching projects",
        },
    );
}

fn render_user_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    render_filter_modal(
        frame,
        area,
        app.accessible_mode,
        &FilterModal {
            title: match (app.user_modal_mode, app.user_filter_allows_all()) {
                (PickerMode::Switch, _) => " User Switcher ",
                (PickerMode::Filter, true) => " User Filter ",
                (PickerMode::Filter, false) => " User Filter (single user) ",
            },
            query: &app.user_modal_query,
            selected: app.user_modal_selected,
            options: app.filtered_user_options(),
            current: app.user_modal_current(),
            empty: "  No matching users",
        },
    );
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
    lines.push(Line::from(format!(
        "  Active User: @{}  |  Active Project: {}  |  Focus Preset: {}  |  Mode: {}",
        app.user, app.active_project, app.preset, mode_label
    )));
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

    let modal_block =
        panel_block(" Help, Status & Licenses ", Borders::ALL, app.accessible_mode).border_style(Style::default().fg(Color::Cyan));
    let inner_area = modal_block.inner(popup_area);
    frame.render_widget(modal_block, popup_area);

    // License text wraps, so scroll bounds must come from the wrapped row count.
    let wrapped_rows = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner_area.width.max(1));
    let max_scroll = wrapped_rows
        .saturating_sub(inner_area.height as usize)
        .min(u16::MAX as usize) as u16;
    app.about_scroll = app.about_scroll.min(max_scroll);

    let modal = Paragraph::new(lines)
        .scroll((app.about_scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, inner_area);

    if !app.accessible_mode && wrapped_rows > inner_area.height as usize {
        let mut scrollbar_state = ScrollbarState::new(wrapped_rows).position(app.about_scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, inner_area, &mut scrollbar_state);
    }
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
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
