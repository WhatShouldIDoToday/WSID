use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Result, WsidError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Stuck,
    Completed,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::InReview => "in_review",
            Self::Stuck => "stuck",
            Self::Completed => "completed",
        }
    }

    pub const fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskStatus {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "backlog" => Ok(Self::Backlog),
            "todo" => Ok(Self::Todo),
            "in_progress" | "in-progress" | "inprogress" => Ok(Self::InProgress),
            "in_review" | "in-review" | "inreview" => Ok(Self::InReview),
            "stuck" => Ok(Self::Stuck),
            "completed" | "done" => Ok(Self::Completed),
            other => Err(WsidError::Validation(format!("invalid task status '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginKind {
    Manual,
    Mention,
    #[serde(rename = "takenote")]
    TakeNote,
    Review,
}

impl OriginKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Mention => "mention",
            Self::TakeNote => "takenote",
            Self::Review => "review",
        }
    }
}

impl fmt::Display for OriginKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OriginKind {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "manual" => Ok(Self::Manual),
            "mention" => Ok(Self::Mention),
            "takenote" | "take_note" => Ok(Self::TakeNote),
            "review" => Ok(Self::Review),
            other => Err(WsidError::Validation(format!("invalid origin kind '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Blocks,
    ParentOf,
    Parallel,
    Related,
}

impl RelationType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::ParentOf => "parent_of",
            Self::Parallel => "parallel",
            Self::Related => "related",
        }
    }

    pub const fn is_directed_acyclic(self) -> bool {
        matches!(self, Self::Blocks | Self::ParentOf)
    }

    pub const fn is_symmetric(self) -> bool {
        matches!(self, Self::Parallel | Self::Related)
    }
}

/// User-facing relation kind that normalizes to a canonical [`RelationType`] edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationInputKind {
    Blocks,
    BlockedBy,
    ParentTaskOf,
    SubTaskOf,
    Parallel,
    Related,
}

impl RelationInputKind {
    /// Resolves `(source_id, input_kind, target_id)` into canonical `(canonical_source, canonical_target, RelationType)`.
    pub fn canonicalize<'a>(self, source_id: &'a str, target_id: &'a str) -> (&'a str, &'a str, RelationType) {
        match self {
            Self::Blocks => (source_id, target_id, RelationType::Blocks),
            Self::BlockedBy => (target_id, source_id, RelationType::Blocks),
            Self::ParentTaskOf => (source_id, target_id, RelationType::ParentOf),
            Self::SubTaskOf => (target_id, source_id, RelationType::ParentOf),
            Self::Parallel => {
                if source_id <= target_id {
                    (source_id, target_id, RelationType::Parallel)
                } else {
                    (target_id, source_id, RelationType::Parallel)
                }
            }
            Self::Related => {
                if source_id <= target_id {
                    (source_id, target_id, RelationType::Related)
                } else {
                    (target_id, source_id, RelationType::Related)
                }
            }
        }
    }
}

impl From<RelationType> for RelationInputKind {
    fn from(value: RelationType) -> Self {
        match value {
            RelationType::Blocks => Self::Blocks,
            RelationType::ParentOf => Self::ParentTaskOf,
            RelationType::Parallel => Self::Parallel,
            RelationType::Related => Self::Related,
        }
    }
}

impl FromStr for RelationInputKind {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "blocks" | "block" => Ok(Self::Blocks),
            "blocked_by" | "blocked-by" | "blockedby" => Ok(Self::BlockedBy),
            "parent_of" | "parent-of" | "parent_task_of" | "parent-task-of" | "parent" => Ok(Self::ParentTaskOf),
            "sub_task_of" | "sub-task-of" | "subtask_of" | "subtask-of" | "child_of" | "child-of" | "subtask" => Ok(Self::SubTaskOf),
            "parallel" | "parallel_to" | "parallel-to" => Ok(Self::Parallel),
            "related" | "related_to" | "related-to" => Ok(Self::Related),
            other => Err(WsidError::Validation(format!("invalid relation kind '{other}'"))),
        }
    }
}

impl fmt::Display for RelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RelationType {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "blocks" => Ok(Self::Blocks),
            "parent_of" | "parent-of" | "parent_task_of" => Ok(Self::ParentOf),
            "parallel" => Ok(Self::Parallel),
            "related" | "related_to" => Ok(Self::Related),
            other => Err(WsidError::Validation(format!("invalid canonical relation type '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusPreset {
    Balanced,
    QuickWins,
    EatTheFrog,
    UnblockOthers,
}

impl FocusPreset {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::QuickWins => "quick_wins",
            Self::EatTheFrog => "eat_the_frog",
            Self::UnblockOthers => "unblock_others",
        }
    }
}

impl fmt::Display for FocusPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FocusPreset {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "balanced" | "default" => Ok(Self::Balanced),
            "quick_wins" | "quick-wins" | "quickwins" => Ok(Self::QuickWins),
            "eat_the_frog" | "eat-the-frog" | "eatthefrog" | "frog" => Ok(Self::EatTheFrog),
            "unblock_others" | "unblock-others" | "unblockothers" | "unblock" => Ok(Self::UnblockOthers),
            other => Err(WsidError::Validation(format!("invalid focus preset '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRelation {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAttachment {
    pub id: i64,
    pub task_id: String,
    pub note_id: Option<i64>,
    pub file_path: String,
    pub file_name: String,
    pub media_type: String,
    pub description: String,
    pub is_managed: bool,
    pub created_at: DateTime<Utc>,
}

impl TaskAttachment {
    /// Returns the accessibility alt text (`description` when non-empty, otherwise `file_name`).
    pub fn alt_text(&self) -> &str {
        let trimmed = self.description.trim();
        if trimmed.is_empty() { &self.file_name } else { trimmed }
    }

    /// Returns `true` if `media_type` starts with `"image/"`.
    pub fn is_image(&self) -> bool {
        self.media_type.starts_with("image/")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInput {
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<TaskAttachment>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddTaskNoteOutcome {
    pub note: TaskNote,
    pub spawned_tasks: Vec<Task>,
    pub reused_tasks: Vec<Task>,
    pub completed_review_tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusTransitionOutcome {
    pub task: Task,
    pub spawned_review_task: Option<Task>,
    pub auto_completed_parent_tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub origin_kind: OriginKind,
    pub origin_note_id: Option<i64>,
    pub due_date: Option<NaiveDate>,
    pub sgdb_date: Option<NaiveDate>,
    pub time_estimate_minutes: Option<i64>,
    pub difficulty: u8,
    pub importance: u8,
    pub urgent_override: bool,
    pub manual_priority: i8,
    pub tags: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<TaskAttachment>,
    pub reporter: String,
    pub assignee: String,
    pub evaluator: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateTaskInput {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub origin_kind: Option<OriginKind>,
    pub origin_note_id: Option<i64>,
    pub due_date: Option<NaiveDate>,
    pub sgdb_date: Option<NaiveDate>,
    pub time_estimate_minutes: Option<i64>,
    pub difficulty: Option<u8>,
    pub importance: Option<u8>,
    pub urgent_override: Option<bool>,
    pub manual_priority: Option<i8>,
    pub tags: Vec<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateTaskInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    /// `Some(Some(d))` sets date, `Some(None)` clears date, `None` leaves unchanged.
    pub due_date: Option<Option<NaiveDate>>,
    /// `Some(Some(d))` sets date, `Some(None)` clears date, `None` leaves unchanged.
    pub sgdb_date: Option<Option<NaiveDate>>,
    /// `Some(Some(m))` sets estimate, `Some(None)` clears estimate, `None` leaves unchanged.
    pub time_estimate_minutes: Option<Option<i64>>,
    pub difficulty: Option<u8>,
    pub importance: Option<u8>,
    pub urgent_override: Option<bool>,
    pub manual_priority: Option<i8>,
    pub tags: Option<Vec<String>>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
}
