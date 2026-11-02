use std::path::PathBuf;
use std::str::FromStr;

use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};
use wsid_core::{FocusPreset, RelationInputKind, Result, TaskStatus, WsidError};

#[derive(Debug, Parser)]
#[command(name = "wsid", version, about = "WSID (What Should I Do?) — Offline-first, accessible task manager & MCP server")]
pub struct Cli {
    /// Explicit path to a SQLite database file
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    /// Use workspace-local `.wsid.db` (discovers existing or creates in git root / current directory)
    #[arg(long, global = true)]
    pub local: bool,

    /// Active persona handle (defaults to configured `default_persona` or OS username)
    #[arg(long, short = 'p', global = true)]
    pub persona: Option<String>,

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

    /// Append a note/comment to a task (supports `@persona`, `@takenote:<persona>`, and `#AB1234`)
    Note(NoteArgs),

    /// Delete a task note (automatically cleaning up any managed app-data attachments on disk)
    DeleteNote {
        /// Numeric note ID
        note_id: i64,
    },

    /// Delete a task (automatically cleaning up any managed app-data attachments on disk)
    Delete {
        /// Task short ID (e.g. AB1234)
        id: String,
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

    /// List tasks assigned to the active persona that are currently blocked or awaiting review
    Blocked {
        /// Optional tag filter
        #[arg(long, short = 't')]
        tag: Option<String>,
    },

    /// Show tasks classified into the 4 Eisenhower Matrix quadrants
    Matrix {
        /// Include all assignees instead of filtering to the active persona
        #[arg(long)]
        all: bool,
    },

    /// Run the passive stdio JSON-RPC 2.0 Model Context Protocol (MCP) server
    Mcp {
        /// Log diagnostic events to stderr
        #[arg(long, short = 'v')]
        verbose: bool,
    },
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

    /// Hard due date (`YYYY-MM-DD`)
    #[arg(long)]
    pub due: Option<String>,

    /// "Should Get Done By" target date (`YYYY-MM-DD`, must be `<= due` when both are set)
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
    #[arg(long, short = 'u')]
    pub urgent: bool,

    /// Manual priority tie-breaker (`-5..=5`)
    #[arg(long)]
    pub priority: Option<i8>,

    /// Single-word tag(s) (can be specified multiple times)
    #[arg(long = "tag", short = 't')]
    pub tags: Vec<String>,

    /// Reporter persona (defaults to active persona)
    #[arg(long)]
    pub reporter: Option<String>,

    /// Assignee persona (defaults to active persona)
    #[arg(long, short = 'a')]
    pub assignee: Option<String>,

    /// Evaluator persona (defaults to active persona)
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

    /// Comment body (supports `@persona`, `@takenote:<persona>`, and `#AB1234`)
    pub body: String,

    /// Override comment author persona (defaults to active persona)
    #[arg(long)]
    pub author: Option<String>,

    /// Disable automatic `@persona` / `@takenote:<persona>` subtask spawning
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
    /// Search query (short ID, title, tag, description, or persona; empty matches all)
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

/// Parses human-friendly time estimates like `"15m"`, `"2h"`, `"1h30m"`, `"3d"`, `"1w"`, or plain minutes `"45"`.
/// Uses 8-hour workdays (`480` minutes/day) and 5-day workweeks (`2400` minutes/week).
pub fn parse_duration_estimate(raw: &str) -> Result<i64> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(WsidError::Validation("time estimate cannot be empty".to_string()));
    }

    if let Ok(plain_mins) = trimmed.parse::<i64>() {
        if plain_mins < 0 {
            return Err(WsidError::Validation("time estimate cannot be negative".to_string()));
        }
        return Ok(plain_mins);
    }

    let mut total_minutes: i64 = 0;
    let mut current_digits = String::new();

    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            current_digits.push(ch);
        } else if ch.is_whitespace() {
            continue;
        } else {
            if current_digits.is_empty() {
                return Err(WsidError::Validation(format!(
                    "invalid time estimate '{raw}': expected format like '15m', '2h', '3d', '1w', or '1h30m'"
                )));
            }
            let val: i64 = current_digits
                .parse()
                .map_err(|_| WsidError::Validation(format!("invalid number in time estimate '{raw}'")))?;
            current_digits.clear();

            let multiplier = match ch {
                'm' => 1,
                'h' => 60,
                'd' => 480,
                'w' => 2400,
                _ => {
                    return Err(WsidError::Validation(format!(
                        "unsupported unit '{ch}' in time estimate '{raw}': use m (minutes), h (hours), d (8h days), or w (5d weeks)"
                    )));
                }
            };
            total_minutes = total_minutes.saturating_add(val.saturating_mul(multiplier));
        }
    }

    if !current_digits.is_empty() {
        let val: i64 = current_digits
            .parse()
            .map_err(|_| WsidError::Validation(format!("invalid number in time estimate '{raw}'")))?;
        total_minutes = total_minutes.saturating_add(val);
    }

    Ok(total_minutes)
}

pub fn format_duration_estimate(minutes: Option<i64>) -> String {
    let Some(mins) = minutes else {
        return "-".to_string();
    };
    if mins <= 0 {
        return "0m".to_string();
    }
    if mins % 2400 == 0 {
        format!("{}w", mins / 2400)
    } else if mins % 480 == 0 {
        format!("{}d", mins / 480)
    } else if mins >= 60 {
        let h = mins / 60;
        let rem = mins % 60;
        if rem == 0 { format!("{h}h") } else { format!("{h}h{rem}m") }
    } else {
        format!("{mins}m")
    }
}

pub fn parse_date_arg(field: &str, raw: Option<&str>) -> Result<Option<NaiveDate>> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|_| WsidError::Validation(format!("invalid {field} '{trimmed}': expected YYYY-MM-DD")))?;
    Ok(Some(date))
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
        assert_eq!(parse_duration_estimate("3d").unwrap(), 1440);
        assert_eq!(parse_duration_estimate("1w").unwrap(), 2400);
        assert_eq!(parse_duration_estimate("45").unwrap(), 45);

        assert!(parse_duration_estimate("10x").is_err());
        assert!(parse_duration_estimate("").is_err());

        assert_eq!(format_duration_estimate(Some(15)), "15m");
        assert_eq!(format_duration_estimate(Some(120)), "2h");
        assert_eq!(format_duration_estimate(Some(90)), "1h30m");
        assert_eq!(format_duration_estimate(Some(480)), "1d");
        assert_eq!(format_duration_estimate(Some(2400)), "1w");
        assert_eq!(format_duration_estimate(None), "-");
    }
}
