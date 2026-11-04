//! Bulk task editing shared by the GUI, CLI, TUI, and MCP surfaces.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::Result;
use crate::models::{Task, TaskStatus, UpdateTaskInput, UserNotice};
use crate::notes::transition_task_status;
use crate::tasks::{delete_task, get_task, normalize_tags, update_task_with_notices};

/// Field changes applied to every selected task. `None` leaves a field unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BulkTaskPatch {
    pub status: Option<TaskStatus>,
    pub assignee: Option<String>,
    pub importance: Option<u8>,
    /// Raises importance to at least this value (applied after `importance`).
    pub importance_at_least: Option<u8>,
    /// Lowers importance to at most this value (applied after `importance_at_least`).
    pub importance_at_most: Option<u8>,
    pub difficulty: Option<u8>,
    pub urgent_override: Option<bool>,
    pub project: Option<String>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkTaskFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BulkEditOutcome {
    /// Tasks successfully updated (final state).
    pub updated: Vec<Task>,
    /// IDs successfully deleted (delete operations only).
    pub deleted: Vec<String>,
    pub spawned_review_tasks: Vec<Task>,
    pub auto_completed_parent_tasks: Vec<Task>,
    pub failures: Vec<BulkTaskFailure>,
    /// Typed user reference notices (`@kind:user`), deduplicated across tasks.
    #[serde(default)]
    pub user_notices: Vec<UserNotice>,
}

fn dedupe(ids: &[String]) -> Vec<&String> {
    let mut seen = std::collections::BTreeSet::new();
    ids.iter()
        .filter(|id| seen.insert(id.to_ascii_uppercase()))
        .collect()
}

/// Applies `patch` to each task. Each task succeeds or fails independently; failures are reported, not fatal.
pub fn bulk_update_tasks(db: &mut Database, ids: &[String], patch: &BulkTaskPatch) -> Result<BulkEditOutcome> {
    let add_tags = normalize_tags(&patch.add_tags)?;
    let remove_tags = normalize_tags(&patch.remove_tags)?;
    let mut outcome = BulkEditOutcome::default();

    for id in dedupe(ids) {
        match apply_patch(db, id, patch, &add_tags, &remove_tags, &mut outcome) {
            Ok(task) => outcome.updated.push(task),
            Err(err) => outcome
                .failures
                .push(BulkTaskFailure { id: id.clone(), error: err.to_string() }),
        }
    }
    Ok(outcome)
}

fn apply_patch(
    db: &mut Database,
    id: &str,
    patch: &BulkTaskPatch,
    add_tags: &[String],
    remove_tags: &[String],
    outcome: &mut BulkEditOutcome,
) -> Result<Task> {
    let current = get_task(db, id)?;

    let mut importance = patch.importance.unwrap_or(current.importance);
    if let Some(min) = patch.importance_at_least {
        importance = importance.max(min);
    }
    if let Some(max) = patch.importance_at_most {
        importance = importance.min(max);
    }

    let tags = if add_tags.is_empty() && remove_tags.is_empty() {
        None
    } else {
        let mut next: Vec<String> = current
            .tags
            .iter()
            .filter(|t| !remove_tags.contains(t))
            .cloned()
            .collect();
        for tag in add_tags {
            if !next.contains(tag) {
                next.push(tag.clone());
            }
        }
        Some(next)
    };

    let has_field_change = patch.assignee.is_some()
        || importance != current.importance
        || patch.difficulty.is_some()
        || patch.urgent_override.is_some()
        || patch.project.is_some()
        || tags.is_some();

    let mut task = current;
    if has_field_change {
        let (updated, notices) = update_task_with_notices(
            db,
            id,
            UpdateTaskInput {
                assignee: patch.assignee.clone(),
                importance: Some(importance),
                difficulty: patch.difficulty,
                urgent_override: patch.urgent_override,
                project: patch.project.clone(),
                tags,
                ..Default::default()
            },
        )?;
        task = updated;
        for notice in notices {
            if !outcome.user_notices.contains(&notice) {
                outcome.user_notices.push(notice);
            }
        }
    }

    // Status goes through the transition so review handoffs and parent auto-complete fire.
    if let Some(status) = patch.status
        && status != task.status
    {
        let transition = transition_task_status(db, id, status)?;
        task = transition.task;
        outcome
            .spawned_review_tasks
            .extend(transition.spawned_review_task);
        outcome
            .auto_completed_parent_tasks
            .extend(transition.auto_completed_parent_tasks);
    }
    Ok(task)
}

/// Deletes each task; per-task failures are reported in the outcome.
pub fn bulk_delete_tasks(db: &mut Database, ids: &[String]) -> Result<BulkEditOutcome> {
    let mut outcome = BulkEditOutcome::default();
    for id in dedupe(ids) {
        match delete_task(db, id) {
            Ok(()) => outcome.deleted.push(id.clone()),
            Err(err) => outcome
                .failures
                .push(BulkTaskFailure { id: id.clone(), error: err.to_string() }),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;
    use crate::models::CreateTaskInput;
    use crate::tasks::create_task;

    fn make(db: &mut Database, title: &str, importance: u8) -> Task {
        create_task(
            db,
            CreateTaskInput {
                title: title.to_string(),
                importance: Some(importance),
                tags: vec!["alpha".to_string()],
                ..Default::default()
            },
            "tester",
        )
        .expect("create")
    }

    #[test]
    fn bulk_patch_updates_tags_importance_status_and_reports_failures() {
        let mut db = open_in_memory_db().expect("db");
        let a = make(&mut db, "A", 1);
        let b = make(&mut db, "B", 5);

        let patch = BulkTaskPatch {
            status: Some(TaskStatus::InProgress),
            importance_at_least: Some(3),
            add_tags: vec!["beta".to_string()],
            remove_tags: vec!["alpha".to_string()],
            ..Default::default()
        };
        let outcome = bulk_update_tasks(&mut db, &[a.id.clone(), b.id.clone(), "ZZ9999".to_string(), a.id.clone()], &patch).expect("bulk");
        assert_eq!(outcome.updated.len(), 2);
        assert_eq!(outcome.failures.len(), 1);
        let a2 = get_task(&db, &a.id).unwrap();
        let b2 = get_task(&db, &b.id).unwrap();
        assert_eq!(a2.importance, 3);
        assert_eq!(b2.importance, 5);
        assert_eq!(a2.tags, vec!["beta".to_string()]);
        assert_eq!(a2.status, TaskStatus::InProgress);

        let deleted = bulk_delete_tasks(&mut db, &[a.id.clone(), b.id.clone()]).expect("delete");
        assert_eq!(deleted.deleted.len(), 2);
        assert!(get_task(&db, &a.id).is_err());
    }

    #[test]
    fn bulk_assignment_reports_typed_user_notices_once() {
        use crate::models::UserNotice;
        use crate::tasks::set_user_kind;

        let mut db = open_in_memory_db().expect("db");
        let a = make(&mut db, "A", 1);
        let b = make(&mut db, "B", 1);
        // `rob` already exists as a jira user, so `claude:` must not change the stored kind.
        create_task(
            &mut db,
            CreateTaskInput { title: "Seed".to_string(), assignee: Some("rob".to_string()), ..Default::default() },
            "tester",
        )
        .expect("seed");
        set_user_kind(&mut db, "rob", "jira").expect("kind");

        let patch = BulkTaskPatch { assignee: Some("claude:rob".to_string()), ..Default::default() };
        let outcome = bulk_update_tasks(&mut db, &[a.id.clone(), b.id.clone()], &patch).expect("bulk");

        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.updated.len(), 2);
        assert!(outcome.updated.iter().all(|t| t.assignee == "rob"));
        assert_eq!(
            outcome.user_notices,
            vec![UserNotice::KindMismatch { user: "rob".to_string(), requested: "claude".to_string(), actual: "jira".to_string() }]
        );
    }
}
