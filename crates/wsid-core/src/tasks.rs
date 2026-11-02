use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::{ShortIdGenerator, normalize_task_id};
use crate::models::{CreateTaskInput, OriginKind, Task, TaskStatus, UpdateTaskInput};

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

/// Normalizes a persona identifier (stripping an optional leading `@` and trimming whitespace).
pub fn normalize_persona(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed).trim();
    if stripped.is_empty() {
        return Err(WsidError::Validation("persona name cannot be empty".to_string()));
    }
    Ok(stripped.to_string())
}

pub fn validate_dates(sgdb_date: Option<NaiveDate>, due_date: Option<NaiveDate>) -> Result<()> {
    if let (Some(sgdb), Some(due)) = (sgdb_date, due_date)
        && sgdb > due
    {
        return Err(WsidError::Validation(format!("sgdb_date ({sgdb}) must be on or before due_date ({due})")));
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
    if !(-5..=5).contains(&manual_priority) {
        return Err(WsidError::Validation(format!("manual_priority must be between -5 and 5 (got {manual_priority})")));
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

pub fn create_task(db: &mut Database, input: CreateTaskInput, default_persona: &str) -> Result<Task> {
    let tx = db.conn.transaction()?;
    let id = create_task_in_tx(&tx, input, default_persona)?;
    tx.commit()?;

    get_task(db, &id)
}

pub(crate) fn create_task_in_tx(conn: &Connection, input: CreateTaskInput, default_persona: &str) -> Result<String> {
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
    let manual_priority = input.manual_priority.unwrap_or(0);

    validate_dates(input.sgdb_date, input.due_date)?;
    validate_difficulty(difficulty)?;
    validate_importance(importance)?;
    validate_manual_priority(manual_priority)?;
    validate_time_estimate(input.time_estimate_minutes)?;

    let fallback_persona = normalize_persona(default_persona)?;
    let reporter = match input.reporter.as_deref() {
        Some(r) if !r.trim().is_empty() => normalize_persona(r)?,
        _ => fallback_persona.clone(),
    };
    let assignee = match input.assignee.as_deref() {
        Some(a) if !a.trim().is_empty() => normalize_persona(a)?,
        _ => fallback_persona.clone(),
    };
    let evaluator = match input.evaluator.as_deref() {
        Some(e) if !e.trim().is_empty() => normalize_persona(e)?,
        _ => fallback_persona,
    };

    let tags = normalize_tags(&input.tags)?;

    let now_dt = Utc::now();
    let now_str = format_utc(now_dt);
    let completed_at_str = if status.is_completed() { Some(now_str.clone()) } else { None };
    let due_str = input.due_date.map(|d| d.format("%Y-%m-%d").to_string());
    let sgdb_str = input.sgdb_date.map(|d| d.format("%Y-%m-%d").to_string());

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
    let due_date = input.due_date.unwrap_or(current.due_date);
    let sgdb_date = input.sgdb_date.unwrap_or(current.sgdb_date);
    let time_estimate_minutes = input
        .time_estimate_minutes
        .unwrap_or(current.time_estimate_minutes);
    let difficulty = input.difficulty.unwrap_or(current.difficulty);
    let importance = input.importance.unwrap_or(current.importance);
    let urgent_override = input.urgent_override.unwrap_or(current.urgent_override);
    let manual_priority = input.manual_priority.unwrap_or(current.manual_priority);

    let reporter = match input.reporter {
        Some(r) => normalize_persona(&r)?,
        None => current.reporter,
    };
    let assignee = match input.assignee {
        Some(a) => normalize_persona(&a)?,
        None => current.assignee,
    };
    let evaluator = match input.evaluator {
        Some(e) => normalize_persona(&e)?,
        None => current.evaluator,
    };

    validate_dates(sgdb_date, due_date)?;
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

    let due_str = due_date.map(|d| d.format("%Y-%m-%d").to_string());
    let sgdb_str = sgdb_date.map(|d| d.format("%Y-%m-%d").to_string());
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

    tx.commit()?;
    get_task(db, &current.id)
}

pub fn delete_task(db: &mut Database, raw_id: &str) -> Result<()> {
    let id = normalize_task_id(raw_id)?;
    let affected = db.conn.execute("DELETE FROM tasks WHERE id = ?1", [&id])?;
    if affected == 0 {
        return Err(WsidError::TaskNotFound(id));
    }
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
    let due_date = due_str
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let sgdb_date = sgdb_str
        .as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

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
        manual_priority: i8::try_from(manual_priority_i64).unwrap_or(0),
        tags: Vec::new(),
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
    use chrono::NaiveDate;

    use super::*;
    use crate::db::open_in_memory_db;

    #[test]
    fn enforces_sgdb_and_due_date_combinations() {
        let mut db = open_in_memory_db().unwrap();
        let d_early = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let d_late = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();

        // Both None is allowed
        let t_none = create_task(&mut db, CreateTaskInput { title: "No dates".to_string(), ..Default::default() }, "david").unwrap();
        assert_eq!(t_none.due_date, None);
        assert_eq!(t_none.sgdb_date, None);

        // SGDB without due_date is allowed
        let t_sgdb_only = create_task(
            &mut db,
            CreateTaskInput { title: "SGDB only".to_string(), sgdb_date: Some(d_early), ..Default::default() },
            "david",
        )
        .unwrap();
        assert_eq!(t_sgdb_only.sgdb_date, Some(d_early));
        assert_eq!(t_sgdb_only.due_date, None);

        // SGDB <= due_date is allowed
        let t_both = create_task(
            &mut db,
            CreateTaskInput {
                title: "SGDB before due".to_string(),
                sgdb_date: Some(d_early),
                due_date: Some(d_late),
                ..Default::default()
            },
            "david",
        )
        .unwrap();
        assert_eq!(t_both.sgdb_date, Some(d_early));
        assert_eq!(t_both.due_date, Some(d_late));

        // SGDB > due_date is rejected on create
        let err = create_task(
            &mut db,
            CreateTaskInput { title: "Invalid SGDB".to_string(), sgdb_date: Some(d_late), due_date: Some(d_early), ..Default::default() },
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
    fn populates_default_persona_and_normalizes_tags() {
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
            create_task(&mut db, CreateTaskInput { title: "Invalid importance".to_string(), importance: Some(6), ..Default::default() }, "bob")
                .is_err()
        );

        assert!(
            create_task(&mut db, CreateTaskInput { title: "Invalid difficulty".to_string(), difficulty: Some(0), ..Default::default() }, "bob")
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
}
