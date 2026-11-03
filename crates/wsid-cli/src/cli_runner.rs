use std::io::Write;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use serde_json::json;
use wsid_core::{
    AppConfig, AttachmentInput, BulkEditOutcome, BulkTaskPatch, Database, EisenhowerClassification, EisenhowerQuadrant, FocusQueueResponse,
    RelationDraft, RelationInputKind, Scalar, SearchQuery, THIRD_PARTY_NOTICES_TEXT, Task, TaskAttachment, TaskDraft, add_task_note,
    attach_existing_file, bulk_delete_tasks, bulk_update_tasks, compute_focus_queue_in_project, create_task_from_draft, delete_task_note,
    delete_user_from_db, format_deadline_display, get_about_metadata, get_attachment, get_blocked_tasks, get_direct_blocked_tasks,
    get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers, get_related_tasks, get_task, link_tasks,
    list_eisenhower_matrix, list_known_users, list_task_notes, list_tasks, list_user_kinds, normalize_user, normalize_user_kind,
    open_file_in_os, preview_user_deletion, register_user, remove_attachment, save_clipboard_attachment, search_tasks, set_user_kind,
    transition_task_status, unblock_or_dismiss_subtask, unlink_tasks, update_attachment_description,
};
use wsid_mcp::McpServer;

use crate::args::{
    AddArgs, AttachArgs, Commands, EditArgs, NextArgs, NoteArgs, SearchArgs, UsersAction, format_duration_estimate, parse_preset_arg,
    parse_relation_arg, parse_status_arg,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Standard,
    Accessible,
    Json,
}

impl OutputMode {
    pub const fn from_flags(accessible: bool, json: bool) -> Self {
        if json {
            Self::Json
        } else if accessible {
            Self::Accessible
        } else {
            Self::Standard
        }
    }
}

pub fn execute_command<W: Write>(
    command: Commands,
    db_path: &Path,
    config: &AppConfig,
    user: &str,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    match command {
        Commands::Tui => Ok(()),
        Commands::Mcp { verbose } => {
            let db = Database::open(db_path)?;
            let mut server = McpServer::new(db, config.clone()).with_stderr_logging(verbose);
            server.run_stdio()?;
            Ok(())
        }
        Commands::Next(args) => run_next(db_path, config, user, &args, mode, out),
        Commands::Add(args) => run_add(db_path, user, &args, mode, out),
        Commands::Show { id } => run_show(db_path, &id, mode, out),
        Commands::Status { id, status } => run_status(db_path, &id, &status, mode, out),
        Commands::Note(args) => run_note(db_path, user, &args, mode, out),
        Commands::DeleteNote { note_id } => run_delete_note(db_path, note_id, mode, out),
        Commands::Delete { ids } => run_delete(db_path, &ids, mode, out),
        Commands::Edit(args) => run_edit(db_path, &args, mode, out),
        Commands::Users { action } => run_users(db_path, config, user, action, mode, out),
        Commands::Kanban { all } => run_kanban(db_path, user, all, mode, out),
        Commands::Attach(args) => run_attach(db_path, &args, mode, out),
        Commands::UpdateAttachment { attachment_id, desc } => run_update_attachment(db_path, attachment_id, desc, mode, out),
        Commands::RemoveAttachment { attachment_id } => run_remove_attachment(db_path, attachment_id, mode, out),
        Commands::OpenAttachment { attachment_id } => run_open_attachment(db_path, attachment_id, mode, out),
        Commands::Link { source_id, relation, target_id } => run_link(db_path, &source_id, &relation, &target_id, mode, out),
        Commands::Unlink { source_id, relation, target_id } => run_unlink(db_path, &source_id, &relation, &target_id, mode, out),
        Commands::Unblock { subtask_id, keep_open } => run_unblock(db_path, &subtask_id, keep_open, mode, out),
        Commands::Search(args) => run_search(db_path, &args, mode, out),
        Commands::Blocked { tag } => run_blocked(db_path, user, tag.as_deref(), mode, out),
        Commands::Matrix { all } => run_matrix(db_path, config, user, all, mode, out),
        Commands::About { full } => run_about(full, mode, out),
    }
}

fn run_next<W: Write>(
    db_path: &Path,
    config: &AppConfig,
    user: &str,
    args: &NextArgs,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let preset = match args.preset.as_deref() {
        Some(raw) => parse_preset_arg(raw)?,
        None => config.focus_preset,
    };
    let custom_weights = if args.preset.is_none() { config.custom_weights } else { None };
    let limit = Some(args.limit.unwrap_or(config.focus_limit));
    let today = Utc::now().date_naive();

    let focus = compute_focus_queue_in_project(&db, user, preset, custom_weights, limit, today, config.scoring, args.project.as_deref())?;
    render_focus_queue(&focus, mode, out)
}

pub fn render_focus_queue<W: Write>(focus: &FocusQueueResponse, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(focus).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(
                out,
                "Focus Queue for user {} using preset {}. Actionable tasks count: {}.",
                focus.user,
                focus.preset,
                focus.actionable_tasks.len()
            )?;
            if focus.actionable_tasks.is_empty() {
                writeln!(out, "No immediately actionable tasks found for {}.", focus.user)?;
            } else {
                for (idx, item) in focus.actionable_tasks.iter().enumerate() {
                    let t = &item.task;
                    let deadline_str = item
                        .breakdown
                        .effective_deadline
                        .map_or_else(|| "none".to_string(), |d| d.to_string());
                    let tags_str = if t.tags.is_empty() { "none".to_string() } else { t.tags.join(", ") };
                    writeln!(
                        out,
                        "Rank {}: Task {} — {}. Status: {}. Score: {:.1}. Quadrant: Q{}. Importance: {} of 5. Difficulty: {} of 5. Effective deadline: {}. Tags: {}. Attachments: {}.",
                        idx + 1,
                        t.id,
                        t.title,
                        t.status,
                        item.breakdown.total_score,
                        item.quadrant.number(),
                        t.importance,
                        t.difficulty,
                        deadline_str,
                        tags_str,
                        t.attachments.len()
                    )?;
                }
            }
            if !focus.blocked_summary.is_empty() {
                writeln!(out, "Blocked Summary for user {}:", focus.user)?;
                for grp in &focus.blocked_summary {
                    writeln!(
                        out,
                        "Project or tag {}: {} task(s) ({}) blocked by {} waiting on @{}.",
                        grp.project_or_tag,
                        grp.count,
                        grp.task_ids.join(", "),
                        grp.blocker_kind,
                        grp.waiting_on_assignee
                    )?;
                }
            }
        }
        OutputMode::Standard => {
            writeln!(out, "Focus Queue (@{}, preset: {})", focus.user, focus.preset)?;
            if focus.actionable_tasks.is_empty() {
                writeln!(out, "  (No actionable unblocked tasks)")?;
            } else {
                for (idx, item) in focus.actionable_tasks.iter().enumerate() {
                    let t = &item.task;
                    let deadline_badge = item
                        .breakdown
                        .effective_deadline
                        .map_or_else(String::new, |d| format!(" due:{d}"));
                    let tags_badge = if t.tags.is_empty() { String::new() } else { format!(" [{}]", t.tags.join(", ")) };
                    let attach_badge = if t.attachments.is_empty() { String::new() } else { format!(" (files:{})", t.attachments.len()) };
                    writeln!(
                        out,
                        "  {}. [{}] {} (score: {:.1}, Q{}, status: {}, imp:{}/diff:{}{}{}{})",
                        idx + 1,
                        t.id,
                        t.title,
                        item.breakdown.total_score,
                        item.quadrant.number(),
                        t.status,
                        t.importance,
                        t.difficulty,
                        deadline_badge,
                        tags_badge,
                        attach_badge
                    )?;
                }
            }

            if !focus.blocked_summary.is_empty() {
                writeln!(out, "\nBlocked / Waiting on Others:")?;
                for grp in &focus.blocked_summary {
                    writeln!(
                        out,
                        "  - [{}] {} task(s) ({}) waiting on @{} ({})",
                        grp.project_or_tag,
                        grp.count,
                        grp.task_ids.join(", "),
                        grp.waiting_on_assignee,
                        grp.blocker_kind
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn run_add<W: Write>(db_path: &Path, user: &str, args: &AddArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;

    let status = match args.status.as_deref() {
        Some(s) => Some(parse_status_arg(s)?),
        None => None,
    };

    let relation_groups: [(RelationInputKind, &Vec<String>); 6] = [
        (RelationInputKind::Blocks, &args.blocks),
        (RelationInputKind::BlockedBy, &args.blocked_by),
        (RelationInputKind::SubTaskOf, &args.subtask_of),
        (RelationInputKind::ParentTaskOf, &args.parent_of),
        (RelationInputKind::Parallel, &args.parallel),
        (RelationInputKind::Related, &args.related),
    ];
    let relations = relation_groups
        .iter()
        .flat_map(|(kind, targets)| {
            targets
                .iter()
                .map(|t| RelationDraft { relation: kind.name().to_string(), target: t.clone() })
        })
        .collect();

    let attachments = args
        .attachments
        .iter()
        .enumerate()
        .map(|(idx, path)| AttachmentInput {
            path: path.to_string_lossy().to_string(),
            description: args.attachment_descriptions.get(idx).cloned(),
        })
        .collect();

    let draft = TaskDraft {
        title: args.title.clone(),
        description: args.description.clone(),
        status,
        due_date: args.due.clone(),
        sgdb_date: args.sgdb.clone(),
        time_estimate: args.estimate.clone().map(Scalar::Text),
        difficulty: args.difficulty.map(|n| Scalar::Int(i64::from(n))),
        importance: args.importance.map(|n| Scalar::Int(i64::from(n))),
        manual_priority: args.priority.map(|n| Scalar::Int(i64::from(n))),
        urgent_override: if args.urgent { Some(Scalar::Bool(true)) } else { None },
        tags: args.tags.clone(),
        project: args.project.clone(),
        relations,
        attachments,
        reporter: args.reporter.clone(),
        assignee: args.assignee.clone(),
        evaluator: args.evaluator.clone(),
    };

    // Atomic: nothing is created if any relation target, attachment, or field is invalid.
    let created = create_task_from_draft(&mut db, &draft, user)?;

    let refreshed = get_task(&db, &created.id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&refreshed).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(
                out,
                "Created task {} — {}. Status: {}. Assignee: @{}. Evaluator: @{}. Attachments count: {}.",
                refreshed.id,
                refreshed.title,
                refreshed.status,
                refreshed.assignee,
                refreshed.evaluator,
                refreshed.attachments.len()
            )?;
            render_attachments_accessible(&refreshed.attachments, out)?;
        }
        OutputMode::Standard => {
            writeln!(
                out,
                "Created task [{}] {} (status: {}, assignee: @{}, evaluator: @{}, attachments: {})",
                refreshed.id,
                refreshed.title,
                refreshed.status,
                refreshed.assignee,
                refreshed.evaluator,
                refreshed.attachments.len()
            )?;
        }
    }
    Ok(())
}

fn run_show<W: Write>(db_path: &Path, raw_id: &str, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let task = get_task(&db, raw_id)?;
    let blockers = get_direct_blockers(&db, &task.id)?;
    let blocking = get_direct_blocked_tasks(&db, &task.id)?;
    let parents = get_direct_parents(&db, &task.id)?;
    let subtasks = get_direct_subtasks(&db, &task.id)?;
    let parallel = get_parallel_peers(&db, &task.id)?;
    let related = get_related_tasks(&db, &task.id)?;
    let notes = list_task_notes(&db, &task.id)?;

    match mode {
        OutputMode::Json => {
            let payload = json!({
                "task": task,
                "blockers": blockers,
                "blocking": blocking,
                "parents": parents,
                "subtasks": subtasks,
                "parallel": parallel,
                "related": related,
                "notes": notes,
            });
            writeln!(out, "{}", serde_json::to_string_pretty(&payload).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(out, "Task {} — {}.", task.id, task.title)?;
            writeln!(
                out,
                "Status: {}. Origin: {}. Importance: {} of 5. Difficulty: {} of 5. Urgent override: {}. Manual priority: {}.",
                task.status, task.origin_kind, task.importance, task.difficulty, task.urgent_override, task.manual_priority
            )?;
            writeln!(out, "Reporter: @{}. Assignee: @{}. Evaluator: @{}.", task.reporter, task.assignee, task.evaluator)?;
            writeln!(
                out,
                "Due date: {}. Should Get Done By date: {}. Time estimate: {}.",
                task.due_date
                    .map_or_else(|| "none".to_string(), |d| format_deadline_display(&d)),
                task.sgdb_date
                    .map_or_else(|| "none".to_string(), |d| format_deadline_display(&d)),
                format_duration_estimate(task.time_estimate_minutes)
            )?;
            writeln!(out, "Project: {}.", task.project)?;
            writeln!(out, "Tags: {}.", if task.tags.is_empty() { "none".to_string() } else { task.tags.join(", ") })?;
            if !task.description.trim().is_empty() {
                writeln!(out, "Description: {}", task.description)?;
            }
            render_attachments_accessible(&task.attachments, out)?;
            render_relation_line_accessible("Blocked by", &blockers, out)?;
            render_relation_line_accessible("Blocks", &blocking, out)?;
            render_relation_line_accessible("Subtask of", &parents, out)?;
            render_relation_line_accessible("Parent of subtasks", &subtasks, out)?;
            render_relation_line_accessible("Parallel with", &parallel, out)?;
            render_relation_line_accessible("Related to", &related, out)?;

            writeln!(out, "Task Notes count: {}.", notes.len())?;
            for note in &notes {
                writeln!(out, "Note #{} by @{} at {}: {}", note.id, note.author, note.created_at.format("%Y-%m-%d %H:%M UTC"), note.body)?;
                render_attachments_accessible(&note.attachments, out)?;
            }
        }
        OutputMode::Standard => {
            writeln!(out, "[{}] {}", task.id, task.title)?;
            writeln!(
                out,
                "  Status: {} | Origin: {} | Importance: {}/5 | Difficulty: {}/5 | Urgent: {} | Priority: {}",
                task.status, task.origin_kind, task.importance, task.difficulty, task.urgent_override, task.manual_priority
            )?;
            writeln!(out, "  Roles: reporter=@{}, assignee=@{}, evaluator=@{}", task.reporter, task.assignee, task.evaluator)?;
            writeln!(
                out,
                "  Dates: due={}, sgdb={}, estimate={}",
                task.due_date
                    .map_or_else(|| "-".to_string(), |d| format_deadline_display(&d)),
                task.sgdb_date
                    .map_or_else(|| "-".to_string(), |d| format_deadline_display(&d)),
                format_duration_estimate(task.time_estimate_minutes)
            )?;
            if !task.tags.is_empty() {
                writeln!(out, "  Tags: {}", task.tags.join(", "))?;
            }
            if !task.description.trim().is_empty() {
                writeln!(out, "  Description:\n    {}", task.description.replace('\n', "\n    "))?;
            }
            render_attachments_standard("  Task Attachments", &task.attachments, out)?;
            render_relation_line_standard("  Blocked by", &blockers, out)?;
            render_relation_line_standard("  Blocks", &blocking, out)?;
            render_relation_line_standard("  Subtask of", &parents, out)?;
            render_relation_line_standard("  Subtasks", &subtasks, out)?;
            render_relation_line_standard("  Parallel", &parallel, out)?;
            render_relation_line_standard("  Related", &related, out)?;

            if !notes.is_empty() {
                writeln!(out, "  Notes ({}):", notes.len())?;
                for note in &notes {
                    writeln!(out, "    #{} @{} ({}): {}", note.id, note.author, note.created_at.format("%Y-%m-%d %H:%M"), note.body)?;
                    render_attachments_standard("      Note Attachments", &note.attachments, out)?;
                }
            }
        }
    }
    Ok(())
}

fn render_attachments_accessible<W: Write>(attachments: &[TaskAttachment], out: &mut W) -> wsid_core::Result<()> {
    for att in attachments {
        writeln!(
            out,
            "Attachment #{} ({}) — Alt description: \"{}\". Disk file path: {}. Managed in app data: {}.",
            att.id,
            att.media_type,
            att.alt_text(),
            att.file_path,
            att.is_managed
        )?;
    }
    Ok(())
}

fn render_attachments_standard<W: Write>(heading: &str, attachments: &[TaskAttachment], out: &mut W) -> wsid_core::Result<()> {
    if attachments.is_empty() {
        return Ok(());
    }
    writeln!(out, "{heading} ({}):", attachments.len())?;
    for att in attachments {
        writeln!(out, "    - [att #{}] {} ({}) alt=\"{}\" -> {}", att.id, att.file_name, att.media_type, att.alt_text(), att.file_path)?;
    }
    Ok(())
}

fn render_relation_line_accessible<W: Write>(label: &str, tasks: &[Task], out: &mut W) -> wsid_core::Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }
    let items: Vec<String> = tasks
        .iter()
        .map(|t| format!("{} ({}, status: {})", t.id, t.title, t.status))
        .collect();
    writeln!(out, "{label}: {}.", items.join("; "))?;
    Ok(())
}

fn render_relation_line_standard<W: Write>(label: &str, tasks: &[Task], out: &mut W) -> wsid_core::Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }
    let items: Vec<String> = tasks
        .iter()
        .map(|t| format!("[{}] {} ({})", t.id, t.title, t.status))
        .collect();
    writeln!(out, "{label}: {}", items.join(", "))?;
    Ok(())
}

fn run_status<W: Write>(db_path: &Path, raw_id: &str, raw_status: &str, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let target_status = parse_status_arg(raw_status)?;
    let outcome = transition_task_status(&mut db, raw_id, target_status)?;

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&outcome).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(out, "Task {} — {} is now in status {}.", outcome.task.id, outcome.task.title, outcome.task.status)?;
            if let Some(ref rev) = outcome.spawned_review_task {
                writeln!(out, "Spawned blocking review subtask {} assigned to @{}: {}.", rev.id, rev.assignee, rev.title)?;
            }
            for parent in &outcome.auto_completed_parent_tasks {
                writeln!(out, "Automatically completed reviewed parent task {} — {}.", parent.id, parent.title)?;
            }
        }
        OutputMode::Standard => {
            writeln!(out, "Updated [{}] {} -> status: {}", outcome.task.id, outcome.task.title, outcome.task.status)?;
            if let Some(ref rev) = outcome.spawned_review_task {
                writeln!(out, "  Spawned review subtask [{}] assigned to @{}", rev.id, rev.assignee)?;
            }
            for parent in &outcome.auto_completed_parent_tasks {
                writeln!(out, "  Auto-completed parent task [{}] {}", parent.id, parent.title)?;
            }
        }
    }
    Ok(())
}

fn run_note<W: Write>(db_path: &Path, user: &str, args: &NoteArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let author = args.author.as_deref().unwrap_or(user);
    let mut outcome = add_task_note(&mut db, &args.id, author, &args.body, !args.no_spawn)?;

    for (idx, file_path) in args.attachments.iter().enumerate() {
        let desc = args.attachment_descriptions.get(idx).map(String::as_str);
        let att = attach_existing_file(&mut db, &outcome.note.task_id, Some(outcome.note.id), file_path, desc)?;
        outcome.note.attachments.push(att);
    }

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&outcome).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(
                out,
                "Added note #{} on task {} by @{}. Spawned tasks: {}. Reused tasks: {}.",
                outcome.note.id,
                outcome.note.task_id,
                outcome.note.author,
                outcome.spawned_tasks.len(),
                outcome.reused_tasks.len()
            )?;
            render_attachments_accessible(&outcome.note.attachments, out)?;
            for spawned in &outcome.spawned_tasks {
                writeln!(out, "Spawned {} task {} assigned to @{}: {}.", spawned.origin_kind, spawned.id, spawned.assignee, spawned.title)?;
            }
        }
        OutputMode::Standard => {
            writeln!(out, "Added note #{} on [{}] by @{}", outcome.note.id, outcome.note.task_id, outcome.note.author)?;
            render_attachments_standard("  Note Attachments", &outcome.note.attachments, out)?;
            for spawned in &outcome.spawned_tasks {
                writeln!(
                    out,
                    "  Spawned {} task [{}] assigned to @{}: {}",
                    spawned.origin_kind, spawned.id, spawned.assignee, spawned.title
                )?;
            }
        }
    }
    Ok(())
}

fn run_delete_note<W: Write>(db_path: &Path, note_id: i64, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    delete_task_note(&mut db, note_id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&json!({ "deleted_note_id": note_id })).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Deleted note #{note_id}.")?;
        }
    }
    Ok(())
}

fn render_bulk<W: Write>(verb: &str, outcome: &BulkEditOutcome, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    match mode {
        OutputMode::Json => writeln!(out, "{}", serde_json::to_string_pretty(outcome).unwrap_or_default())?,
        OutputMode::Accessible | OutputMode::Standard => {
            for t in &outcome.updated {
                writeln!(out, "{verb} task {} — {} (status {}).", t.id, t.title, t.status)?;
            }
            for id in &outcome.deleted {
                writeln!(out, "{verb} task {id}.")?;
            }
            for r in &outcome.spawned_review_tasks {
                writeln!(out, "Spawned review subtask {} assigned to @{}.", r.id, r.assignee)?;
            }
            for f in &outcome.failures {
                writeln!(out, "Failed {}: {}", f.id, f.error)?;
            }
        }
    }
    if outcome.failures.is_empty() {
        Ok(())
    } else {
        Err(wsid_core::WsidError::Validation(format!("{} task(s) failed", outcome.failures.len())))
    }
}

fn run_delete<W: Write>(db_path: &Path, ids: &[String], mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let outcome = bulk_delete_tasks(&mut db, ids)?;
    render_bulk("Deleted", &outcome, mode, out)
}

fn run_edit<W: Write>(db_path: &Path, args: &EditArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let patch = BulkTaskPatch {
        status: args.status.as_deref().map(parse_status_arg).transpose()?,
        assignee: args.assignee.clone(),
        importance: args.importance,
        difficulty: args.difficulty,
        urgent_override: if args.urgent {
            Some(true)
        } else if args.not_urgent {
            Some(false)
        } else {
            None
        },
        add_tags: args.add_tags.clone(),
        remove_tags: args.remove_tags.clone(),
        project: args.project.clone(),
        ..BulkTaskPatch::default()
    };
    let outcome = bulk_update_tasks(&mut db, &args.ids, &patch)?;
    render_bulk("Updated", &outcome, mode, out)
}

fn run_users<W: Write>(
    db_path: &Path,
    config: &AppConfig,
    user: &str,
    action: UsersAction,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    match action {
        UsersAction::List => {
            let users = list_known_users(&db, &[config.primary_user.as_str(), config.default_user.as_str()])?;
            let kinds = list_user_kinds(&db)?;
            match mode {
                OutputMode::Json => writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json!({ "primary_user": config.primary_user, "users": users, "kinds": kinds }))
                        .unwrap_or_default()
                )?,
                OutputMode::Accessible | OutputMode::Standard => {
                    for u in users {
                        let tag = if u.eq_ignore_ascii_case(&config.primary_user) { " (main user)" } else { "" };
                        let kind = kinds
                            .get(&u)
                            .map(String::as_str)
                            .unwrap_or(wsid_core::DEFAULT_USER_KIND);
                        let kind_tag = if kind == wsid_core::DEFAULT_USER_KIND { String::new() } else { format!(" [{kind}]") };
                        writeln!(out, "@{u}{tag}{kind_tag}")?;
                    }
                }
            }
        }
        UsersAction::Kind { user: target, kind } => {
            set_user_kind(&mut db, &target, &kind)?;
            let norm_user = normalize_user(&target)?;
            let norm_kind = normalize_user_kind(&kind)?;
            match mode {
                OutputMode::Json => {
                    writeln!(out, "{}", serde_json::to_string_pretty(&json!({ "user": norm_user, "kind": norm_kind })).unwrap_or_default())?
                }
                _ => writeln!(out, "Set  kind to {norm_kind}.")?,
            }
        }
        UsersAction::Add { user: new_user } => {
            let registered = register_user(&mut db, &new_user)?;
            match mode {
                OutputMode::Json => {
                    writeln!(out, "{}", serde_json::to_string_pretty(&json!({ "registered": registered })).unwrap_or_default())?
                }
                _ => writeln!(out, "Registered @{registered}.")?,
            }
        }
        UsersAction::Delete { user: target, delete_authored, yes } => {
            if !user.eq_ignore_ascii_case(&config.primary_user) {
                return Err(wsid_core::WsidError::Validation(format!(
                    "only the main user '@{}' can delete users (active user is '@{user}')",
                    config.primary_user
                )));
            }
            if !yes {
                let preview = preview_user_deletion(&db, &target, &config.primary_user)?;
                match mode {
                    OutputMode::Json => writeln!(out, "{}", serde_json::to_string_pretty(&preview).unwrap_or_default())?,
                    _ => {
                        writeln!(
                            out,
                            "Deleting @{} would reassign {} task(s) to @{} ({} authored, {} assigned, {} validated).",
                            preview.username,
                            preview.affected_tasks.len(),
                            preview.fallback_user,
                            preview.authored_task_count,
                            preview.assigned_task_count,
                            preview.evaluator_task_count
                        )?;
                        for t in &preview.affected_tasks {
                            writeln!(out, "  [{}] {} ({})", t.id, t.title, t.status)?;
                        }
                        writeln!(out, "Re-run with --yes to delete (add --delete-authored to also delete authored tasks).")?;
                    }
                }
            } else {
                let outcome = delete_user_from_db(&mut db, &target, &config.primary_user, delete_authored, None)?;
                match mode {
                    OutputMode::Json => writeln!(out, "{}", serde_json::to_string_pretty(&outcome).unwrap_or_default())?,
                    _ => writeln!(
                        out,
                        "Deleted @{}: reassigned {} task(s) to @{}, deleted {} authored task(s).",
                        outcome.deleted_user, outcome.reassigned_task_count, outcome.fallback_user, outcome.deleted_task_count
                    )?,
                }
            }
        }
    }
    Ok(())
}

fn run_kanban<W: Write>(db_path: &Path, user: &str, all: bool, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let tasks: Vec<Task> = list_tasks(&db)?
        .into_iter()
        .filter(|t| all || t.assignee.eq_ignore_ascii_case(user))
        .collect();
    match mode {
        OutputMode::Json => {
            let columns: serde_json::Map<String, serde_json::Value> = wsid_core::TaskStatus::ALL
                .iter()
                .map(|s| {
                    let col: Vec<&Task> = tasks.iter().filter(|t| t.status == *s).collect();
                    (s.as_str().to_string(), serde_json::to_value(col).unwrap_or_default())
                })
                .collect();
            writeln!(out, "{}", serde_json::to_string_pretty(&columns).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            for s in wsid_core::TaskStatus::ALL {
                let col: Vec<&Task> = tasks.iter().filter(|t| t.status == s).collect();
                writeln!(out, "{} ({} tasks):", s, col.len())?;
                for t in col {
                    writeln!(out, "  [{}] {} (@{}, imp {}/5)", t.id, t.title, t.assignee, t.importance)?;
                }
            }
        }
    }
    Ok(())
}

fn run_attach<W: Write>(db_path: &Path, args: &AttachArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let attachment = if let Some(ref file_path) = args.file {
        attach_existing_file(&mut db, &args.id, args.note, file_path, args.desc.as_deref())?
    } else if args.clipboard {
        let (bytes, file_name, _media_type) = read_system_clipboard_attachment()?;
        save_clipboard_attachment(&mut db, &args.id, args.note, &bytes, Some(&file_name), args.desc.as_deref())?
    } else {
        return Err(wsid_core::WsidError::Validation("must specify either `--file <path>` or `--clipboard`".to_string()));
    };

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&attachment).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            render_attachments_accessible(std::slice::from_ref(&attachment), out)?;
        }
        OutputMode::Standard => {
            writeln!(
                out,
                "Attached [att #{}] {} ({}) alt=\"{}\" -> {}",
                attachment.id,
                attachment.file_name,
                attachment.media_type,
                attachment.alt_text(),
                attachment.file_path
            )?;
        }
    }
    Ok(())
}

fn run_update_attachment<W: Write>(
    db_path: &Path,
    attachment_id: i64,
    desc: Option<String>,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let updated = update_attachment_description(&mut db, attachment_id, desc.as_deref().unwrap_or(""))?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&updated).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            render_attachments_accessible(std::slice::from_ref(&updated), out)?;
        }
        OutputMode::Standard => {
            writeln!(out, "Updated attachment #{} alt description to \"{}\" ({})", updated.id, updated.alt_text(), updated.file_path)?;
        }
    }
    Ok(())
}

fn run_remove_attachment<W: Write>(db_path: &Path, attachment_id: i64, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let att = get_attachment(&db, attachment_id)?;
    remove_attachment(&mut db, attachment_id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&att).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Removed attachment #{} ({}).", att.id, att.file_name)?;
        }
    }
    Ok(())
}

fn run_open_attachment<W: Write>(db_path: &Path, attachment_id: i64, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let att = get_attachment(&db, attachment_id)?;
    open_file_in_os(Path::new(&att.file_path))?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&att).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Opened attachment #{} ({}) at {}.", att.id, att.alt_text(), att.file_path)?;
        }
    }
    Ok(())
}

fn run_link<W: Write>(
    db_path: &Path,
    source_id: &str,
    raw_rel: &str,
    target_id: &str,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let kind = parse_relation_arg(raw_rel)?;
    let rel = link_tasks(&mut db, source_id, kind, target_id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&rel).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Linked {} {} {}.", rel.source_id, rel.relation_type, rel.target_id)?;
        }
    }
    Ok(())
}

fn run_unlink<W: Write>(
    db_path: &Path,
    source_id: &str,
    raw_rel: &str,
    target_id: &str,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let kind = parse_relation_arg(raw_rel)?;
    let removed = unlink_tasks(&mut db, source_id, kind, target_id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&json!({ "removed": removed })).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Unlinked {source_id} {raw_rel} {target_id} (removed: {removed}).")?;
        }
    }
    Ok(())
}

fn run_unblock<W: Write>(db_path: &Path, subtask_id: &str, keep_open: bool, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let updated = unblock_or_dismiss_subtask(&mut db, subtask_id, !keep_open)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&updated).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Unblocked subtask {} — {} (status: {}).", updated.id, updated.title, updated.status)?;
        }
    }
    Ok(())
}

fn run_search<W: Write>(db_path: &Path, args: &SearchArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let status = match args.status.as_deref() {
        Some(s) => Some(parse_status_arg(s)?),
        None => None,
    };
    let hits = search_tasks(
        &db,
        &SearchQuery {
            query: args.query.clone(),
            status,
            tag: args.tag.clone(),
            assignee: args.assignee.clone(),
            reporter: args.reporter.clone(),
            evaluator: args.evaluator.clone(),
            project: args.project.clone(),
            limit: args.limit,
        },
    )?;

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&hits).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(out, "Search results count: {}.", hits.len())?;
            for hit in &hits {
                let t = &hit.task;
                writeln!(
                    out,
                    "Task {} — {}. Status: {}. Assignee: @{}. Importance: {} of 5. Difficulty: {} of 5. Score: {}.",
                    t.id, t.title, t.status, t.assignee, t.importance, t.difficulty, hit.score
                )?;
            }
        }
        OutputMode::Standard => {
            writeln!(out, "Search hits ({}):", hits.len())?;
            for hit in &hits {
                let t = &hit.task;
                writeln!(
                    out,
                    "  [{}] {} (status: {}, assignee: @{}, imp:{}/diff:{}, score: {})",
                    t.id, t.title, t.status, t.assignee, t.importance, t.difficulty, hit.score
                )?;
            }
        }
    }
    Ok(())
}

fn run_blocked<W: Write>(db_path: &Path, user: &str, tag_filter: Option<&str>, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let blocked = get_blocked_tasks(&db, user, tag_filter)?;

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&blocked).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(out, "Blocked tasks for user {}: {} task(s).", user, blocked.len())?;
            for entry in &blocked {
                let waiting_str: Vec<String> = entry
                    .blocked_by
                    .iter()
                    .map(|b| format!("{} ({} assigned to @{}, status {})", b.blocker_id, b.origin_kind, b.assignee, b.status))
                    .collect();
                writeln!(
                    out,
                    "Task {} — {} (status: {}). Waiting on: {}.",
                    entry.task_id,
                    entry.title,
                    entry.status,
                    if waiting_str.is_empty() { entry.status.to_string() } else { waiting_str.join("; ") }
                )?;
            }
        }
        OutputMode::Standard => {
            writeln!(out, "Blocked tasks for @{} ({}):", user, blocked.len())?;
            for entry in &blocked {
                let waiting_str: Vec<String> = entry
                    .blocked_by
                    .iter()
                    .map(|b| format!("[{}] @{} ({})", b.blocker_id, b.assignee, b.origin_kind))
                    .collect();
                writeln!(
                    out,
                    "  [{}] {} (status: {}) -> waiting on {}",
                    entry.task_id,
                    entry.title,
                    entry.status,
                    if waiting_str.is_empty() { entry.status.to_string() } else { waiting_str.join(", ") }
                )?;
            }
        }
    }
    Ok(())
}

fn run_matrix<W: Write>(db_path: &Path, config: &AppConfig, user: &str, all: bool, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let filter = if all { None } else { Some(user) };
    let today = Utc::now().date_naive();
    let items = list_eisenhower_matrix(&db, filter, today, config.scoring)?;

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&items).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            let quadrants = [
                (EisenhowerQuadrant::Q1DoFirst, "Quadrant 1 — Do First (Urgent & Important)"),
                (EisenhowerQuadrant::Q2Schedule, "Quadrant 2 — Schedule (Not Urgent & Important)"),
                (EisenhowerQuadrant::Q3DelegateQuick, "Quadrant 3 — Delegate / Quick (Urgent & Not Important)"),
                (EisenhowerQuadrant::Q4Later, "Quadrant 4 — Later (Not Urgent & Not Important)"),
            ];
            for (q, label) in quadrants {
                let q_items: Vec<&EisenhowerClassification> = items.iter().filter(|i| i.quadrant == q).collect();
                writeln!(out, "{label} ({} tasks):", q_items.len())?;
                for item in q_items {
                    let t = &item.task;
                    let slack = item
                        .slack_days
                        .map_or_else(|| "no deadline".to_string(), |s| format!("{s:.1}d slack"));
                    writeln!(out, "  [{}] {} (status: {}, imp: {}, {})", t.id, t.title, t.status, t.importance, slack)?;
                }
            }
        }
    }
    Ok(())
}

/// Reads clipboard content from the OS clipboard (or `WSID_CLIPBOARD_FILE` override in tests/headless scripts).
pub fn read_system_clipboard_attachment() -> wsid_core::Result<(Vec<u8>, String, String)> {
    if let Ok(mock_path) = std::env::var("WSID_CLIPBOARD_FILE")
        && !mock_path.trim().is_empty()
    {
        let p = Path::new(mock_path.trim());
        let bytes = std::fs::read(p)?;
        let file_name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("clipboard.png")
            .to_string();
        let media = wsid_core::infer_media_type(p).to_string();
        return Ok((bytes, file_name, media));
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("pbpaste").output()?;
        if output.status.success() && !output.stdout.is_empty() {
            return Ok((output.stdout, "clipboard.txt".to_string(), "text/plain".to_string()));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = Command::new("wl-paste").arg("--no-newline").output()
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Ok((output.stdout, "clipboard.txt".to_string(), "text/plain".to_string()));
        }
        if let Ok(output) = Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Ok((output.stdout, "clipboard.txt".to_string(), "text/plain".to_string()));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Get-Clipboard"])
            .output()
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Ok((output.stdout, "clipboard.txt".to_string(), "text/plain".to_string()));
        }
    }

    Err(wsid_core::WsidError::Validation(
        "clipboard is empty or could not be read; pass `--file <path>` or copy content to the clipboard first".to_string(),
    ))
}

fn run_about<W: Write>(full: bool, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let about = get_about_metadata();

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&about).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "{} v{}", about.app_full_name, about.version)?;
            writeln!(out, "License: {}", about.license_spdx)?;
            writeln!(out, "Repository: {}", about.repository_url)?;
            writeln!(out)?;
            writeln!(out, "Third-Party Rust License Summary ({} license groups):", about.rust_licenses.licenses.len())?;
            for count in &about.rust_licenses.overview {
                writeln!(out, "  • {} ({}): {} crate(s)", count.name, count.id, count.count)?;
            }
            writeln!(out)?;
            writeln!(out, "Third-Party Crates by License:")?;
            for lic in &about.rust_licenses.licenses {
                writeln!(out, "  [{}] {}", lic.id, lic.name)?;
                for c in &lic.used_by {
                    if let Some(ref repo) = c.repository {
                        writeln!(out, "    - {} v{} ({repo})", c.name, c.version)?;
                    } else {
                        writeln!(out, "    - {} v{}", c.name, c.version)?;
                    }
                }
            }
            if full {
                writeln!(out)?;
                writeln!(out, "================================================================================")?;
                writeln!(out, "PROJECT LICENSE ({})", about.license_spdx)?;
                writeln!(out, "================================================================================")?;
                writeln!(out, "{}", about.license_text)?;
                writeln!(out)?;
                writeln!(out, "================================================================================")?;
                writeln!(out, "FULL THIRD-PARTY LICENSE NOTICES")?;
                writeln!(out, "================================================================================")?;
                writeln!(out, "{THIRD_PARTY_NOTICES_TEXT}")?;
            } else {
                writeln!(out)?;
                writeln!(out, "Tip: run `wsid about --full` (or `wsid licenses --full`) to print full license texts.")?;
            }
        }
    }
    Ok(())
}
