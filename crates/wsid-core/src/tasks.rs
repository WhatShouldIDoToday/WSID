use std::collections::BTreeSet;
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::attachments::{cleanup_managed_files_after_delete, load_all_attachments_for_task, load_task_description_attachments};
use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::{ShortIdGenerator, normalize_task_id};
use crate::models::{
    CreateTaskInput, OriginKind, Task, TaskStatus, UpdateTaskInput, UserAffectedTask, UserDeletionOutcome, UserDeletionPreview,
};

/// Normalizes a single tag into a lowercase token matching `^[a-z0-9_-]+$`.
pub fn normalize_tag(raw: &str) -> Result<String> {
    let trimmed = raw
        .trim()
        .strip_prefix('#')
        .unwrap_or_else(|| raw.trim())
        .to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(WsidError::Validation("tag cannot be empty".to_string()));
    }
    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(WsidError::Validation(format!(
            "invalid tag '{raw}': tags must be single-word tokens containing only letters, digits, '-', or '_'"
        )));
    }
    Ok(trimmed)
}

/// Normalizes and deduplicates a list of tags into sorted lowercase tokens.
pub fn normalize_tags(raw_tags: &[String]) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for tag in raw_tags {
        normalized.insert(normalize_tag(tag)?);
    }
    Ok(normalized.into_iter().collect())
}

/// Normalizes a user identifier into a lowercase token matching `^[a-z0-9_-]+$`
/// (stripping an optional leading `@` and trimming whitespace).
pub fn normalize_user(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('@')
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase();
    if stripped.is_empty() {
        return Err(WsidError::Validation("user name cannot be empty".to_string()));
    }
    if !stripped
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(WsidError::Validation(format!("invalid user name '{raw}': user names may only contain letters, numbers, '_', or '-'")));
    }
    Ok(stripped)
}

pub(crate) fn register_user_conn(conn: &Connection, raw_user: &str) -> Result<String> {
    let norm = normalize_user(raw_user)?;
    let now_str = format_utc(Utc::now());
    conn.execute("INSERT OR IGNORE INTO users (username, created_at) VALUES (?1, ?2)", params![norm, now_str])?;
    Ok(norm)
}

/// Validates and permanently registers a user in the database's `users` table.
pub fn register_user(db: &mut Database, raw_user: &str) -> Result<String> {
    register_user_conn(&db.conn, raw_user)
}

/// Lists all known users in the database (`users`, `tasks`, `task_notes`, plus any `extra_users`).
pub fn list_known_users(db: &Database, extra_users: &[&str]) -> Result<Vec<String>> {
    let mut set = BTreeSet::new();
    for raw in extra_users {
        if let Ok(norm) = normalize_user(raw) {
            set.insert(norm);
        }
    }

    let mut stmt = db.conn.prepare(
        "SELECT username FROM users
         UNION
         SELECT reporter FROM tasks
         UNION
         SELECT assignee FROM tasks
         UNION
         SELECT evaluator FROM tasks
         UNION
         SELECT author FROM task_notes",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        if let Ok(raw) = row
            && let Ok(norm) = normalize_user(&raw)
        {
            set.insert(norm);
        }
    }
    Ok(set.into_iter().collect())
}

/// Previews all tasks that will be affected (reassigned or deleted) if `target_user` is deleted.
pub fn preview_user_deletion(db: &Database, target_user: &str, fallback_user: &str) -> Result<UserDeletionPreview> {
    let target = normalize_user(target_user)?;
    let fallback = normalize_user(fallback_user)?;
    if target == fallback {
        return Err(WsidError::Validation(format!("cannot delete the primary user '@{fallback}'")));
    }

    let mut stmt = db.conn.prepare(
        "SELECT id, title, status, reporter, assignee, evaluator
         FROM tasks
         WHERE LOWER(reporter) = ?1 OR LOWER(assignee) = ?1 OR LOWER(evaluator) = ?1
         ORDER BY created_at ASC, rowid ASC",
    )?;

    let rows = stmt.query_map([&target], |row| {
        let id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let status_str: String = row.get(2)?;
        let reporter: String = row.get(3)?;
        let assignee: String = row.get(4)?;
        let evaluator: String = row.get(5)?;
        let status = TaskStatus::from_str(&status_str).unwrap_or(TaskStatus::Todo);
        Ok((id, title, status, reporter, assignee, evaluator))
    })?;

    let mut affected_tasks = Vec::new();
    let mut authored_task_count = 0;
    let mut assigned_task_count = 0;
    let mut evaluator_task_count = 0;

    for row in rows {
        let (id, title, status, reporter, assignee, evaluator) = row?;
        let is_reporter = reporter.eq_ignore_ascii_case(&target);
        let is_assignee = assignee.eq_ignore_ascii_case(&target);
        let is_evaluator = evaluator.eq_ignore_ascii_case(&target);
        if is_reporter {
            authored_task_count += 1;
        }
        if is_assignee {
            assigned_task_count += 1;
        }
        if is_evaluator {
            evaluator_task_count += 1;
        }
        affected_tasks.push(UserAffectedTask { id, title, status, is_reporter, is_assignee, is_evaluator });
    }

    Ok(UserDeletionPreview {
        username: target,
        fallback_user: fallback,
        affected_tasks,
        authored_task_count,
        assigned_task_count,
        evaluator_task_count,
    })
}

/// Deletes `target_user` from the database, optionally deleting tasks authored (`reporter`) by `target_user`,
/// and reassigning any remaining tasks where `target_user` is `reporter`, `assignee`, or `evaluator` to `fallback_user`.
pub fn delete_user_from_db(
    db: &mut Database,
    target_user: &str,
    fallback_user: &str,
    delete_authored_tasks: bool,
    managed_dir_override: Option<&Path>,
) -> Result<UserDeletionOutcome> {
    let target = normalize_user(target_user)?;
    let fallback = normalize_user(fallback_user)?;
    if target == fallback {
        return Err(WsidError::Validation(format!("cannot delete the primary user '@{fallback}'")));
    }

    // Everything below runs in one transaction so a failure cannot leave a half-deleted user.
    let authored_ids: Vec<String> = if delete_authored_tasks {
        let mut stmt = db
            .conn
            .prepare("SELECT id FROM tasks WHERE LOWER(reporter) = ?1")?;
        let rows = stmt.query_map([&target], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let mut doomed_attachments = Vec::new();
    for id in &authored_ids {
        doomed_attachments.extend(load_all_attachments_for_task(&db.conn, id).unwrap_or_default());
    }

    let now_str = format_utc(Utc::now());
    let tx = db.conn.transaction()?;

    for id in &authored_ids {
        tx.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
    }
    let deleted_task_count = authored_ids.len();

    let reassigned_task_count: usize = {
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE LOWER(reporter) = ?1 OR LOWER(assignee) = ?1 OR LOWER(evaluator) = ?1",
            [&target],
            |row| row.get(0),
        )?;
        usize::try_from(count).unwrap_or(0)
    };

    tx.execute(
        "UPDATE tasks SET
            reporter = CASE WHEN LOWER(reporter) = ?1 THEN ?2 ELSE reporter END,
            assignee = CASE WHEN LOWER(assignee) = ?1 THEN ?2 ELSE assignee END,
            evaluator = CASE WHEN LOWER(evaluator) = ?1 THEN ?2 ELSE evaluator END,
            updated_at = ?3
         WHERE LOWER(reporter) = ?1 OR LOWER(assignee) = ?1 OR LOWER(evaluator) = ?1",
        params![target, fallback, now_str],
    )?;

    // Keep comment history but re-attribute it so the deleted user does not reappear in the known-users list.
    tx.execute("UPDATE task_notes SET author = ?2 WHERE LOWER(author) = ?1", params![target, fallback])?;

    tx.execute("DELETE FROM users WHERE LOWER(username) = ?1", [&target])?;
    register_user_conn(&tx, &fallback)?;

    tx.commit()?;
    cleanup_managed_files_after_delete(&db.conn, &doomed_attachments, managed_dir_override);

    Ok(UserDeletionOutcome { deleted_user: target, fallback_user: fallback, reassigned_task_count, deleted_task_count })
}

/// Parses and validates a deadline string (`YYYY-MM-DD` or ISO-8601 date-time),
/// returning the normalized string and its comparable local `NaiveDateTime`.
/// When only a date (`YYYY-MM-DD`) is provided, the time defaults to `20:00:00` (8:00 PM).
pub fn parse_deadline_datetime(field_name: &str, raw: &str) -> Result<(String, NaiveDateTime)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WsidError::Validation(format!("{field_name} cannot be empty")));
    }

    // Strip optional bracketed IANA timezone suffix (e.g. `2026-11-05T20:00:00-08:00[America/Los_Angeles]`)
    let without_bracket = match trimmed.find('[') {
        Some(idx) if trimmed.ends_with(']') => &trimmed[..idx],
        _ => trimmed,
    };
    // Normalize historical LMT offsets with seconds (e.g. `-04:56:02` -> `-04:56`) for RFC3339 compatibility
    let normalized_offset: String = if without_bracket.len() > 19 {
        let tail = &without_bracket[19..];
        if tail.len() == 9 && (tail.starts_with('+') || tail.starts_with('-')) && tail.as_bytes()[3] == b':' && tail.as_bytes()[6] == b':' {
            format!("{}{}", &without_bracket[..19], &tail[..6])
        } else {
            without_bracket.to_string()
        }
    } else {
        without_bracket.to_string()
    };

    if let Ok(d) = NaiveDate::parse_from_str(&normalized_offset, "%Y-%m-%d") {
        let default_time = NaiveTime::from_hms_opt(20, 0, 0).expect("valid 20:00:00 time");
        return Ok((normalized_offset, NaiveDateTime::new(d, default_time)));
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(&normalized_offset) {
        let local_naive = dt.with_timezone(&Local).naive_local();
        return Ok((normalized_offset, local_naive));
    }

    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(&normalized_offset, fmt) {
            let iso = ndt.format("%Y-%m-%dT%H:%M").to_string();
            return Ok((iso, ndt));
        }
    }

    Err(WsidError::Validation(format!(
        "invalid {field_name} '{trimmed}': expected YYYY-MM-DD or ISO-8601 date-time (e.g. 2026-11-05T20:00)"
    )))
}

/// Parses a human-readable duration string (e.g. `"1w 2d 3h 20m"`, `"1d 2h 20m"`, `"2h 30m"`, `"45m"`, or `"45"`)
/// into total minutes, rejecting duplicate time units (e.g. `"20m 10m"` or `"1h 2h"`).
pub fn parse_duration_minutes(raw: &str) -> Result<u32> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Err(WsidError::Validation("time estimate cannot be empty".to_string()));
    }

    if let Ok(mins) = s.parse::<u32>() {
        if mins == 0 {
            return Err(WsidError::Validation("time estimate must be greater than 0 minutes".to_string()));
        }
        return Ok(mins);
    }

    let mut total_minutes: u32 = 0;
    let mut seen_w = false;
    let mut seen_d = false;
    let mut seen_h = false;
    let mut seen_m = false;
    let mut matched_any = false;

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let num_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if num_start == i {
            return Err(WsidError::Validation(format!(
                "invalid time estimate '{raw}': expected format like '1w 2d 3h 20m', '1d 2h 20m', or '45m'"
            )));
        }
        let val: u32 = s[num_start..i]
            .parse()
            .map_err(|_| WsidError::Validation(format!("invalid number in time estimate '{raw}'")))?;

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if unit_start == i {
            return Err(WsidError::Validation(format!("missing time unit after '{}' in '{raw}' (use w, d, h, or m)", &s[num_start..i])));
        }

        let unit = &s[unit_start..i];
        let (canonical, multiplier) = match unit {
            "w" | "wk" | "wks" | "week" | "weeks" => ('w', 7 * 24 * 60u32),
            "d" | "day" | "days" => ('d', 24 * 60u32),
            "h" | "hr" | "hrs" | "hour" | "hours" => ('h', 60u32),
            "m" | "min" | "mins" | "minute" | "minutes" => ('m', 1u32),
            _ => {
                return Err(WsidError::Validation(format!("unknown time unit '{unit}' in '{raw}' (use w, d, h, or m)")));
            }
        };

        let seen_slot = match canonical {
            'w' => &mut seen_w,
            'd' => &mut seen_d,
            'h' => &mut seen_h,
            _ => &mut seen_m,
        };
        if *seen_slot {
            return Err(WsidError::Validation(format!(
                "duplicate time unit '{canonical}' in '{raw}' (specify each unit w, d, h, m at most once)"
            )));
        }
        *seen_slot = true;

        let added = val
            .checked_mul(multiplier)
            .ok_or_else(|| WsidError::Validation(format!("time estimate '{raw}' is too large")))?;
        total_minutes = total_minutes
            .checked_add(added)
            .ok_or_else(|| WsidError::Validation(format!("time estimate '{raw}' is too large")))?;
        matched_any = true;
    }

    if !matched_any || total_minutes == 0 {
        return Err(WsidError::Validation("time estimate must be greater than 0 minutes".to_string()));
    }

    Ok(total_minutes)
}

/// Formats a duration in minutes into a human-readable string in decreasing order of time span
/// (`w`, `d`, `h`, `m`, e.g. `"1w 2d 3h 20m"`).
pub fn format_duration_minutes(total_minutes: u32) -> String {
    if total_minutes == 0 {
        return "0m".to_string();
    }
    let weeks = total_minutes / (7 * 24 * 60);
    let rem_after_weeks = total_minutes % (7 * 24 * 60);
    let days = rem_after_weeks / (24 * 60);
    let rem_after_days = rem_after_weeks % (24 * 60);
    let hours = rem_after_days / 60;
    let mins = rem_after_days % 60;

    let mut parts = Vec::with_capacity(4);
    if weeks > 0 {
        parts.push(format!("{weeks}w"));
    }
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if mins > 0 {
        parts.push(format!("{mins}m"));
    }
    parts.join(" ")
}

/// Extracts the calendar `NaiveDate` from a stored deadline string (`YYYY-MM-DD` or ISO-8601 date-time).
pub fn parse_deadline_naive_date(raw: &str) -> Option<NaiveDate> {
    let trimmed = raw.trim();
    if trimmed.len() >= 10
        && let Ok(d) = NaiveDate::parse_from_str(&trimmed[..10], "%Y-%m-%d")
    {
        return Some(d);
    }
    parse_deadline_datetime("deadline", trimmed)
        .ok()
        .map(|(_, dt)| dt.date())
}

pub fn validate_dates(sgdb_date: Option<&str>, due_date: Option<&str>) -> Result<()> {
    let sgdb_parsed = match sgdb_date {
        Some(s) => Some(parse_deadline_datetime("sgdb_date", s)?),
        None => None,
    };
    let due_parsed = match due_date {
        Some(d) => Some(parse_deadline_datetime("due_date", d)?),
        None => None,
    };

    if let (Some((sgdb_str, sgdb_dt)), Some((due_str, due_dt))) = (sgdb_parsed, due_parsed)
        && sgdb_dt > due_dt
    {
        return Err(WsidError::Validation(format!("sgdb_date ({sgdb_str}) must be on or before due_date ({due_str})")));
    }
    Ok(())
}

/// Rejects deadlines that are already in the past (local time). Date-only values count as 20:00 that day.
pub fn validate_not_past(field_name: &str, normalized: &str) -> Result<()> {
    let (_, dt) = parse_deadline_datetime(field_name, normalized)?;
    if dt < Local::now().naive_local() {
        let label = if field_name == "sgdb_date" { "Should Get Done By (SGDB) date/time" } else { "Hard due date/time" };
        return Err(WsidError::Validation(format!("{label} cannot be in the past ({normalized})")));
    }
    Ok(())
}

pub fn validate_difficulty(difficulty: u8) -> Result<()> {
    if !(1..=5).contains(&difficulty) {
        return Err(WsidError::Validation(format!("difficulty must be between 1 and 5 (got {difficulty})")));
    }
    Ok(())
}

pub fn validate_importance(importance: u8) -> Result<()> {
    if !(1..=5).contains(&importance) {
        return Err(WsidError::Validation(format!("importance must be between 1 and 5 (got {importance})")));
    }
    Ok(())
}

pub fn validate_manual_priority(manual_priority: i8) -> Result<()> {
    if !(1..=5).contains(&manual_priority) {
        return Err(WsidError::Validation(format!("manual_priority must be between 1 and 5 (got {manual_priority})")));
    }
    Ok(())
}

pub fn validate_time_estimate(time_estimate_minutes: Option<i64>) -> Result<()> {
    if let Some(mins) = time_estimate_minutes
        && mins < 0
    {
        return Err(WsidError::Validation(format!("time_estimate_minutes cannot be negative (got {mins})")));
    }
    Ok(())
}

pub fn create_task(db: &mut Database, input: CreateTaskInput, default_user: &str) -> Result<Task> {
    let tx = db.conn.transaction()?;
    let id = create_task_in_tx(&tx, input, default_user)?;
    tx.commit()?;

    get_task(db, &id)
}

pub(crate) fn create_task_in_tx(conn: &Connection, input: CreateTaskInput, default_user: &str) -> Result<String> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(WsidError::Validation("task title cannot be empty".to_string()));
    }

    let description = input.description.unwrap_or_default();
    let status = input.status.unwrap_or(TaskStatus::Todo);
    let origin_kind = input.origin_kind.unwrap_or(OriginKind::Manual);
    let difficulty = input.difficulty.unwrap_or(3);
    let importance = input.importance.unwrap_or(3);
    let urgent_override = input.urgent_override.unwrap_or(false);
    let manual_priority = input.manual_priority.unwrap_or(3);

    let due_str = match input.due_date.as_deref() {
        Some(s) if !s.trim().is_empty() => Some(parse_deadline_datetime("due_date", s)?.0),
        _ => None,
    };
    let sgdb_str = match input.sgdb_date.as_deref() {
        Some(s) if !s.trim().is_empty() => Some(parse_deadline_datetime("sgdb_date", s)?.0),
        _ => None,
    };

    validate_dates(sgdb_str.as_deref(), due_str.as_deref())?;
    if !status.is_completed() {
        if let Some(s) = sgdb_str.as_deref() {
            validate_not_past("sgdb_date", s)?;
        }
        if let Some(d) = due_str.as_deref() {
            validate_not_past("due_date", d)?;
        }
    }
    validate_difficulty(difficulty)?;
    validate_importance(importance)?;
    validate_manual_priority(manual_priority)?;
    validate_time_estimate(input.time_estimate_minutes)?;

    let fallback_user = normalize_user(default_user)?;
    let reporter = match input.reporter.as_deref() {
        Some(r) if !r.trim().is_empty() => normalize_user(r)?,
        _ => fallback_user.clone(),
    };
    let assignee = match input.assignee.as_deref() {
        Some(a) if !a.trim().is_empty() => normalize_user(a)?,
        _ => fallback_user.clone(),
    };
    let evaluator = match input.evaluator.as_deref() {
        Some(e) if !e.trim().is_empty() => normalize_user(e)?,
        _ => fallback_user,
    };

    let tags = normalize_tags(&input.tags)?;

    let now_dt = Utc::now();
    let now_str = format_utc(now_dt);
    let completed_at_str = if status.is_completed() { Some(now_str.clone()) } else { None };

    let id = ShortIdGenerator::generate_unique_conn(conn)?;

    conn.execute(
        "INSERT INTO tasks (
            id, title, description, status, origin_kind, origin_note_id,
            due_date, sgdb_date, time_estimate_minutes, difficulty, importance,
            urgent_override, manual_priority, reporter, assignee, evaluator,
            created_at, updated_at, completed_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16,
            ?17, ?18, ?19
        )",
        params![
            id,
            title,
            description,
            status.as_str(),
            origin_kind.as_str(),
            input.origin_note_id,
            due_str,
            sgdb_str,
            input.time_estimate_minutes,
            i64::from(difficulty),
            i64::from(importance),
            i64::from(urgent_override),
            i64::from(manual_priority),
            reporter,
            assignee,
            evaluator,
            now_str,
            now_str,
            completed_at_str,
        ],
    )?;

    sync_task_tags(conn, &id, &tags)?;
    let _ = register_user_conn(conn, &reporter);
    let _ = register_user_conn(conn, &assignee);
    let _ = register_user_conn(conn, &evaluator);
    Ok(id)
}

pub fn get_task(db: &Database, raw_id: &str) -> Result<Task> {
    get_task_conn(&db.conn, raw_id)
}

pub(crate) fn get_task_conn(conn: &Connection, raw_id: &str) -> Result<Task> {
    let id = normalize_task_id(raw_id)?;

    let task_opt = conn
        .query_row(
            "SELECT
                id, title, description, status, origin_kind, origin_note_id,
                due_date, sgdb_date, time_estimate_minutes, difficulty, importance,
                urgent_override, manual_priority, reporter, assignee, evaluator,
                created_at, updated_at, completed_at
             FROM tasks
             WHERE id = ?1",
            [&id],
            map_task_row,
        )
        .optional()?;

    let mut task = task_opt.ok_or_else(|| WsidError::TaskNotFound(id.clone()))?;
    task.tags = load_task_tags(conn, &id)?;
    task.attachments = load_task_description_attachments(conn, &id)?;
    Ok(task)
}

pub fn list_tasks(db: &Database) -> Result<Vec<Task>> {
    list_tasks_conn(&db.conn)
}

pub(crate) fn list_tasks_conn(conn: &Connection) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(
        "SELECT
            id, title, description, status, origin_kind, origin_note_id,
            due_date, sgdb_date, time_estimate_minutes, difficulty, importance,
            urgent_override, manual_priority, reporter, assignee, evaluator,
            created_at, updated_at, completed_at
         FROM tasks
         ORDER BY created_at ASC, rowid ASC",
    )?;

    let rows = stmt.query_map([], map_task_row)?;
    let mut tasks = Vec::new();
    for row in rows {
        let mut task = row?;
        task.tags = load_task_tags(conn, &task.id)?;
        task.attachments = load_task_description_attachments(conn, &task.id)?;
        tasks.push(task);
    }
    Ok(tasks)
}

pub fn update_task(db: &mut Database, raw_id: &str, input: UpdateTaskInput) -> Result<Task> {
    let current = get_task(db, raw_id)?;

    let title = match input.title {
        Some(t) => {
            let trimmed = t.trim().to_string();
            if trimmed.is_empty() {
                return Err(WsidError::Validation("task title cannot be empty".to_string()));
            }
            trimmed
        }
        None => current.title,
    };

    let description = input.description.unwrap_or(current.description);
    let status = input.status.unwrap_or(current.status);
    let due_str = match input.due_date {
        Some(Some(ref s)) if !s.trim().is_empty() => Some(parse_deadline_datetime("due_date", s)?.0),
        Some(_) => None,
        None => current.due_date.clone(),
    };
    let sgdb_str = match input.sgdb_date {
        Some(Some(ref s)) if !s.trim().is_empty() => Some(parse_deadline_datetime("sgdb_date", s)?.0),
        Some(_) => None,
        None => current.sgdb_date.clone(),
    };
    let time_estimate_minutes = input
        .time_estimate_minutes
        .unwrap_or(current.time_estimate_minutes);
    let difficulty = input.difficulty.unwrap_or(current.difficulty);
    let importance = input.importance.unwrap_or(current.importance);
    let urgent_override = input.urgent_override.unwrap_or(current.urgent_override);
    let manual_priority = input.manual_priority.unwrap_or(current.manual_priority);

    let reporter = match input.reporter {
        Some(r) => normalize_user(&r)?,
        None => current.reporter,
    };
    let assignee = match input.assignee {
        Some(a) => normalize_user(&a)?,
        None => current.assignee,
    };
    let evaluator = match input.evaluator {
        Some(e) => normalize_user(&e)?,
        None => current.evaluator,
    };

    validate_dates(sgdb_str.as_deref(), due_str.as_deref())?;
    // Only newly set/changed deadlines are checked, so editing other fields of an overdue task still works.
    if !status.is_completed() {
        if let Some(s) = sgdb_str.as_deref()
            && sgdb_str != current.sgdb_date
        {
            validate_not_past("sgdb_date", s)?;
        }
        if let Some(d) = due_str.as_deref()
            && due_str != current.due_date
        {
            validate_not_past("due_date", d)?;
        }
    }
    validate_difficulty(difficulty)?;
    validate_importance(importance)?;
    validate_manual_priority(manual_priority)?;
    validate_time_estimate(time_estimate_minutes)?;

    let now_str = format_utc(Utc::now());
    let completed_at_str = if status.is_completed() {
        Some(
            current
                .completed_at
                .map(format_utc)
                .unwrap_or_else(|| now_str.clone()),
        )
    } else {
        None
    };

    let normalized_tags = match input.tags {
        Some(ref raw_tags) => Some(normalize_tags(raw_tags)?),
        None => None,
    };

    let tx = db.conn.transaction()?;
    tx.execute(
        "UPDATE tasks SET
            title = ?1,
            description = ?2,
            status = ?3,
            due_date = ?4,
            sgdb_date = ?5,
            time_estimate_minutes = ?6,
            difficulty = ?7,
            importance = ?8,
            urgent_override = ?9,
            manual_priority = ?10,
            reporter = ?11,
            assignee = ?12,
            evaluator = ?13,
            updated_at = ?14,
            completed_at = ?15
         WHERE id = ?16",
        params![
            title,
            description,
            status.as_str(),
            due_str,
            sgdb_str,
            time_estimate_minutes,
            i64::from(difficulty),
            i64::from(importance),
            i64::from(urgent_override),
            i64::from(manual_priority),
            reporter,
            assignee,
            evaluator,
            now_str,
            completed_at_str,
            current.id,
        ],
    )?;

    if let Some(tags) = normalized_tags {
        sync_task_tags(&tx, &current.id, &tags)?;
    }

    let _ = register_user_conn(&tx, &reporter);
    let _ = register_user_conn(&tx, &assignee);
    let _ = register_user_conn(&tx, &evaluator);

    tx.commit()?;
    get_task(db, &current.id)
}

pub fn delete_task(db: &mut Database, raw_id: &str) -> Result<()> {
    delete_task_with_attachments_dir(db, raw_id, None)
}

pub fn delete_task_with_attachments_dir(db: &mut Database, raw_id: &str, managed_dir_override: Option<&Path>) -> Result<()> {
    let id = normalize_task_id(raw_id)?;
    let existing_attachments = load_all_attachments_for_task(&db.conn, &id).unwrap_or_default();
    let affected = db.conn.execute("DELETE FROM tasks WHERE id = ?1", [&id])?;
    if affected == 0 {
        return Err(WsidError::TaskNotFound(id));
    }
    cleanup_managed_files_after_delete(&db.conn, &existing_attachments, managed_dir_override);
    Ok(())
}

fn sync_task_tags(conn: &Connection, task_id: &str, tags: &[String]) -> Result<()> {
    conn.execute("DELETE FROM task_tags WHERE task_id = ?1", [task_id])?;
    for tag in tags {
        conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
        let tag_id: i64 = conn.query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))?;
        conn.execute("INSERT OR IGNORE INTO task_tags (task_id, tag_id) VALUES (?1, ?2)", params![task_id, tag_id])?;
    }
    Ok(())
}

pub(crate) fn load_task_tags(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.name
         FROM tags t
         INNER JOIN task_tags tt ON tt.tag_id = t.id
         WHERE tt.task_id = ?1
         ORDER BY t.name ASC",
    )?;
    let rows = stmt.query_map([task_id], |row| row.get::<_, String>(0))?;
    let mut tags = Vec::new();
    for row in rows {
        tags.push(row?);
    }
    Ok(tags)
}

pub(crate) fn map_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status_str: String = row.get(3)?;
    let origin_str: String = row.get(4)?;
    let due_str: Option<String> = row.get(6)?;
    let sgdb_str: Option<String> = row.get(7)?;
    let difficulty_i64: i64 = row.get(9)?;
    let importance_i64: i64 = row.get(10)?;
    let urgent_i64: i64 = row.get(11)?;
    let manual_priority_i64: i64 = row.get(12)?;
    let created_str: String = row.get(16)?;
    let updated_str: String = row.get(17)?;
    let completed_str: Option<String> = row.get(18)?;

    let status = TaskStatus::from_str(&status_str).unwrap_or(TaskStatus::Todo);
    let origin_kind = OriginKind::from_str(&origin_str).unwrap_or(OriginKind::Manual);
    let due_date = due_str.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    });
    let sgdb_date = sgdb_str.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    });

    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        status,
        origin_kind,
        origin_note_id: row.get(5)?,
        due_date,
        sgdb_date,
        time_estimate_minutes: row.get(8)?,
        difficulty: u8::try_from(difficulty_i64).unwrap_or(3),
        importance: u8::try_from(importance_i64).unwrap_or(3),
        urgent_override: urgent_i64 != 0,
        manual_priority: i8::try_from(manual_priority_i64).unwrap_or(3).clamp(1, 5),
        tags: Vec::new(),
        attachments: Vec::new(),
        reporter: row.get(13)?,
        assignee: row.get(14)?,
        evaluator: row.get(15)?,
        created_at: parse_utc(&created_str),
        updated_at: parse_utc(&updated_str),
        completed_at: completed_str.as_deref().map(parse_utc),
    })
}

pub(crate) fn format_utc(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn parse_utc(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;

    #[test]
    fn enforces_sgdb_and_due_date_combinations() {
        let mut db = open_in_memory_db().unwrap();
        let d_early = "2099-11-05".to_string();
        let d_late = "2099-11-10".to_string();

        // Both None is allowed
        let t_none = create_task(&mut db, CreateTaskInput { title: "No dates".to_string(), ..Default::default() }, "david").unwrap();
        assert_eq!(t_none.due_date, None);
        assert_eq!(t_none.sgdb_date, None);
        assert_eq!(t_none.manual_priority, 3);

        // SGDB without due_date is allowed
        let t_sgdb_only = create_task(
            &mut db,
            CreateTaskInput { title: "SGDB only".to_string(), sgdb_date: Some(d_early.clone()), ..Default::default() },
            "david",
        )
        .unwrap();
        assert_eq!(t_sgdb_only.sgdb_date, Some(d_early.clone()));
        assert_eq!(t_sgdb_only.due_date, None);

        // SGDB <= due_date is allowed (including ISO date-time strings)
        let t_both = create_task(
            &mut db,
            CreateTaskInput {
                title: "SGDB before due".to_string(),
                sgdb_date: Some("2099-11-05T14:30".to_string()),
                due_date: Some("2099-11-05T20:00".to_string()),
                ..Default::default()
            },
            "david",
        )
        .unwrap();
        assert_eq!(t_both.sgdb_date.as_deref(), Some("2099-11-05T14:30"));
        assert_eq!(t_both.due_date.as_deref(), Some("2099-11-05T20:00"));

        // SGDB > due_date is rejected on create (including same-day 21:00 > default 20:00)
        let err = create_task(
            &mut db,
            CreateTaskInput {
                title: "Invalid SGDB".to_string(),
                sgdb_date: Some("2099-11-05T21:00".to_string()),
                due_date: Some("2099-11-05".to_string()),
                ..Default::default()
            },
            "david",
        )
        .unwrap_err();
        assert!(matches!(err, WsidError::Validation(_)));

        // SGDB > due_date is also rejected on update
        let err_update = update_task(
            &mut db,
            &t_both.id,
            UpdateTaskInput { sgdb_date: Some(Some(d_late)), due_date: Some(Some(d_early)), ..Default::default() },
        )
        .unwrap_err();
        assert!(matches!(err_update, WsidError::Validation(_)));
    }

    #[test]
    fn populates_default_user_and_normalizes_tags() {
        let mut db = open_in_memory_db().unwrap();

        let task = create_task(
            &mut db,
            CreateTaskInput {
                title: "Build Focus View".to_string(),
                tags: vec!["#Project-A".to_string(), "UI_Core".to_string(), "project-a".to_string()],
                ..Default::default()
            },
            "@alice",
        )
        .unwrap();

        assert_eq!(task.reporter, "alice");
        assert_eq!(task.assignee, "alice");
        assert_eq!(task.evaluator, "alice");
        assert_eq!(task.tags, vec!["project-a".to_string(), "ui_core".to_string()]);

        // Multi-word or invalid character tags are rejected
        let bad_tag_err = create_task(
            &mut db,
            CreateTaskInput { title: "Bad tag task".to_string(), tags: vec!["two words".to_string()], ..Default::default() },
            "alice",
        )
        .unwrap_err();
        assert!(matches!(bad_tag_err, WsidError::Validation(_)));
    }

    #[test]
    fn enforces_numeric_bounds_and_supports_crud_lifecycle() {
        let mut db = open_in_memory_db().unwrap();

        assert!(
            create_task(
                &mut db,
                CreateTaskInput { title: "Invalid importance".to_string(), importance: Some(6), ..Default::default() },
                "bob"
            )
            .is_err()
        );

        assert!(
            create_task(
                &mut db,
                CreateTaskInput { title: "Invalid difficulty".to_string(), difficulty: Some(0), ..Default::default() },
                "bob"
            )
            .is_err()
        );

        assert!(
            create_task(
                &mut db,
                CreateTaskInput { title: "Negative estimate".to_string(), time_estimate_minutes: Some(-15), ..Default::default() },
                "bob"
            )
            .is_err()
        );

        let created = create_task(
            &mut db,
            CreateTaskInput {
                title: "Initial title".to_string(),
                importance: Some(4),
                difficulty: Some(2),
                manual_priority: Some(3),
                time_estimate_minutes: Some(45),
                tags: vec!["backend".to_string()],
                ..Default::default()
            },
            "bob",
        )
        .unwrap();

        let updated = update_task(
            &mut db,
            &created.id,
            UpdateTaskInput {
                title: Some("Updated title".to_string()),
                status: Some(TaskStatus::Completed),
                tags: Some(vec!["rust".to_string(), "sqlite".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(updated.title, "Updated title");
        assert_eq!(updated.status, TaskStatus::Completed);
        assert!(updated.completed_at.is_some());
        assert_eq!(updated.tags, vec!["rust".to_string(), "sqlite".to_string()]);

        let all = list_tasks(&db).unwrap();
        assert_eq!(all.len(), 1);

        delete_task(&mut db, &created.id).unwrap();
        assert!(matches!(get_task(&db, &created.id), Err(WsidError::TaskNotFound(_))));
    }

    #[test]
    fn parses_and_formats_durations_and_rejects_duplicate_units() {
        assert_eq!(parse_duration_minutes("1d 2h 20m").unwrap(), 1440 + 120 + 20);
        assert_eq!(parse_duration_minutes("20m 2h 1d").unwrap(), 1440 + 120 + 20);
        assert_eq!(parse_duration_minutes("1w 2d 3h 20m").unwrap(), 10080 + 2880 + 180 + 20);
        assert_eq!(parse_duration_minutes("45").unwrap(), 45);

        // Reject duplicate units
        assert!(parse_duration_minutes("20m 10m").is_err());
        assert!(parse_duration_minutes("1h 2h").is_err());
        assert!(parse_duration_minutes("1d 3d").is_err());
        assert!(parse_duration_minutes("1w 2w").is_err());

        // Format back in decreasing order (w, d, h, m)
        assert_eq!(format_duration_minutes(1440 + 120 + 20), "1d 2h 20m");
        assert_eq!(format_duration_minutes(10080 + 2880 + 180 + 20), "1w 2d 3h 20m");
        assert_eq!(format_duration_minutes(140), "2h 20m");
        assert_eq!(format_duration_minutes(45), "45m");
    }

    #[test]
    fn enforces_strict_usernames_and_manages_user_deletion_reassignment_and_cleanup() {
        let mut db = open_in_memory_db().unwrap();

        // Invalid usernames with quotes, hashes, or spaces are rejected
        assert!(normalize_user("@invalid'hehe").is_err());
        assert!(normalize_user("@#coolboy").is_err());
        assert!(normalize_user("two words").is_err());
        assert_eq!(normalize_user("@Cool_User-1").unwrap(), "cool_user-1");

        // Registering a user persists even without any tasks
        register_user(&mut db, "davidb").unwrap();
        register_user(&mut db, "temp_user").unwrap();
        let known = list_known_users(&db, &[]).unwrap();
        assert!(known.contains(&"davidb".to_string()));
        assert!(known.contains(&"temp_user".to_string()));

        // Create tasks involving `foobar` as reporter, assignee, and evaluator
        let t_authored = create_task(
            &mut db,
            CreateTaskInput {
                title: "Task authored by foobar".to_string(),
                reporter: Some("foobar".to_string()),
                assignee: Some("davidb".to_string()),
                evaluator: Some("davidb".to_string()),
                ..Default::default()
            },
            "davidb",
        )
        .unwrap();

        let t_assigned = create_task(
            &mut db,
            CreateTaskInput {
                title: "Task assigned to foobar".to_string(),
                reporter: Some("davidb".to_string()),
                assignee: Some("foobar".to_string()),
                evaluator: Some("foobar".to_string()),
                ..Default::default()
            },
            "davidb",
        )
        .unwrap();

        // A comment by `foobar` must be re-attributed (not left behind) when the user is deleted
        crate::notes::add_task_note(&mut db, &t_assigned.id, "foobar", "hello there", false).unwrap();

        let preview = preview_user_deletion(&db, "foobar", "davidb").unwrap();
        assert_eq!(preview.username, "foobar");
        assert_eq!(preview.fallback_user, "davidb");
        assert_eq!(preview.affected_tasks.len(), 2);
        assert_eq!(preview.authored_task_count, 1);
        assert_eq!(preview.assigned_task_count, 1);
        assert_eq!(preview.evaluator_task_count, 1);

        // Cannot delete primary user
        assert!(preview_user_deletion(&db, "davidb", "davidb").is_err());

        // Delete `foobar` with `delete_authored_tasks = true`:
        // `t_authored` is deleted, `t_assigned` is reassigned to `davidb`
        let outcome = delete_user_from_db(&mut db, "foobar", "davidb", true, None).unwrap();
        assert_eq!(outcome.deleted_user, "foobar");
        assert_eq!(outcome.deleted_task_count, 1);
        assert_eq!(outcome.reassigned_task_count, 1);
        assert!(
            !list_known_users(&db, &[])
                .unwrap()
                .contains(&"foobar".to_string())
        );

        assert!(get_task(&db, &t_authored.id).is_err());
        let reassigned = get_task(&db, &t_assigned.id).unwrap();
        assert_eq!(reassigned.assignee, "davidb");
        assert_eq!(reassigned.evaluator, "davidb");
        assert!(
            !list_known_users(&db, &[])
                .unwrap()
                .contains(&"foobar".to_string())
        );
    }

    #[test]
    fn rejects_past_deadlines_only_when_newly_set() {
        let mut db = open_in_memory_db().unwrap();
        let past = CreateTaskInput { title: "Past".to_string(), due_date: Some("2000-01-01".to_string()), ..Default::default() };
        assert!(create_task(&mut db, past.clone(), "alice").is_err());
        // Already-completed tasks may carry historical deadlines.
        let done = CreateTaskInput { status: Some(TaskStatus::Completed), ..past };
        let task = create_task(&mut db, done, "alice").unwrap();

        // Unrelated edits and unchanged deadlines still work; changing to another past date does not.
        let kept = update_task(&mut db, &task.id, UpdateTaskInput { title: Some("Renamed".to_string()), ..Default::default() }).unwrap();
        assert_eq!(kept.due_date.as_deref(), Some("2000-01-01"));
        let reopened = update_task(
            &mut db,
            &task.id,
            UpdateTaskInput { status: Some(TaskStatus::Todo), due_date: Some(Some("2000-01-02".to_string())), ..Default::default() },
        );
        assert!(reopened.is_err());
    }

    /// Shared with the frontend (`ui/src/lib/validation.test.ts`) so both validators agree.
    #[test]
    fn matches_shared_validation_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../../test-fixtures/validation.json")).unwrap();

        for c in v["usernames"].as_array().unwrap() {
            let input = c["input"].as_str().unwrap();
            assert_eq!(normalize_user(input).ok().as_deref(), c["normalized"].as_str(), "username {input:?}");
        }
        for c in v["tags"].as_array().unwrap() {
            let input = c["input"].as_str().unwrap();
            assert_eq!(normalize_tag(input).ok().as_deref(), c["normalized"].as_str(), "tag {input:?}");
        }
        for c in v["durations"].as_array().unwrap() {
            let input = c["input"].as_str().unwrap();
            let expected = c["minutes"].as_u64().map(|m| m as u32);
            assert_eq!(parse_duration_minutes(input).ok(), expected, "duration {input:?}");
        }
        for c in v["deadline_order"].as_array().unwrap() {
            let (sgdb, due) = (c["sgdb"].as_str(), c["due"].as_str());
            assert_eq!(validate_dates(sgdb, due).is_ok(), c["valid"].as_bool().unwrap(), "order {sgdb:?} {due:?}");
        }
        for c in v["past_deadlines"].as_array().unwrap() {
            let value = c["value"].as_str().unwrap();
            assert_eq!(validate_not_past("due_date", value).is_err(), c["past"].as_bool().unwrap(), "past {value:?}");
        }
    }
}
