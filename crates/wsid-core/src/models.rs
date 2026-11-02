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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRelation {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
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
