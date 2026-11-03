//! Shared "new task" validation and atomic creation used by the GUI, TUI, CLI, and MCP surfaces.
//!
//! A [`TaskDraft`] carries raw (possibly text) user input. [`validate_task_draft`] reports per-field
//! [`FieldIssue`]s (including *relation target doesn't exist* and *new user will be created*), and
//! [`create_task_from_draft`] creates the task, its relations, and its attachments in a single transaction.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::attachments::{attach_existing_file, attach_existing_file_in_tx};
use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::normalize_task_id;
use crate::models::{AttachmentInput, CreateTaskInput, RelationInputKind, Task, TaskStatus, UpdateTaskInput};
use crate::relations::{link_tasks, link_tasks_in_tx};
use crate::search::{SearchQuery, search_tasks};
use crate::tasks::{
    create_task_in_tx, get_task, get_task_conn, list_known_users, normalize_project, normalize_tags, normalize_user,
    parse_deadline_datetime, parse_duration_minutes, update_task, validate_dates, validate_not_past, validate_time_estimate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskField {
    Title,
    Description,
    DueDate,
    SgdbDate,
    TimeEstimate,
    Difficulty,
    Importance,
    ManualPriority,
    UrgentOverride,
    Project,
    Tags,
    Relations,
    Attachments,
    Reporter,
    Assignee,
    Evaluator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueLevel {
    Error,
    Info,
}

/// One validation message, optionally pointing at a specific list item (relation or attachment) via `index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldIssue {
    pub field: TaskField,
    pub level: IssueLevel,
    pub message: String,
    pub index: Option<usize>,
}

impl FieldIssue {
    fn error(field: TaskField, message: impl Into<String>) -> Self {
        Self { field, level: IssueLevel::Error, message: message.into(), index: None }
    }

    fn info(field: TaskField, message: impl Into<String>) -> Self {
        Self { field, level: IssueLevel::Info, message: message.into(), index: None }
    }

    fn at(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    pub fn is_error(&self) -> bool {
        self.level == IssueLevel::Error
    }
}

/// Joins the error-level issues into a single readable message.
pub fn summarize_issues(issues: &[FieldIssue]) -> String {
    issues
        .iter()
        .filter(|i| i.is_error())
        .map(|i| i.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

/// A scalar that may arrive as a JSON number, boolean, or raw text (the TUI passes text; GUI/MCP pass numbers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Int(i64),
    Bool(bool),
    Text(String),
}

impl Scalar {
    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Bool(_) => None,
            Self::Text(s) => s.trim().parse().ok(),
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Int(i) => Some(*i != 0),
            Self::Text(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "y" | "1" | "on" => Some(true),
                "false" | "no" | "n" | "0" | "off" => Some(false),
                _ => None,
            },
        }
    }

    fn is_blank(&self) -> bool {
        matches!(self, Self::Text(s) if s.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationDraft {
    /// Relation keyword (`blocks`, `blocked_by`, `parent_of`, `sub_task_of`, `parallel`, `related`).
    pub relation: String,
    /// Target task short ID.
    pub target: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskDraft {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub due_date: Option<String>,
    pub sgdb_date: Option<String>,
    /// Minutes as a number, or text such as `1h30m`.
    pub time_estimate: Option<Scalar>,
    pub difficulty: Option<Scalar>,
    pub importance: Option<Scalar>,
    pub manual_priority: Option<Scalar>,
    pub urgent_override: Option<Scalar>,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub relations: Vec<RelationDraft>,
    pub attachments: Vec<AttachmentInput>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
}

struct Resolved {
    input: CreateTaskInput,
    relations: Vec<(RelationInputKind, String)>,
    attachments: Vec<AttachmentInput>,
}

fn blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn rating(field: TaskField, label: &str, value: &Option<Scalar>, issues: &mut Vec<FieldIssue>) -> Option<i64> {
    let v = value.as_ref().filter(|v| !v.is_blank())?;
    match v.as_i64() {
        Some(n) if (1..=5).contains(&n) => Some(n),
        _ => {
            issues.push(FieldIssue::error(field, format!("{label} must be an integer from 1 to 5")));
            None
        }
    }
}

/// Resolves a draft into a typed [`CreateTaskInput`] plus list items, collecting every issue found.
fn resolve(conn: &Connection, draft: &TaskDraft, default_user: &str) -> (Option<Resolved>, Vec<FieldIssue>) {
    let mut issues: Vec<FieldIssue> = Vec::new();

    if draft.title.trim().is_empty() {
        issues.push(FieldIssue::error(TaskField::Title, "Title is required"));
    }

    let status = draft.status.unwrap_or(TaskStatus::Todo);
    let mut date = |field: TaskField, name: &str, raw: &Option<String>| -> Option<String> {
        let value = blank(raw)?;
        match parse_deadline_datetime(name, value) {
            Ok((normalized, _)) => {
                if !status.is_completed()
                    && let Err(e) = validate_not_past(name, &normalized)
                {
                    issues.push(FieldIssue::error(field, strip_prefix(&e)));
                }
                Some(normalized)
            }
            Err(e) => {
                issues.push(FieldIssue::error(field, strip_prefix(&e)));
                None
            }
        }
    };
    let due_date = date(TaskField::DueDate, "due_date", &draft.due_date);
    let sgdb_date = date(TaskField::SgdbDate, "sgdb_date", &draft.sgdb_date);
    if let (Some(s), Some(d)) = (&sgdb_date, &due_date)
        && let Err(e) = validate_dates(Some(s), Some(d))
    {
        issues.push(FieldIssue::error(TaskField::SgdbDate, strip_prefix(&e)));
    }

    let time_estimate_minutes = match draft.time_estimate.as_ref().filter(|v| !v.is_blank()) {
        None => None,
        Some(Scalar::Text(s)) => match parse_duration_minutes(s) {
            Ok(m) => Some(i64::from(m)),
            Err(e) => {
                issues.push(FieldIssue::error(TaskField::TimeEstimate, strip_prefix(&e)));
                None
            }
        },
        Some(other) => match other.as_i64() {
            Some(m) if validate_time_estimate(Some(m)).is_ok() => Some(m),
            _ => {
                issues.push(FieldIssue::error(TaskField::TimeEstimate, "Time estimate must be a non-negative number of minutes"));
                None
            }
        },
    };

    let difficulty = rating(TaskField::Difficulty, "Difficulty", &draft.difficulty, &mut issues);
    let importance = rating(TaskField::Importance, "Importance", &draft.importance, &mut issues);
    let manual_priority = rating(TaskField::ManualPriority, "Manual priority", &draft.manual_priority, &mut issues);
    let urgent_override = match draft.urgent_override.as_ref().filter(|v| !v.is_blank()) {
        None => None,
        Some(v) => match v.as_bool() {
            Some(b) => Some(b),
            None => {
                issues.push(FieldIssue::error(TaskField::UrgentOverride, "Urgent override must be true or false"));
                None
            }
        },
    };

    let tags = match normalize_tags(&draft.tags) {
        Ok(_) => draft.tags.clone(),
        Err(e) => {
            issues.push(FieldIssue::error(TaskField::Tags, strip_prefix(&e)));
            Vec::new()
        }
    };

    let mut relations: Vec<(RelationInputKind, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (i, rel) in draft.relations.iter().enumerate() {
        let kind = match RelationInputKind::from_str(&rel.relation) {
            Ok(k) => k,
            Err(_) => {
                issues.push(
                    FieldIssue::error(
                        TaskField::Relations,
                        format!("Unknown relation '{}' (use {})", rel.relation.trim(), RelationInputKind::NAMES.join(", ")),
                    )
                    .at(i),
                );
                continue;
            }
        };
        let target = match normalize_task_id(&rel.target) {
            Ok(t) => t,
            Err(_) => {
                issues.push(
                    FieldIssue::error(TaskField::Relations, format!("'{}' is not a valid task ID (e.g. AB1234)", rel.target.trim())).at(i),
                );
                continue;
            }
        };
        if get_task_conn(conn, &target).is_err() {
            issues.push(FieldIssue::error(TaskField::Relations, format!("No task {target} exists")).at(i));
            continue;
        }
        if !seen.insert((rel.relation.trim().to_ascii_lowercase(), target.clone())) {
            issues.push(FieldIssue::error(TaskField::Relations, format!("{} {target} is listed twice", rel.relation.trim())).at(i));
            continue;
        }
        relations.push((kind, target));
    }

    for (i, att) in draft.attachments.iter().enumerate() {
        let path = att.path.trim();
        if path.is_empty() {
            issues.push(FieldIssue::error(TaskField::Attachments, "Attachment path cannot be empty").at(i));
        } else if !Path::new(path).is_file() {
            issues.push(FieldIssue::error(TaskField::Attachments, format!("File not found: {path}")).at(i));
        }
    }

    let known = list_known_users_conn(conn);
    let mut user = |field: TaskField, raw: &Option<String>| -> Option<String> {
        let value = blank(raw)?;
        match normalize_user(value) {
            Ok(u) => {
                if !u.eq_ignore_ascii_case(default_user) && !known.contains(&u) {
                    issues.push(FieldIssue::info(field, format!("@{u} is new and will be created")));
                }
                Some(value.to_string())
            }
            Err(e) => {
                issues.push(FieldIssue::error(field, strip_prefix(&e)));
                None
            }
        }
    };
    let reporter = user(TaskField::Reporter, &draft.reporter);
    let assignee = user(TaskField::Assignee, &draft.assignee);
    let evaluator = user(TaskField::Evaluator, &draft.evaluator);

    let project = match blank(&draft.project) {
        Some(raw) => match normalize_project(raw) {
            Ok(p) => Some(p),
            Err(e) => {
                issues.push(FieldIssue::error(TaskField::Project, strip_prefix(&e)));
                None
            }
        },
        None => None,
    };

    if issues.iter().any(FieldIssue::is_error) {
        return (None, issues);
    }

    let input = CreateTaskInput {
        title: draft.title.trim().to_string(),
        description: blank(&draft.description).map(ToString::to_string),
        status: draft.status,
        origin_kind: None,
        origin_note_id: None,
        due_date,
        sgdb_date,
        time_estimate_minutes,
        difficulty: difficulty.map(|n| n as u8),
        importance: importance.map(|n| n as u8),
        urgent_override,
        manual_priority: manual_priority.map(|n| n as i8),
        tags,
        project,
        reporter,
        assignee,
        evaluator,
    };
    let attachments = draft
        .attachments
        .iter()
        .map(|a| AttachmentInput { path: a.path.trim().to_string(), description: a.description.clone() })
        .collect();
    (Some(Resolved { input, relations, attachments }), issues)
}

fn list_known_users_conn(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "SELECT username FROM users UNION SELECT reporter FROM tasks UNION SELECT assignee FROM tasks \
         UNION SELECT evaluator FROM tasks UNION SELECT author FROM task_notes",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(std::result::Result::ok).collect())
        .unwrap_or_default()
}

fn strip_prefix(err: &WsidError) -> String {
    let msg = err.to_string();
    msg.strip_prefix("validation error: ")
        .unwrap_or(&msg)
        .to_string()
}

/// Validates a draft against the database without creating anything.
pub fn validate_task_draft(db: &Database, draft: &TaskDraft, default_user: &str) -> Vec<FieldIssue> {
    resolve(&db.conn, draft, default_user).1
}

/// Validates and creates the task, its relations, and its attachments in one transaction.
/// On any problem nothing is written and [`WsidError::DraftRejected`] carries the issues.
pub fn create_task_from_draft(db: &mut Database, draft: &TaskDraft, default_user: &str) -> Result<Task> {
    let (resolved, issues) = resolve(&db.conn, draft, default_user);
    let Some(resolved) = resolved else {
        return Err(WsidError::DraftRejected(issues));
    };

    let tx = db.conn.transaction()?;
    let id = match create_task_in_tx(&tx, resolved.input, default_user) {
        Ok(id) => id,
        Err(e) => return Err(WsidError::DraftRejected(vec![FieldIssue::error(TaskField::Title, strip_prefix(&e))])),
    };
    for (i, (kind, target)) in resolved.relations.iter().enumerate() {
        if let Err(e) = link_tasks_in_tx(&tx, &id, *kind, target) {
            return Err(WsidError::DraftRejected(vec![FieldIssue::error(TaskField::Relations, strip_prefix(&e)).at(i)]));
        }
    }
    for (i, att) in resolved.attachments.iter().enumerate() {
        if let Err(e) = attach_existing_file_in_tx(&tx, &id, None, Path::new(&att.path), att.description.as_deref(), None) {
            return Err(WsidError::DraftRejected(vec![FieldIssue::error(TaskField::Attachments, strip_prefix(&e)).at(i)]));
        }
    }
    tx.commit()?;
    get_task(db, &id)
}

/// Validates a draft and applies it to an existing task: scalar fields, tags, new relations, and
/// newly added attachments. Existing attachments are kept untouched (they are managed from the
/// task detail view), so only paths not already attached are added.
pub fn update_task_from_draft(db: &mut Database, raw_id: &str, draft: &TaskDraft, default_user: &str) -> Result<Task> {
    let (resolved, issues) = resolve(&db.conn, draft, default_user);
    let Some(resolved) = resolved else {
        return Err(WsidError::DraftRejected(issues));
    };

    let current = get_task(db, raw_id)?;
    let existing_paths: HashSet<String> = current
        .attachments
        .iter()
        .map(|a| a.file_path.clone())
        .collect();
    let input = UpdateTaskInput {
        title: Some(resolved.input.title),
        description: Some(resolved.input.description.unwrap_or_default()),
        status: resolved.input.status,
        due_date: Some(resolved.input.due_date),
        sgdb_date: Some(resolved.input.sgdb_date),
        time_estimate_minutes: Some(resolved.input.time_estimate_minutes),
        difficulty: resolved.input.difficulty,
        importance: resolved.input.importance,
        urgent_override: resolved.input.urgent_override,
        manual_priority: resolved.input.manual_priority,
        tags: Some(resolved.input.tags),
        project: resolved.input.project,
        reporter: resolved.input.reporter,
        assignee: resolved.input.assignee,
        evaluator: resolved.input.evaluator,
    };
    let id = update_task(db, &current.id, input)?.id;

    for (i, (kind, target)) in resolved.relations.iter().enumerate() {
        if let Err(e) = link_tasks(db, &id, *kind, target) {
            return Err(WsidError::DraftRejected(vec![FieldIssue::error(TaskField::Relations, strip_prefix(&e)).at(i)]));
        }
    }
    for (i, att) in resolved.attachments.iter().enumerate() {
        if existing_paths.contains(&att.path) {
            continue;
        }
        if let Err(e) = attach_existing_file(db, &id, None, Path::new(&att.path), att.description.as_deref()) {
            return Err(WsidError::DraftRejected(vec![FieldIssue::error(TaskField::Attachments, strip_prefix(&e)).at(i)]));
        }
    }
    get_task(db, &id)
}

/// Tasks matching `query` (ID or title/fuzzy) for relation-target typeahead.
pub fn search_relation_targets(db: &Database, query: &str, limit: usize) -> Result<Vec<Task>> {
    let hits = search_tasks(db, &SearchQuery { query: query.to_string(), limit: Some(limit), ..Default::default() })?;
    Ok(hits.into_iter().map(|h| h.task).collect())
}

/// Known users whose handle starts with (or contains) `prefix`, for user-field typeahead.
pub fn suggest_users(db: &Database, prefix: &str, extra: &[&str]) -> Result<Vec<String>> {
    let needle = prefix.trim().trim_start_matches('@').to_ascii_lowercase();
    let mut users = list_known_users(db, extra)?;
    users.retain(|u| u.contains(&needle));
    users.sort_by_key(|u| !u.starts_with(&needle));
    Ok(users)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;
    use crate::tasks::{create_task, list_tasks};

    fn draft(title: &str) -> TaskDraft {
        TaskDraft { title: title.to_string(), ..Default::default() }
    }

    fn seed(db: &mut Database, title: &str) -> Task {
        create_task(db, CreateTaskInput { title: title.to_string(), ..Default::default() }, "alice").unwrap()
    }

    #[test]
    fn reports_relation_attachment_and_user_issues_with_indexes() {
        let mut db = open_in_memory_db().unwrap();
        let existing = seed(&mut db, "Existing");

        let mut d = draft("New");
        d.relations = vec![
            RelationDraft { relation: "blocks".to_string(), target: existing.id.clone() },
            RelationDraft { relation: "blocks".to_string(), target: "AB1333".to_string() },
            RelationDraft { relation: "blockz".to_string(), target: existing.id.clone() },
            RelationDraft { relation: "blocks".to_string(), target: existing.id.clone() },
            RelationDraft { relation: "related".to_string(), target: "zz".to_string() },
        ];
        d.attachments = vec![AttachmentInput { path: "/definitely/not/here.png".to_string(), description: None }];
        d.assignee = Some("quebert".to_string());
        d.due_date = Some("2000-01-01".to_string());
        d.difficulty = Some(Scalar::Text("9".to_string()));

        let issues = validate_task_draft(&db, &d, "alice");
        let find = |field: TaskField, idx: Option<usize>| issues.iter().find(|i| i.field == field && i.index == idx);

        assert!(find(TaskField::Relations, Some(0)).is_none(), "valid relation has no issue");
        assert!(
            find(TaskField::Relations, Some(1))
                .unwrap()
                .message
                .contains("No task AB1333")
        );
        assert!(
            find(TaskField::Relations, Some(2))
                .unwrap()
                .message
                .contains("Unknown relation 'blockz'")
        );
        assert!(
            find(TaskField::Relations, Some(3))
                .unwrap()
                .message
                .contains("twice")
        );
        assert!(
            find(TaskField::Relations, Some(4))
                .unwrap()
                .message
                .contains("not a valid task ID")
        );
        assert!(
            find(TaskField::Attachments, Some(0))
                .unwrap()
                .message
                .contains("File not found")
        );
        assert!(
            find(TaskField::DueDate, None)
                .unwrap()
                .message
                .contains("past")
        );
        assert!(find(TaskField::Difficulty, None).is_some());
        let info = find(TaskField::Assignee, None).unwrap();
        assert_eq!(info.level, IssueLevel::Info);
        assert!(info.message.contains("@quebert is new"));
    }

    #[test]
    fn creation_is_atomic_and_creates_relations_attachments_and_users() {
        let mut db = open_in_memory_db().unwrap();
        let existing = seed(&mut db, "Existing");
        let before = list_tasks(&db).unwrap().len();

        // A bad relation rejects the draft and nothing is created.
        let mut bad = draft("Half made?");
        bad.relations = vec![RelationDraft { relation: "blocks".to_string(), target: "AB1333".to_string() }];
        let err = create_task_from_draft(&mut db, &bad, "alice").unwrap_err();
        assert!(matches!(err, WsidError::DraftRejected(_)));
        assert!(err.to_string().contains("No task AB1333"));
        assert_eq!(list_tasks(&db).unwrap().len(), before);

        // A cycle only detectable at link time rolls back the whole transaction.
        let mut cyc = draft("Cyclic");
        cyc.relations = vec![
            RelationDraft { relation: "blocks".to_string(), target: existing.id.clone() },
            RelationDraft { relation: "blocked_by".to_string(), target: existing.id.clone() },
        ];
        assert!(create_task_from_draft(&mut db, &cyc, "alice").is_err());
        assert_eq!(list_tasks(&db).unwrap().len(), before);

        // Happy path with a file, a relation, text numbers, and a brand-new assignee.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ok = draft("Real task");
        ok.relations = vec![RelationDraft { relation: "blocked_by".to_string(), target: existing.id.clone() }];
        ok.attachments = vec![AttachmentInput { path: file.to_string_lossy().to_string(), description: Some("My notes".to_string()) }];
        ok.assignee = Some("quebert".to_string());
        ok.importance = Some(Scalar::Text("4".to_string()));
        ok.time_estimate = Some(Scalar::Text("1h30m".to_string()));
        ok.urgent_override = Some(Scalar::Text("yes".to_string()));
        let created = create_task_from_draft(&mut db, &ok, "alice").unwrap();

        assert_eq!(created.importance, 4);
        assert_eq!(created.time_estimate_minutes, Some(90));
        assert!(created.urgent_override);
        assert_eq!(created.attachments.len(), 1);
        assert_eq!(created.attachments[0].description, "My notes");
        assert_eq!(
            crate::relations::get_direct_blockers(&db, &created.id)
                .unwrap()
                .len(),
            1
        );
        assert!(
            list_known_users(&db, &[])
                .unwrap()
                .contains(&"quebert".to_string())
        );
    }

    #[test]
    fn suggests_users_and_relation_targets() {
        let mut db = open_in_memory_db().unwrap();
        let t = seed(&mut db, "Fix login bug");
        crate::tasks::register_user(&mut db, "quebert").unwrap();
        assert_eq!(suggest_users(&db, "@que", &[]).unwrap(), vec!["quebert".to_string()]);
        let hits = search_relation_targets(&db, "login", 5).unwrap();
        assert_eq!(hits[0].id, t.id);
    }
}
