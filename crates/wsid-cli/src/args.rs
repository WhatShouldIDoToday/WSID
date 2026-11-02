use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand};
use wsid_core::{FocusPreset, RelationInputKind, Result, TaskStatus, parse_deadline_datetime};

#[derive(Debug, Parser)]
#[command(name = "wsid", version, about = "WSID (What Should I Do?) — Offline-first, accessible task manager & MCP server")]
pub struct Cli {
    /// Explicit path to a SQLite database file
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    /// Use workspace-local `.wsid.db` (discovers existing or creates in git root / current directory)
    #[arg(long, global = true)]
    pub local: bool,

    /// Active user handle (defaults to configured `default_user` or OS username)
    #[arg(long, short = 'u', global = true)]
    pub user: Option<String>,

    /// Enable linear screen-reader-friendly accessible output / single-column TUI layout
    #[arg(long, global = true)]
    pub accessible: bool,

    /// Emit machine-readable JSON output for CLI subcommands
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Launch the interactive terminal UI (default when no subcommand is given)
    Tui,

    /// Show the top prioritized actionable tasks in the Focus Queue plus Blocked Summary
    Next(NextArgs),

    /// Add a new task with optional dates, estimate, tags, relations, and file attachments
    Add(Box<AddArgs>),

    /// Show full details, relations, attachments, and notes for a task
    Show {
        /// Task short ID (e.g. AB1234)
        id: String,
    },

    /// Transition a task's status (enforcing `in_review` handoff when assignee != evaluator)
    Status {
        /// Task short ID (e.g. AB1234)
        id: String,
        /// Target status (`backlog`, `todo`, `in_progress`, `in_review`, `stuck`, `completed`)
        status: String,
    },

    /// Append a note/comment to a task (supports `@user`, `@takenote:<user>`, and `#AB1234`)
    Note(NoteArgs),

    /// Delete a task note (automatically cleaning up any managed app-data attachments on disk)
    DeleteNote {
        /// Numeric note ID
        note_id: i64,
    },

    /// Delete one or more tasks (automatically cleaning up any managed app-data attachments on disk)
    Delete {
        /// Task short ID(s) (e.g. AB1234 CD5678)
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
    },

    /// Edit one or more tasks at once (status, assignee, importance, difficulty, urgency, tags)
    Edit(EditArgs),

    /// Manage users (list, add, or — as the main user — delete)
    Users {
        #[command(subcommand)]
        action: UsersAction,
    },

    /// Show tasks grouped into Kanban status columns
    Kanban {
        /// Include all assignees instead of filtering to the active user
        #[arg(long)]
        all: bool,
    },

    /// Attach a file on disk or clipboard data to a task or comment
    Attach(AttachArgs),

    /// Update the accessible alt-text description of an attachment
    UpdateAttachment {
        /// Numeric attachment ID
        attachment_id: i64,
        /// Accessible alt-text description (omit or pass empty string to reset to file name)
        #[arg(long)]
        desc: Option<String>,
    },

    /// Remove an attachment (deleting the file on disk only if stored in managed app data)
    RemoveAttachment {
        /// Numeric attachment ID
        attachment_id: i64,
    },

    /// Open an attachment file in the native OS viewer application
    OpenAttachment {
        /// Numeric attachment ID
        attachment_id: i64,
    },

    /// Link two tasks (`blocks`, `blocked_by`, `sub_task_of`, `parent_task_of`, `parallel`, `related`)
    Link {
        /// Source task ID (e.g. AB1234)
        source_id: String,
        /// Relation kind
        relation: String,
        /// Target task ID (e.g. CD5678)
        target_id: String,
    },

    /// Remove a link between two tasks
    Unlink {
        /// Source task ID (e.g. AB1234)
        source_id: String,
        /// Relation kind
        relation: String,
        /// Target task ID (e.g. CD5678)
        target_id: String,
    },

    /// Dismiss or convert an overzealous blocking subtask into a non-blocking related task
    Unblock {
        /// Subtask ID (e.g. AB1234)
        subtask_id: String,
        /// Keep the subtask open as a non-blocking related task instead of marking it completed
        #[arg(long)]
        keep_open: bool,
    },

    /// Search tasks using hybrid FTS5 trigram + fuzzy subsequence and typo-tolerant matching
    Search(SearchArgs),

    /// List tasks assigned to the active user that are currently blocked or awaiting review
    Blocked {
        /// Optional tag filter
        #[arg(long, short = 't')]
        tag: Option<String>,
    },

    /// Show tasks classified into the 4 Eisenhower Matrix quadrants
    Matrix {
        /// Include all assignees instead of filtering to the active user
        #[arg(long)]
        all: bool,
    },

    /// Run the passive stdio JSON-RPC 2.0 Model Context Protocol (MCP) server
    Mcp {
        /// Log diagnostic events to stderr
        #[arg(long, short = 'v')]
        verbose: bool,
    },

    /// Display WSID application version, Apache-2.0 license, repository URL, and third-party license attributions
    #[command(visible_alias = "licenses")]
    About {
        /// Print full third-party license notice texts in addition to the crate attribution summary
        #[arg(long)]
        full: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum UsersAction {
    /// List all known users
    List,
    /// Register a user handle (lowercase letters, digits, `_`, `-`)
    Add { user: String },
    /// Delete a user, reassigning their tasks to the main user (previews unless `--yes` is given)
    Delete {
        user: String,
        /// Also permanently delete every task this user authored (reporter)
        #[arg(long)]
        delete_authored: bool,
        /// Perform the deletion instead of only previewing it
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Args)]
pub struct EditArgs {
    /// Task short ID(s) to edit (e.g. AB1234 CD5678)
    #[arg(required = true, num_args = 1..)]
    pub ids: Vec<String>,

    /// New status (`backlog`, `todo`, `in_progress`, `in_review`, `stuck`, `completed`)
    #[arg(long)]
    pub status: Option<String>,

    /// New assignee
    #[arg(long, short = 'a')]
    pub assignee: Option<String>,

    /// Importance rating (`1..=5`)
    #[arg(long, short = 'i')]
    pub importance: Option<u8>,

    /// Difficulty rating (`1..=5`)
    #[arg(long)]
    pub difficulty: Option<u8>,

    /// Force urgent classification on
    #[arg(long, short = 'U', conflicts_with = "not_urgent")]
    pub urgent: bool,

    /// Clear the urgent override
    #[arg(long)]
    pub not_urgent: bool,

    /// Tag(s) to add (can be specified multiple times)
    #[arg(long = "add-tag")]
    pub add_tags: Vec<String>,

    /// Tag(s) to remove (can be specified multiple times)
    #[arg(long = "remove-tag")]
    pub remove_tags: Vec<String>,
}

#[derive(Debug, Args)]
pub struct NextArgs {
    /// Focus scoring preset (`balanced`, `quick_wins`, `eat_the_frog`, `unblock_others`)
    #[arg(long)]
    pub preset: Option<String>,

    /// Maximum number of actionable tasks to return
    #[arg(long, short = 'n')]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Task title
    pub title: String,

    /// Detailed description
    #[arg(long, short = 'd')]
    pub description: Option<String>,

    /// Initial status (`backlog`, `todo`, `in_progress`, `in_review`, `stuck`, `completed`)
    #[arg(long)]
    pub status: Option<String>,

    /// Hard due date (`YYYY-MM-DD` or ISO 8601 datetime)
    #[arg(long)]
    pub due: Option<String>,

    /// "Should Get Done By" target date (`YYYY-MM-DD` or ISO 8601 datetime, must be `<= due` when both are set)
    #[arg(long)]
    pub sgdb: Option<String>,

    /// Time estimate (e.g. `15m`, `2h`, `3d`, `1w`, or minutes like `45`)
    #[arg(long, short = 'e')]
    pub estimate: Option<String>,

    /// Difficulty rating (`1..=5`)
    #[arg(long)]
    pub difficulty: Option<u8>,

    /// Importance rating (`1..=5`)
    #[arg(long, short = 'i')]
    pub importance: Option<u8>,

    /// Force urgent classification regardless of date slack
    #[arg(long, short = 'U')]
    pub urgent: bool,

    /// Manual priority rating (`1..=5`, `3` = normal)
    #[arg(long)]
    pub priority: Option<i8>,

    /// Single-word tag(s) (can be specified multiple times)
    #[arg(long = "tag", short = 't')]
    pub tags: Vec<String>,

    /// Reporter user (defaults to active user)
    #[arg(long)]
    pub reporter: Option<String>,

    /// Assignee user (defaults to active user)
    #[arg(long, short = 'a')]
    pub assignee: Option<String>,

    /// Evaluator user (defaults to active user)
    #[arg(long)]
    pub evaluator: Option<String>,

    /// Task IDs that this new task blocks
    #[arg(long = "blocks")]
    pub blocks: Vec<String>,

    /// Task IDs that block this new task
    #[arg(long = "blocked-by")]
    pub blocked_by: Vec<String>,

    /// Parent task IDs that this new task is a subtask of
    #[arg(long = "subtask-of")]
    pub subtask_of: Vec<String>,

    /// Child task IDs that this new task is a parent of
    #[arg(long = "parent-of")]
    pub parent_of: Vec<String>,

    /// Task IDs that can be worked on in parallel with this task
    #[arg(long = "parallel")]
    pub parallel: Vec<String>,

    /// Task IDs related to this task
    #[arg(long = "related")]
    pub related: Vec<String>,

    /// File path(s) on disk to attach to the task
    #[arg(long = "attach")]
    pub attachments: Vec<PathBuf>,

    /// Optional accessible alt-text description(s) corresponding to `--attach` files
    #[arg(long = "attach-desc")]
    pub attachment_descriptions: Vec<String>,
}

#[derive(Debug, Args)]
pub struct NoteArgs {
    /// Task short ID (e.g. AB1234)
    pub id: String,

    /// Comment body (supports `@user`, `@takenote:<user>`, and `#AB1234`)
    pub body: String,

    /// Override comment author user (defaults to active user)
    #[arg(long)]
    pub author: Option<String>,

    /// Disable automatic `@user` / `@takenote:<user>` subtask spawning
    #[arg(long)]
    pub no_spawn: bool,

    /// File path(s) on disk to attach to this note
    #[arg(long = "attach")]
    pub attachments: Vec<PathBuf>,

    /// Optional accessible alt-text description(s) corresponding to `--attach` files
    #[arg(long = "attach-desc")]
    pub attachment_descriptions: Vec<String>,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    /// Task short ID (e.g. AB1234)
    pub id: String,

    /// Existing file path on disk to link to the task/note
    #[arg(long, short = 'f', conflicts_with = "clipboard")]
    pub file: Option<PathBuf>,

    /// Read image/data from the system clipboard and save into the OS app-data attachments folder
    #[arg(long, conflicts_with = "file")]
    pub clipboard: bool,

    /// Optional numeric note ID to attach to a specific comment on the task
    #[arg(long)]
    pub note: Option<i64>,

    /// Optional accessible alt-text description (defaults to the file name when omitted)
    #[arg(long, short = 'd')]
    pub desc: Option<String>,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Search query (short ID, title, tag, description, or user; empty matches all)
    #[arg(default_value = "")]
    pub query: String,

    /// Filter by status
    #[arg(long)]
    pub status: Option<String>,

    /// Filter by tag
    #[arg(long, short = 't')]
    pub tag: Option<String>,

    /// Filter by assignee
    #[arg(long, short = 'a')]
    pub assignee: Option<String>,

    /// Filter by reporter
    #[arg(long)]
    pub reporter: Option<String>,

    /// Filter by evaluator
    #[arg(long)]
    pub evaluator: Option<String>,

    /// Maximum number of search hits
    #[arg(long, short = 'n')]
    pub limit: Option<usize>,
}

/// Parses human-friendly time estimates like `"1w 2d 3h 20m"`, `"1d 2h 20m"`, `"2h 30m"`, `"15m"`, or plain minutes `"45"`,
/// rejecting duplicate time units (e.g. `"20m 10m"`).
pub fn parse_duration_estimate(raw: &str) -> Result<i64> {
    let mins = wsid_core::parse_duration_minutes(raw)?;
    Ok(i64::from(mins))
}

pub fn format_duration_estimate(minutes: Option<i64>) -> String {
    let Some(mins) = minutes else {
        return "-".to_string();
    };
    if mins <= 0 {
        return "0m".to_string();
    }
    wsid_core::format_duration_minutes(mins as u32)
}

pub fn parse_date_arg(field: &str, raw: Option<&str>) -> Result<Option<String>> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_deadline_datetime(field, trimmed)?;
    Ok(Some(trimmed.to_string()))
}

pub fn parse_status_arg(raw: &str) -> Result<TaskStatus> {
    TaskStatus::from_str(raw)
}

pub fn parse_preset_arg(raw: &str) -> Result<FocusPreset> {
    FocusPreset::from_str(raw)
}

pub fn parse_relation_arg(raw: &str) -> Result<RelationInputKind> {
    RelationInputKind::from_str(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_duration_estimates() {
        assert_eq!(parse_duration_estimate("15m").unwrap(), 15);
        assert_eq!(parse_duration_estimate("2h").unwrap(), 120);
        assert_eq!(parse_duration_estimate("1h30m").unwrap(), 90);
        assert_eq!(parse_duration_estimate("1d 2h 20m").unwrap(), 1580);
        assert_eq!(parse_duration_estimate("20m 2h 1d").unwrap(), 1580);
        assert_eq!(parse_duration_estimate("1w").unwrap(), 10080);
        assert_eq!(parse_duration_estimate("45").unwrap(), 45);

        assert!(parse_duration_estimate("10x").is_err());
        assert!(parse_duration_estimate("20m 10m").is_err());
        assert!(parse_duration_estimate("1h 2h").is_err());
        assert!(parse_duration_estimate("").is_err());

        assert_eq!(format_duration_estimate(Some(15)), "15m");
        assert_eq!(format_duration_estimate(Some(120)), "2h");
        assert_eq!(format_duration_estimate(Some(90)), "1h 30m");
        assert_eq!(format_duration_estimate(Some(1580)), "1d 2h 20m");
        assert_eq!(format_duration_estimate(Some(10080)), "1w");
        assert_eq!(format_duration_estimate(None), "-");
    }
}
