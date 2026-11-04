use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::attachments::{cleanup_managed_files_after_delete, load_note_attachments};
use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::{is_valid_task_id, normalize_task_id};
use crate::models::{
    AddTaskNoteOutcome, CreateTaskInput, OriginKind, RelationInputKind, StatusTransitionOutcome, Task, TaskNote, TaskStatus, UserNotice,
};
use crate::relations::{ensure_task_exists, link_tasks_in_tx};
use crate::tasks::{
    create_task_in_tx, format_utc, get_task, get_task_conn, normalize_user, parse_user_ref, parse_utc, register_user_conn,
    resolve_persona_conn, resolve_user_ref_conn,
};

/// Parsed tokens extracted from a task note body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedNoteTokens {
    /// Ordered, deduplicated `@<user>` mentions that spawn blocking subtasks.
    pub blocking_mentions: Vec<String>,
    /// Ordered, deduplicated `@takenote:<user>` mentions that spawn non-blocking related tasks.
    pub takenote_mentions: Vec<String>,
    /// Ordered, deduplicated `#<TASK_ID>` references normalized to uppercase `AB1234`.
    pub task_references: Vec<String>,
}

/// Parses `@<user>`, `@takenote:<user>`, and `#<task_id>` tokens from a comment body.
pub fn parse_note_tokens(body: &str) -> ParsedNoteTokens {
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let mut blocking_set = BTreeSet::new();
    let mut blocking_mentions = Vec::new();

    let mut takenote_set = BTreeSet::new();
    let mut takenote_mentions = Vec::new();

    let mut task_ref_set = BTreeSet::new();
    let mut task_references = Vec::new();

    while i < len {
        let b = bytes[i];
        let prev_is_ident = i > 0 && is_ident_char(bytes[i - 1]);

        if b == b'@' && !prev_is_ident {
            let start = i + 1;
            let mut token_end = start;
            while token_end < len && !bytes[token_end].is_ascii_whitespace() {
                token_end += 1;
            }
            let mut clean_end = token_end;
            while clean_end > start && is_trailing_punct(bytes[clean_end - 1]) {
                clean_end -= 1;
            }
            let raw_mention = &body[start..clean_end];
            if let Some(rest) = raw_mention
                .strip_prefix("takenote:")
                .or_else(|| raw_mention.strip_prefix("TAKENOTE:"))
                .or_else(|| {
                    if raw_mention.len() > 9 && raw_mention[..9].eq_ignore_ascii_case("takenote:") { Some(&raw_mention[9..]) } else { None }
                })
            {
                if let Ok(user) = normalize_user(user_part(rest))
                    && takenote_set.insert(user.clone())
                {
                    takenote_mentions.push(user);
                }
            } else if !raw_mention.is_empty()
                && let Ok(user) = normalize_user(user_part(raw_mention))
                && blocking_set.insert(user.clone())
            {
                blocking_mentions.push(user);
            }
            if token_end > start {
                i = token_end;
                continue;
            }
        } else if b == b'#' && !prev_is_ident {
            let start = i + 1;
            let mut end = start;
            while end < len && is_ident_char(bytes[end]) {
                end += 1;
            }
            let token = &body[start..end];
            if is_valid_task_id(token)
                && let Ok(norm) = normalize_task_id(token)
                && task_ref_set.insert(norm.clone())
            {
                task_references.push(norm);
            }
            if end > start {
                i = end;
                continue;
            }
        }

        i += 1;
    }

    ParsedNoteTokens { blocking_mentions, takenote_mentions, task_references }
}

/// The username part of a possibly kind-qualified mention (`claude:bob` -> `bob`).
fn user_part(raw: &str) -> &str {
    raw.rsplit(':').next().unwrap_or(raw)
}

/// Strips a case-insensitive `takenote:` directive prefix, returning the rest.
fn strip_takenote_prefix(raw: &str) -> Option<&str> {
    raw.get(..9)
        .filter(|prefix| prefix.eq_ignore_ascii_case("takenote:"))
        .map(|_| &raw[9..])
}

/// Rewrites typed mentions (`@kind:user`, `@takenote:kind:user`) to canonical `@user` /
/// `@takenote:user` form. Each reference is resolved (and created with its kind when new), and
/// conflicts or creations are recorded in `notices`. Plain mentions are left untouched.
pub(crate) fn canonicalize_user_mentions(conn: &Connection, body: &str, notices: &mut Vec<UserNotice>) -> Result<String> {
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut copied = 0;
    let mut i = 0;

    while i < len {
        if bytes[i] != b'@' || (i > 0 && is_ident_char(bytes[i - 1])) {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut token_end = start;
        while token_end < len && !bytes[token_end].is_ascii_whitespace() {
            token_end += 1;
        }
        let mut clean_end = token_end;
        while clean_end > start && is_trailing_punct(bytes[clean_end - 1]) {
            clean_end -= 1;
        }
        let raw_mention = &body[start..clean_end];
        let (takenote, rest) = match strip_takenote_prefix(raw_mention) {
            Some(rest) => (true, rest),
            None => (false, raw_mention),
        };
        if rest.contains(':')
            && let Ok(user_ref) = parse_user_ref(rest)
        {
            let username = resolve_user_ref_conn(conn, &user_ref, notices)?;
            out.push_str(&body[copied..i]);
            out.push('@');
            if takenote {
                out.push_str("takenote:");
            }
            out.push_str(&username);
            copied = clean_end;
        }
        i = token_end.max(i + 1);
    }

    out.push_str(&body[copied..]);
    Ok(out)
}

const fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

const fn is_trailing_punct(b: u8) -> bool {
    matches!(b, b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}' | b'"')
}

/// Applies the rich-text part of a task *description*: mentioned users are registered
/// (so `@someone` in a description makes that user pickable) and existing `#AB1234`
/// references are linked as `related`.
///
/// Comments use the same token parser but additionally spawn subtasks; descriptions
/// deliberately never spawn anything.
pub(crate) fn apply_rich_text_references(conn: &Connection, task_id: &str, body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Ok(());
    }
    let tokens = parse_note_tokens(body);
    for user in tokens
        .blocking_mentions
        .iter()
        .chain(tokens.takenote_mentions.iter())
    {
        let _ = register_user_conn(conn, user);
    }
    for ref_id in &tokens.task_references {
        if ref_id == task_id {
            continue;
        }
        if get_task_conn(conn, ref_id).is_ok() {
            let _ = link_tasks_in_tx(conn, task_id, RelationInputKind::Related, ref_id);
        }
    }
    Ok(())
}

/// Appends a note to `raw_task_id` and optionally spawns `@user` blocking subtasks,
/// `@takenote:<user>` non-blocking related tasks, and `#<task_id>` cross-links.
pub fn add_task_note(
    db: &mut Database,
    raw_task_id: &str,
    raw_author: &str,
    raw_body: &str,
    spawn_subtasks: bool,
) -> Result<AddTaskNoteOutcome> {
    let task_id = normalize_task_id(raw_task_id)?;
    let trimmed_body = raw_body.trim();
    if trimmed_body.is_empty() {
        return Err(WsidError::Validation("task note body cannot be empty".to_string()));
    }

    let commented_task = get_task(db, &task_id)?;
    let now_str = format_utc(Utc::now());

    let tx = db.conn.transaction()?;
    let mut user_notices = Vec::new();
    let author = resolve_persona_conn(&tx, raw_author, &mut user_notices)?;
    let body = canonicalize_user_mentions(&tx, trimmed_body, &mut user_notices)?;
    let tokens = parse_note_tokens(&body);

    tx.execute(
        "INSERT INTO task_notes (task_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![commented_task.id, author, body, now_str],
    )?;
    let _ = register_user_conn(&tx, &author);
    let note_id = tx.last_insert_rowid();

    let note = TaskNote {
        id: note_id,
        task_id: commented_task.id.clone(),
        author: author.clone(),
        body: body.clone(),
        attachments: Vec::new(),
        created_at: parse_utc(&now_str),
    };

    // Determine the logical parent task for blocking subtasks:
    // If the user commented on a `review` subtask `R` (which is a subtask of `P`),
    // blocking `@assignee` feedback should block `P` and close the review round on `P`.
    let review_parent_id =
        if commented_task.origin_kind == OriginKind::Review { find_review_parent_id(&tx, &commented_task.id)? } else { None };
    let effective_parent_id = review_parent_id
        .clone()
        .unwrap_or_else(|| commented_task.id.clone());
    let effective_parent = get_task_conn(&tx, &effective_parent_id)?;

    // Resolve valid existing referenced `#AB1234` tasks
    let mut existing_ref_ids = Vec::new();
    for ref_id in &tokens.task_references {
        if ensure_task_exists(&tx, ref_id).is_ok() {
            if *ref_id != commented_task.id {
                let _ = link_tasks_in_tx(&tx, &commented_task.id, RelationInputKind::Related, ref_id);
            }
            existing_ref_ids.push(ref_id.clone());
        }
    }

    let mut spawned_ids = Vec::new();
    let mut reused_ids = Vec::new();
    let mut completed_review_ids = Vec::new();

    if spawn_subtasks {
        let mut any_blocking_spawned_or_reused = false;

        // 1. Handle `@<user>` blocking mentions
        for mentioned_user in &tokens.blocking_mentions {
            if mentioned_user.eq_ignore_ascii_case(&author) {
                continue;
            }

            any_blocking_spawned_or_reused = true;

            if let Some(existing_subtask_id) = find_open_mention_subtask(&tx, &effective_parent.id, mentioned_user)? {
                append_note_to_existing_subtask(&tx, &existing_subtask_id, &author, note_id, &effective_parent.id, &body, &now_str)?;
                for ref_id in &existing_ref_ids {
                    if *ref_id != existing_subtask_id {
                        let _ = link_tasks_in_tx(&tx, &existing_subtask_id, RelationInputKind::Related, ref_id);
                    }
                }
                reused_ids.push(existing_subtask_id);
            } else {
                let title = format!("Address @{author} note on {}: {}", effective_parent.id, effective_parent.title);
                let description = format!("You were mentioned by @{author} in note #{note_id} on {}:\n\n> {body}", effective_parent.id);
                let new_id = create_task_in_tx(
                    &tx,
                    CreateTaskInput {
                        title,
                        description: Some(description),
                        status: Some(TaskStatus::Todo),
                        origin_kind: Some(OriginKind::Mention),
                        origin_note_id: Some(note_id),
                        due_date: effective_parent.due_date.clone(),
                        sgdb_date: effective_parent.sgdb_date.clone(),
                        importance: Some(effective_parent.importance),
                        urgent_override: Some(true),
                        tags: effective_parent.tags.clone(),
                        project: Some(effective_parent.project.clone()),
                        reporter: Some(author.clone()),
                        assignee: Some(mentioned_user.clone()),
                        evaluator: Some(mentioned_user.clone()),
                        ..Default::default()
                    },
                    &author,
                    &mut Vec::new(),
                )?;

                link_tasks_in_tx(&tx, &new_id, RelationInputKind::Blocks, &effective_parent.id)?;
                link_tasks_in_tx(&tx, &new_id, RelationInputKind::SubTaskOf, &effective_parent.id)?;

                for ref_id in &existing_ref_ids {
                    if *ref_id != new_id {
                        let _ = link_tasks_in_tx(&tx, &new_id, RelationInputKind::Related, ref_id);
                    }
                }

                spawned_ids.push(new_id);
            }
        }

        // If `effective_parent` was `in_review` and a blocking mention subtask was spawned/reused,
        // close open review subtasks on `effective_parent` and move `effective_parent` back to `in_progress`.
        if any_blocking_spawned_or_reused && effective_parent.status == TaskStatus::InReview {
            let open_reviews = find_open_review_subtasks(&tx, &effective_parent.id)?;
            for rev_id in open_reviews {
                set_task_status_raw(&tx, &rev_id, TaskStatus::Completed, &now_str)?;
                completed_review_ids.push(rev_id);
            }
            set_task_status_raw(&tx, &effective_parent.id, TaskStatus::InProgress, &now_str)?;
        }

        // 2. Handle `@takenote:<user>` non-blocking mentions
        for mentioned_user in &tokens.takenote_mentions {
            if mentioned_user.eq_ignore_ascii_case(&author) {
                continue;
            }

            let title = format!("Take note from {}: {}", commented_task.id, commented_task.title);
            let description =
                format!("You were tagged by @{author} via @takenote in note #{note_id} on {}:\n\n> {body}", commented_task.id);
            let new_id = create_task_in_tx(
                &tx,
                CreateTaskInput {
                    title,
                    description: Some(description),
                    status: Some(TaskStatus::Todo),
                    origin_kind: Some(OriginKind::TakeNote),
                    origin_note_id: Some(note_id),
                    importance: Some(commented_task.importance),
                    urgent_override: Some(true),
                    tags: commented_task.tags.clone(),
                    project: Some(commented_task.project.clone()),
                    reporter: Some(author.clone()),
                    assignee: Some(mentioned_user.clone()),
                    evaluator: Some(mentioned_user.clone()),
                    ..Default::default()
                },
                &author,
                &mut Vec::new(),
            )?;

            // Crucial: `@takenote:<user>` links via `related` ONLY (never `blocks` or `sub_task_of`).
            link_tasks_in_tx(&tx, &new_id, RelationInputKind::Related, &commented_task.id)?;

            for ref_id in &existing_ref_ids {
                if *ref_id != new_id {
                    let _ = link_tasks_in_tx(&tx, &new_id, RelationInputKind::Related, ref_id);
                }
            }

            spawned_ids.push(new_id);
        }
    }

    tx.commit()?;

    let mut spawned_tasks = Vec::with_capacity(spawned_ids.len());
    for id in &spawned_ids {
        spawned_tasks.push(get_task(db, id)?);
    }

    let mut reused_tasks = Vec::with_capacity(reused_ids.len());
    for id in &reused_ids {
        reused_tasks.push(get_task(db, id)?);
    }

    let mut completed_review_tasks = Vec::with_capacity(completed_review_ids.len());
    for id in &completed_review_ids {
        completed_review_tasks.push(get_task(db, id)?);
    }

    Ok(AddTaskNoteOutcome { note, spawned_tasks, reused_tasks, completed_review_tasks, user_notices })
}

/// Lists all notes for `raw_task_id` in chronological order.
pub fn list_task_notes(db: &Database, raw_task_id: &str) -> Result<Vec<TaskNote>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT id, task_id, author, body, created_at
         FROM task_notes
         WHERE task_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;

    let rows = stmt.query_map([&task_id], |row| {
        let created_str: String = row.get(4)?;
        Ok(TaskNote {
            id: row.get(0)?,
            task_id: row.get(1)?,
            author: row.get(2)?,
            body: row.get(3)?,
            attachments: Vec::new(),
            created_at: parse_utc(&created_str),
        })
    })?;

    let mut notes = Vec::new();
    for r in rows {
        let mut note = r?;
        note.attachments = load_note_attachments(&db.conn, note.id)?;
        notes.push(note);
    }
    Ok(notes)
}

/// Deletes a task note by `note_id` and cleans up any managed clipboard attachments belonging to that note.
pub fn delete_task_note(db: &mut Database, note_id: i64) -> Result<()> {
    delete_task_note_with_attachments_dir(db, note_id, None)
}

pub fn delete_task_note_with_attachments_dir(db: &mut Database, note_id: i64, managed_dir_override: Option<&Path>) -> Result<()> {
    let note_attachments = load_note_attachments(&db.conn, note_id).unwrap_or_default();
    let affected = db
        .conn
        .execute("DELETE FROM task_notes WHERE id = ?1", [note_id])?;
    if affected == 0 {
        return Err(WsidError::Validation(format!("task note #{note_id} not found")));
    }
    cleanup_managed_files_after_delete(&db.conn, &note_attachments, managed_dir_override);
    Ok(())
}

/// One-step cleanup helper for overzealous mention/subtask blockers:
/// removes any `blocks` or `parent_of` (subtask) incoming/outgoing blocking links from `raw_task_id`
/// and marks the subtask `completed` if `dismiss` is true (or converts it into a non-blocking `related` task if `dismiss` is false).
pub fn unblock_or_dismiss_subtask(db: &mut Database, raw_task_id: &str, dismiss: bool) -> Result<Task> {
    let task_id = normalize_task_id(raw_task_id)?;
    let subtask = get_task(db, &task_id)?;
    let now_str = format_utc(Utc::now());

    let tx = db.conn.transaction()?;

    // Find any parent tasks that this subtask currently blocks
    let mut stmt = tx.prepare("SELECT target_id FROM task_relations WHERE source_id = ?1 AND relation_type = 'blocks'")?;
    let blocked_targets: Vec<String> = stmt
        .query_map([&subtask.id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    // Delete `subtask blocks target` and `target parent_of subtask` so it no longer blocks or defers parent
    tx.execute("DELETE FROM task_relations WHERE source_id = ?1 AND relation_type = 'blocks'", [&subtask.id])?;
    tx.execute("DELETE FROM task_relations WHERE target_id = ?1 AND relation_type = 'parent_of'", [&subtask.id])?;

    for target_id in &blocked_targets {
        let _ = link_tasks_in_tx(&tx, &subtask.id, RelationInputKind::Related, target_id);
    }

    if dismiss {
        set_task_status_raw(&tx, &subtask.id, TaskStatus::Completed, &now_str)?;
    }

    tx.commit()?;
    get_task(db, &subtask.id)
}

/// Transitions a task's status while enforcing role-based `in_review` handoffs and blocker checks.
pub fn transition_task_status(db: &mut Database, raw_task_id: &str, new_status: TaskStatus) -> Result<StatusTransitionOutcome> {
    let task = get_task(db, raw_task_id)?;
    let now_str = format_utc(Utc::now());

    let tx = db.conn.transaction()?;

    // Non-completion status transitions (e.g. backlog -> todo -> in_progress -> stuck)
    if new_status != TaskStatus::Completed {
        set_task_status_raw(&tx, &task.id, new_status, &now_str)?;
        tx.commit()?;
        return Ok(StatusTransitionOutcome {
            task: get_task(db, &task.id)?,
            spawned_review_task: None,
            auto_completed_parent_tasks: Vec::new(),
        });
    }

    // Attempting to complete `task`: verify it has no incomplete direct `blocks` predecessors or incomplete `sub_task_of` children
    let open_blockers = find_incomplete_blockers_or_subtasks(&tx, &task.id)?;
    if !open_blockers.is_empty() {
        return Err(WsidError::Validation(format!(
            "cannot complete task {} while blocked by incomplete task(s): {}",
            task.id,
            open_blockers.join(", ")
        )));
    }

    // Case 1: `assignee != evaluator` -> transition task to `in_review` and spawn a blocking review subtask for `evaluator`.
    if !task.assignee.eq_ignore_ascii_case(&task.evaluator) {
        set_task_status_raw(&tx, &task.id, TaskStatus::InReview, &now_str)?;

        let review_id = if let Some(existing_rev) = find_open_review_subtasks(&tx, &task.id)?.into_iter().next() {
            existing_rev
        } else {
            let title = format!("Review {}: {}", task.id, task.title);
            let description = format!(
                "Task {} ({}) was submitted for review by @{}. Complete this review task to approve and complete {}, or comment with @{} on {} to request changes.",
                task.id, task.title, task.assignee, task.id, task.assignee, task.id
            );
            let new_rev_id = create_task_in_tx(
                &tx,
                CreateTaskInput {
                    title,
                    description: Some(description),
                    status: Some(TaskStatus::Todo),
                    origin_kind: Some(OriginKind::Review),
                    due_date: task.due_date,
                    sgdb_date: task.sgdb_date,
                    importance: Some(task.importance),
                    urgent_override: Some(true),
                    tags: task.tags.clone(),
                    project: Some(task.project.clone()),
                    reporter: Some(task.assignee.clone()),
                    assignee: Some(task.evaluator.clone()),
                    evaluator: Some(task.evaluator.clone()),
                    ..Default::default()
                },
                &task.evaluator,
                &mut Vec::new(),
            )?;

            link_tasks_in_tx(&tx, &new_rev_id, RelationInputKind::Blocks, &task.id)?;
            link_tasks_in_tx(&tx, &new_rev_id, RelationInputKind::SubTaskOf, &task.id)?;
            new_rev_id
        };

        tx.commit()?;
        return Ok(StatusTransitionOutcome {
            task: get_task(db, &task.id)?,
            spawned_review_task: Some(get_task(db, &review_id)?),
            auto_completed_parent_tasks: Vec::new(),
        });
    }

    // Case 2: `assignee == evaluator` -> mark `task` completed directly.
    set_task_status_raw(&tx, &task.id, TaskStatus::Completed, &now_str)?;

    // If `task` was a `review` subtask (`origin_kind == 'review'`), check its parent task(s) in `in_review`.
    // Any parent task in `in_review` with no remaining incomplete blockers/subtasks automatically transitions to `completed`.
    let mut auto_completed_parent_ids = Vec::new();
    if task.origin_kind == OriginKind::Review {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT target_id FROM task_relations WHERE source_id = ?1 AND relation_type = 'blocks'
             UNION
             SELECT DISTINCT source_id FROM task_relations WHERE target_id = ?1 AND relation_type = 'parent_of'",
        )?;
        let candidate_parents: Vec<String> = stmt
            .query_map([&task.id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        for parent_id in candidate_parents {
            let parent = get_task_conn(&tx, &parent_id)?;
            if parent.status == TaskStatus::InReview {
                let remaining = find_incomplete_blockers_or_subtasks(&tx, &parent.id)?;
                if remaining.is_empty() {
                    set_task_status_raw(&tx, &parent.id, TaskStatus::Completed, &now_str)?;
                    auto_completed_parent_ids.push(parent.id);
                }
            }
        }
    }

    tx.commit()?;

    let mut auto_completed_parent_tasks = Vec::with_capacity(auto_completed_parent_ids.len());
    for pid in &auto_completed_parent_ids {
        auto_completed_parent_tasks.push(get_task(db, pid)?);
    }

    Ok(StatusTransitionOutcome { task: get_task(db, &task.id)?, spawned_review_task: None, auto_completed_parent_tasks })
}

fn set_task_status_raw(conn: &Connection, task_id: &str, status: TaskStatus, now_str: &str) -> Result<()> {
    let completed_at = if status.is_completed() { Some(now_str) } else { None };
    conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, completed_at = ?3 WHERE id = ?4",
        params![status.as_str(), now_str, completed_at, task_id],
    )?;
    Ok(())
}

fn find_open_mention_subtask(conn: &Connection, parent_id: &str, assignee: &str) -> Result<Option<String>> {
    let id_opt: Option<String> = conn
        .query_row(
            "SELECT t.id
             FROM tasks t
             INNER JOIN task_relations r ON r.source_id = t.id AND r.target_id = ?1 AND r.relation_type = 'blocks'
             WHERE t.origin_kind = 'mention'
               AND lower(t.assignee) = lower(?2)
               AND t.status != 'completed'
             ORDER BY t.created_at ASC, t.rowid ASC
             LIMIT 1",
            params![parent_id, assignee],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id_opt)
}

fn append_note_to_existing_subtask(
    conn: &Connection,
    subtask_id: &str,
    author: &str,
    note_id: i64,
    parent_id: &str,
    body: &str,
    now_str: &str,
) -> Result<()> {
    let current_desc: String = conn.query_row("SELECT description FROM tasks WHERE id = ?1", [subtask_id], |r| r.get(0))?;
    let updated_desc = format!("{current_desc}\n\n---\nFollow-up from @{author} in note #{note_id} on {parent_id}:\n\n> {body}");
    conn.execute(
        "UPDATE tasks SET description = ?1, urgent_override = 1, updated_at = ?2 WHERE id = ?3",
        params![updated_desc, now_str, subtask_id],
    )?;
    Ok(())
}

fn find_review_parent_id(conn: &Connection, review_task_id: &str) -> Result<Option<String>> {
    let parent_opt: Option<String> = conn
        .query_row(
            "SELECT target_id
             FROM task_relations
             WHERE source_id = ?1 AND relation_type = 'blocks'
             ORDER BY created_at ASC
             LIMIT 1",
            [review_task_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(parent_opt)
}

fn find_open_review_subtasks(conn: &Connection, parent_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.id
         FROM tasks t
         INNER JOIN task_relations r ON r.source_id = t.id AND r.target_id = ?1 AND r.relation_type = 'blocks'
         WHERE t.origin_kind = 'review' AND t.status != 'completed'
         ORDER BY t.created_at ASC, t.rowid ASC",
    )?;
    let ids = stmt
        .query_map([parent_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn find_incomplete_blockers_or_subtasks(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT t.id
         FROM tasks t
         WHERE t.status != 'completed'
           AND (
               EXISTS (
                   SELECT 1 FROM task_relations r
                   WHERE r.source_id = t.id AND r.target_id = ?1 AND r.relation_type = 'blocks'
               )
               OR EXISTS (
                   SELECT 1 FROM task_relations r
                   WHERE r.source_id = ?1 AND r.target_id = t.id AND r.relation_type = 'parent_of'
               )
           )
         ORDER BY t.id ASC",
    )?;
    let ids = stmt
        .query_map([task_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;
    use crate::relations::{get_direct_blockers, get_direct_subtasks, get_related_tasks};
    use crate::tasks::create_task;

    #[test]
    fn spawns_blocking_mention_and_non_blocking_takenote_with_cross_links() {
        let mut db = open_in_memory_db().unwrap();

        let parent = create_task(
            &mut db,
            CreateTaskInput {
                title: "Implement color palette".to_string(),
                reporter: Some("claude-author".to_string()),
                assignee: Some("bob".to_string()),
                evaluator: Some("human".to_string()),
                tags: vec!["ui".to_string()],
                ..Default::default()
            },
            "human",
        )
        .unwrap();

        let other_project_task =
            create_task(&mut db, CreateTaskInput { title: "Update design tokens".to_string(), ..Default::default() }, "human").unwrap();

        // Human comments mentioning @bob (current assignee), @human (self-mention, ignored),
        // @takenote:carol (non-blocking), and #<other_project_task.id>
        let comment = format!(
            "Hey @bob and @bob please tweak contrast for WCAG AA. Also @human will double check. \
             Hey @takenote:carol when working on #{}, take note of what we did here.",
            other_project_task.id.to_ascii_lowercase()
        );

        let outcome = add_task_note(&mut db, &parent.id, "human", &comment, true).unwrap();

        // Exactly 2 tasks spawned: 1 blocking mention for bob (deduplicated), 1 takenote for carol; @human self-mention suppressed
        assert_eq!(outcome.spawned_tasks.len(), 2);
        assert!(outcome.reused_tasks.is_empty());

        let bob_subtask = outcome
            .spawned_tasks
            .iter()
            .find(|t| t.assignee == "bob")
            .unwrap();
        assert_eq!(bob_subtask.origin_kind, OriginKind::Mention);
        assert!(bob_subtask.urgent_override);
        assert_eq!(bob_subtask.origin_note_id, Some(outcome.note.id));

        let carol_takenote = outcome
            .spawned_tasks
            .iter()
            .find(|t| t.assignee == "carol")
            .unwrap();
        assert_eq!(carol_takenote.origin_kind, OriginKind::TakeNote);
        assert!(carol_takenote.urgent_override);
        assert_eq!(carol_takenote.origin_note_id, Some(outcome.note.id));

        // Verify relations on `parent`:
        // ONLY `bob_subtask` blocks `parent` and is a subtask of `parent`.
        // `carol_takenote` does NOT block `parent` and is NOT a subtask of `parent`.
        let blockers = get_direct_blockers(&db, &parent.id).unwrap();
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].id, bob_subtask.id);

        let subtasks = get_direct_subtasks(&db, &parent.id).unwrap();
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].id, bob_subtask.id);

        // `carol_takenote` is linked via `related` to both `parent` and `other_project_task`
        let carol_related = get_related_tasks(&db, &carol_takenote.id).unwrap();
        let carol_related_ids: Vec<&str> = carol_related.iter().map(|t| t.id.as_str()).collect();
        assert!(carol_related_ids.contains(&parent.id.as_str()));
        assert!(carol_related_ids.contains(&other_project_task.id.as_str()));

        // Posting a second comment mentioning `@bob` while `bob_subtask` is still open reuses `bob_subtask`
        let second_outcome = add_task_note(&mut db, &parent.id, "human", "@bob also check focus outlines", true).unwrap();
        assert!(second_outcome.spawned_tasks.is_empty());
        assert_eq!(second_outcome.reused_tasks.len(), 1);
        assert_eq!(second_outcome.reused_tasks[0].id, bob_subtask.id);
        assert!(
            second_outcome.reused_tasks[0]
                .description
                .contains("focus outlines")
        );

        // Test `unblock_or_dismiss_subtask` cleanup helper
        let dismissed = unblock_or_dismiss_subtask(&mut db, &bob_subtask.id, true).unwrap();
        assert_eq!(dismissed.status, TaskStatus::Completed);
        assert!(get_direct_blockers(&db, &parent.id).unwrap().is_empty());

        // Verify invalid @mentions like `@invalid'hehe` or `@#coolboy` are ignored by `parse_note_tokens`
        let parsed_invalid = parse_note_tokens("Testing @invalid'hehe and @#coolboy vs @valid_user-1.");
        assert_eq!(parsed_invalid.blocking_mentions, vec!["valid_user-1".to_string()]);
        assert!(parsed_invalid.takenote_mentions.is_empty());
    }

    #[test]
    fn executes_full_human_ai_review_ping_pong_lifecycle() {
        let mut db = open_in_memory_db().unwrap();

        // 1. Create a task assigned to `bob` with evaluator `human`
        let task = create_task(
            &mut db,
            CreateTaskInput {
                title: "Build accessible settings modal".to_string(),
                reporter: Some("human".to_string()),
                assignee: Some("bob".to_string()),
                evaluator: Some("human".to_string()),
                importance: Some(5),
                tags: vec!["a11y".to_string()],
                ..Default::default()
            },
            "human",
        )
        .unwrap();

        // 2. Bob finishes initial implementation and marks `task` Completed -> moves to `in_review` + spawns review subtask for `human`
        let step1 = transition_task_status(&mut db, &task.id, TaskStatus::Completed).unwrap();
        assert_eq!(step1.task.status, TaskStatus::InReview);
        let review_round_1 = step1.spawned_review_task.expect("should spawn review task");
        assert_eq!(review_round_1.origin_kind, OriginKind::Review);
        assert_eq!(review_round_1.assignee, "human");
        assert_eq!(review_round_1.evaluator, "human");
        assert!(review_round_1.urgent_override);

        // 3. Human reviews and comments on `task` with `@bob tweaks needed...`
        //    -> `review_round_1` is auto-completed, `task` moves back to `in_progress`, and a blocking `@bob` subtask is created
        let feedback = add_task_note(&mut db, &task.id, "human", "@bob please add aria-modal and Escape key handling", true).unwrap();

        assert_eq!(feedback.completed_review_tasks.len(), 1);
        assert_eq!(feedback.completed_review_tasks[0].id, review_round_1.id);
        assert_eq!(feedback.spawned_tasks.len(), 1);
        let bob_fix_subtask = &feedback.spawned_tasks[0];
        assert_eq!(bob_fix_subtask.assignee, "bob");
        assert_eq!(bob_fix_subtask.origin_kind, OriginKind::Mention);

        let refreshed_parent = get_task(&db, &task.id).unwrap();
        assert_eq!(refreshed_parent.status, TaskStatus::InProgress);

        // 4. Bob cannot mark parent `task` Completed while `bob_fix_subtask` is still open
        assert!(transition_task_status(&mut db, &task.id, TaskStatus::Completed).is_err());

        // 5. Bob completes `bob_fix_subtask` (since bob == evaluator on mention subtasks, it completes directly)
        let fix_done = transition_task_status(&mut db, &bob_fix_subtask.id, TaskStatus::Completed).unwrap();
        assert_eq!(fix_done.task.status, TaskStatus::Completed);

        // 6. Bob resubmits `task` as Completed -> moves to `in_review` and spawns `review_round_2` for `human`
        let step2 = transition_task_status(&mut db, &task.id, TaskStatus::Completed).unwrap();
        assert_eq!(step2.task.status, TaskStatus::InReview);
        let review_round_2 = step2
            .spawned_review_task
            .expect("should spawn second review task");
        assert_ne!(review_round_2.id, review_round_1.id);

        // 7. Human approves by completing `review_round_2` -> `review_round_2` completes AND `task` auto-transitions from `in_review` to `completed`!
        let approval = transition_task_status(&mut db, &review_round_2.id, TaskStatus::Completed).unwrap();
        assert_eq!(approval.task.status, TaskStatus::Completed);
        assert_eq!(approval.auto_completed_parent_tasks.len(), 1);
        assert_eq!(approval.auto_completed_parent_tasks[0].id, task.id);
        assert_eq!(approval.auto_completed_parent_tasks[0].status, TaskStatus::Completed);

        let final_parent = get_task(&db, &task.id).unwrap();
        assert_eq!(final_parent.status, TaskStatus::Completed);
        assert!(final_parent.completed_at.is_some());
    }
}
