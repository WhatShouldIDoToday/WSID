use std::io::Write;
use std::path::Path;
use std::process::Command;

use chrono::Utc;
use serde_json::json;
use wsid_core::{
    AppConfig, CreateTaskInput, Database, EisenhowerClassification, EisenhowerQuadrant, FocusQueueResponse, RelationInputKind, SearchQuery,
    Task, TaskAttachment, add_task_note, attach_existing_file, compute_focus_queue, create_task, delete_task, delete_task_note,
    get_attachment, get_blocked_tasks, get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks,
    get_parallel_peers, get_related_tasks, get_task, link_tasks, list_eisenhower_matrix, list_task_notes, open_file_in_os,
    remove_attachment, save_clipboard_attachment, search_tasks, transition_task_status, unblock_or_dismiss_subtask, unlink_tasks,
    update_attachment_description,
};
use wsid_mcp::McpServer;

use crate::args::{
    AddArgs, AttachArgs, Commands, NextArgs, NoteArgs, SearchArgs, format_duration_estimate, parse_date_arg, parse_duration_estimate,
    parse_preset_arg, parse_relation_arg, parse_status_arg,
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
    persona: &str,
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
        Commands::Next(args) => run_next(db_path, config, persona, &args, mode, out),
        Commands::Add(args) => run_add(db_path, persona, &args, mode, out),
        Commands::Show { id } => run_show(db_path, &id, mode, out),
        Commands::Status { id, status } => run_status(db_path, &id, &status, mode, out),
        Commands::Note(args) => run_note(db_path, persona, &args, mode, out),
        Commands::DeleteNote { note_id } => run_delete_note(db_path, note_id, mode, out),
        Commands::Delete { id } => run_delete(db_path, &id, mode, out),
        Commands::Attach(args) => run_attach(db_path, &args, mode, out),
        Commands::UpdateAttachment { attachment_id, desc } => run_update_attachment(db_path, attachment_id, desc, mode, out),
        Commands::RemoveAttachment { attachment_id } => run_remove_attachment(db_path, attachment_id, mode, out),
        Commands::OpenAttachment { attachment_id } => run_open_attachment(db_path, attachment_id, mode, out),
        Commands::Link { source_id, relation, target_id } => run_link(db_path, &source_id, &relation, &target_id, mode, out),
        Commands::Unlink { source_id, relation, target_id } => run_unlink(db_path, &source_id, &relation, &target_id, mode, out),
        Commands::Unblock { subtask_id, keep_open } => run_unblock(db_path, &subtask_id, keep_open, mode, out),
        Commands::Search(args) => run_search(db_path, &args, mode, out),
        Commands::Blocked { tag } => run_blocked(db_path, persona, tag.as_deref(), mode, out),
        Commands::Matrix { all } => run_matrix(db_path, config, persona, all, mode, out),
    }
}

fn run_next<W: Write>(
    db_path: &Path,
    config: &AppConfig,
    persona: &str,
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

    let focus = compute_focus_queue(&db, persona, preset, custom_weights, limit, today, config.scoring)?;
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
                "Focus Queue for persona {} using preset {}. Actionable tasks count: {}.",
                focus.persona,
                focus.preset,
                focus.actionable_tasks.len()
            )?;
            if focus.actionable_tasks.is_empty() {
                writeln!(out, "No immediately actionable tasks found for {}.", focus.persona)?;
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
                writeln!(out, "Blocked Summary for persona {}:", focus.persona)?;
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
            writeln!(out, "Focus Queue (@{}, preset: {})", focus.persona, focus.preset)?;
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

fn run_add<W: Write>(db_path: &Path, persona: &str, args: &AddArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;

    let status = match args.status.as_deref() {
        Some(s) => Some(parse_status_arg(s)?),
        None => None,
    };
    let due_date = parse_date_arg("due", args.due.as_deref())?;
    let sgdb_date = parse_date_arg("sgdb", args.sgdb.as_deref())?;
    let time_estimate_minutes = match args.estimate.as_deref() {
        Some(raw) => Some(parse_duration_estimate(raw)?),
        None => None,
    };

    let created = create_task(
        &mut db,
        CreateTaskInput {
            title: args.title.clone(),
            description: args.description.clone(),
            status,
            due_date,
            sgdb_date,
            time_estimate_minutes,
            difficulty: args.difficulty,
            importance: args.importance,
            urgent_override: if args.urgent { Some(true) } else { None },
            manual_priority: args.priority,
            tags: args.tags.clone(),
            reporter: args.reporter.clone(),
            assignee: args.assignee.clone(),
            evaluator: args.evaluator.clone(),
            ..Default::default()
        },
        persona,
    )?;

    for target in &args.blocks {
        link_tasks(&mut db, &created.id, RelationInputKind::Blocks, target)?;
    }
    for target in &args.blocked_by {
        link_tasks(&mut db, &created.id, RelationInputKind::BlockedBy, target)?;
    }
    for target in &args.subtask_of {
        link_tasks(&mut db, &created.id, RelationInputKind::SubTaskOf, target)?;
    }
    for target in &args.parent_of {
        link_tasks(&mut db, &created.id, RelationInputKind::ParentTaskOf, target)?;
    }
    for target in &args.parallel {
        link_tasks(&mut db, &created.id, RelationInputKind::Parallel, target)?;
    }
    for target in &args.related {
        link_tasks(&mut db, &created.id, RelationInputKind::Related, target)?;
    }

    for (idx, file_path) in args.attachments.iter().enumerate() {
        let desc = args.attachment_descriptions.get(idx).map(String::as_str);
        attach_existing_file(&mut db, &created.id, None, file_path, desc)?;
    }

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
                    .map_or_else(|| "none".to_string(), |d| d.to_string()),
                task.sgdb_date
                    .map_or_else(|| "none".to_string(), |d| d.to_string()),
                format_duration_estimate(task.time_estimate_minutes)
            )?;
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
                    .map_or_else(|| "-".to_string(), |d| d.to_string()),
                task.sgdb_date
                    .map_or_else(|| "-".to_string(), |d| d.to_string()),
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

fn run_note<W: Write>(db_path: &Path, persona: &str, args: &NoteArgs, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let author = args.author.as_deref().unwrap_or(persona);
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

fn run_delete<W: Write>(db_path: &Path, raw_id: &str, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let mut db = Database::open(db_path)?;
    let task = get_task(&db, raw_id)?;
    delete_task(&mut db, &task.id)?;
    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&json!({ "deleted_task_id": task.id })).unwrap_or_default())?;
        }
        OutputMode::Accessible | OutputMode::Standard => {
            writeln!(out, "Deleted task {} — {}.", task.id, task.title)?;
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

fn run_blocked<W: Write>(db_path: &Path, persona: &str, tag_filter: Option<&str>, mode: OutputMode, out: &mut W) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let blocked = get_blocked_tasks(&db, persona, tag_filter)?;

    match mode {
        OutputMode::Json => {
            writeln!(out, "{}", serde_json::to_string_pretty(&blocked).unwrap_or_default())?;
        }
        OutputMode::Accessible => {
            writeln!(out, "Blocked tasks for persona {}: {} task(s).", persona, blocked.len())?;
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
            writeln!(out, "Blocked tasks for @{} ({}):", persona, blocked.len())?;
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

fn run_matrix<W: Write>(
    db_path: &Path,
    config: &AppConfig,
    persona: &str,
    all: bool,
    mode: OutputMode,
    out: &mut W,
) -> wsid_core::Result<()> {
    let db = Database::open(db_path)?;
    let filter = if all { None } else { Some(persona) };
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
