//! Add Task form model: input masks, list fields (relations, attachments), typeahead dropdowns, and debounced
//! error state. All validation and creation go through `wsid_core::draft`, so the TUI, GUI, CLI, and MCP share
//! one set of rules.

use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use ratatui::widgets::ListState;
use wsid_core::{
    AttachmentInput, Database, FieldIssue, IssueLevel, RelationDraft, RelationInputKind, Scalar, TaskDraft, TaskField,
    format_deadline_display, format_duration_minutes, local_tz_abbr, normalize_task_id, parse_deadline_datetime, parse_duration_minutes,
    search_relation_targets, suggest_users, validate_task_draft,
};

use crate::tui::text_edit::{self, EditKind, TextHistory};
use crate::tui::user_ref::{user_kind_mismatch, user_label};

/// How long the active field must be idle before it is validated.
pub const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(400);

const DESCRIPTION: usize = 1;
const DUE_DATE: usize = 2;
const SGDB_DATE: usize = 3;
const TIME_ESTIMATE: usize = 4;
const RELATIONS: usize = 10;
const ATTACHMENTS: usize = 11;
/// Reporter, Assignee, Evaluator.
const PERSONA_FIELDS: [usize; 3] = [12, 13, 14];
const MAX_DROPDOWN_ROWS: usize = 8;

fn clean(msg: &str) -> String {
    msg.strip_prefix("validation error: ")
        .unwrap_or(msg)
        .to_string()
}

fn tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
}

fn opt(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() { None } else { Some(v.to_string()) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mask {
    None,
    /// Digits only, auto-formatted as `YYYY-MM-DD HH:MM` (separators inserted as you type).
    DateTime,
}

impl Mask {
    /// Formats up to 12 digits as `YYYY-MM-DD HH:MM`, appending the next separator as soon as a group is complete.
    pub fn format_digits(digits: &str) -> String {
        let mut out = String::new();
        for (i, d) in digits.chars().take(12).enumerate() {
            out.push(d);
            match i + 1 {
                4 | 6 => out.push('-'),
                8 => out.push(' '),
                10 => out.push(':'),
                _ => {}
            }
        }
        out
    }

    /// Splits a date-time field into its 12 wall-clock digits and an optional time zone
    /// designator (`Z`, or a signed `±HH[:MM]` offset).
    fn split_value(value: &str) -> (String, String) {
        let mut digits = String::new();
        let mut suffix = String::new();
        let mut chars = value.chars();
        for ch in chars.by_ref() {
            if digits.len() < 12 {
                if ch.is_ascii_digit() {
                    digits.push(ch);
                }
                continue;
            }
            match ch {
                'Z' | 'z' => {
                    suffix = "Z".to_string();
                    break;
                }
                '+' | '-' => {
                    let offset: String = chars
                        .by_ref()
                        .filter(char::is_ascii_digit)
                        .take(4)
                        .collect();
                    suffix = format!("{ch}{}", Self::format_offset(&offset));
                    break;
                }
                _ => {}
            }
        }
        (digits, suffix)
    }

    /// Renders offset digits as `HH`, `HH:M`, or `HH:MM` while the user is typing.
    fn format_offset(digits: &str) -> String {
        if digits.len() <= 2 { digits.to_string() } else { format!("{}:{}", &digits[..2], &digits[2..]) }
    }

    /// Applies a typed character; returns false if it was rejected.
    pub fn push(self, value: &mut String, ch: char) -> bool {
        match self {
            Self::None => {
                value.push(ch);
                true
            }
            Self::DateTime => {
                let (mut digits, suffix) = Self::split_value(value);
                if !suffix.is_empty() {
                    // A suffix is already present: only digits can extend a signed offset.
                    let Some(sign) = suffix.chars().next().filter(|c| *c == '+' || *c == '-') else {
                        return false;
                    };
                    if !ch.is_ascii_digit() {
                        return false;
                    }
                    let mut offset: String = suffix[1..].chars().filter(char::is_ascii_digit).collect();
                    if offset.len() >= 4 {
                        return false;
                    }
                    offset.push(ch);
                    *value = format!("{}{sign}{}", Self::format_digits(&digits), Self::format_offset(&offset));
                    return true;
                }
                if ch.is_ascii_digit() {
                    if digits.len() >= 12 {
                        return false;
                    }
                    digits.push(ch);
                    *value = Self::format_digits(&digits);
                    return true;
                }
                if digits.len() < 12 {
                    // Separators and a pasted ISO `T` are implied by the mask.
                    return matches!(ch, '-' | ':' | ' ' | 'T' | 't' | '/');
                }
                // Full wall clock: `Z` or a signed offset may follow.
                match ch {
                    'Z' | 'z' => {
                        *value = format!("{}Z", Self::format_digits(&digits));
                        true
                    }
                    '+' | '-' => {
                        *value = format!("{}{ch}", Self::format_digits(&digits));
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    pub fn backspace(self, value: &mut String) {
        match self {
            Self::None => {
                value.pop();
            }
            Self::DateTime => {
                let (mut digits, suffix) = Self::split_value(value);
                if !suffix.is_empty() {
                    // Peel the suffix first, then fall back to the wall clock.
                    let mut offset: String = suffix[1..].chars().filter(char::is_ascii_digit).collect();
                    if let Some(sign) = suffix.chars().next().filter(|c| *c == '+' || *c == '-') {
                        offset.pop();
                        *value = if offset.is_empty() {
                            Self::format_digits(&digits)
                        } else {
                            format!("{}{sign}{}", Self::format_digits(&digits), Self::format_offset(&offset))
                        };
                    } else {
                        *value = Self::format_digits(&digits);
                    }
                    return;
                }
                digits.pop();
                *value = Self::format_digits(&digits);
            }
        }
    }

    /// Backward-word deletion for the row: masked rows drop a whole segment (`HH:MM`, then
    /// `DD`, `MM`, `YYYY`) so the mask stays well-formed.
    pub fn delete_word_backward(self, value: &mut String) {
        if !matches!(self, Self::DateTime) {
            super::text_edit::delete_word_backward(value);
            return;
        }
        let (digits, suffix) = Self::split_value(value);
        if !suffix.is_empty() {
            *value = Self::format_digits(&digits);
            return;
        }
        // Segment boundaries of `YYYY-MM-DD HH:MM`, as digit counts.
        let boundary = [4, 6, 8, 10, 12]
            .into_iter()
            .rev()
            .find(|b| *b < digits.len())
            .unwrap_or(0);
        *value = Self::format_digits(&digits[..boundary]);
    }
}

/// Normalizes a duration string to its canonical human-readable form (`"200m"` -> `"3h 20m"`).
pub fn normalize_duration(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    parse_duration_minutes(trimmed)
        .ok()
        .map(format_duration_minutes)
}

pub struct FieldSpec {
    pub label: &'static str,
    pub mask: Mask,
    pub field: TaskField,
}

pub const FIELD_COUNT: usize = 16;

const PROJECT: usize = 15;

pub static FIELDS: [FieldSpec; FIELD_COUNT] = [
    FieldSpec { label: "1. Title *", mask: Mask::None, field: TaskField::Title },
    FieldSpec { label: "2. Description", mask: Mask::None, field: TaskField::Description },
    FieldSpec { label: "3. Due Date (YYYY-MM-DD HH:MM)", mask: Mask::DateTime, field: TaskField::DueDate },
    FieldSpec { label: "4. SGDB Date (YYYY-MM-DD HH:MM, <= Due)", mask: Mask::DateTime, field: TaskField::SgdbDate },
    FieldSpec { label: "5. Time Estimate (15m, 2h, 3d, 1w)", mask: Mask::None, field: TaskField::TimeEstimate },
    FieldSpec { label: "6. Difficulty (1-5)", mask: Mask::None, field: TaskField::Difficulty },
    FieldSpec { label: "7. Importance (1-5)", mask: Mask::None, field: TaskField::Importance },
    FieldSpec { label: "8. Manual Priority (1-5)", mask: Mask::None, field: TaskField::ManualPriority },
    FieldSpec { label: "9. Urgent Override (true/false)", mask: Mask::None, field: TaskField::UrgentOverride },
    FieldSpec { label: "10. Tags (single words)", mask: Mask::None, field: TaskField::Tags },
    FieldSpec { label: "11. Relations (e.g. blocks:AB1234)", mask: Mask::None, field: TaskField::Relations },
    FieldSpec { label: "12. Attachments (file path)", mask: Mask::None, field: TaskField::Attachments },
    FieldSpec { label: "13. Reporter", mask: Mask::None, field: TaskField::Reporter },
    FieldSpec { label: "14. Assignee", mask: Mask::None, field: TaskField::Assignee },
    FieldSpec { label: "15. Evaluator", mask: Mask::None, field: TaskField::Evaluator },
    FieldSpec { label: "16. Project (default)", mask: Mask::None, field: TaskField::Project },
];

/// Row of the form that displays issues for `field`.
pub fn field_index(field: TaskField) -> usize {
    FIELDS.iter().position(|f| f.field == field).unwrap_or(0)
}

/// Row label, including the local short time zone abbreviation on the Due/SGDB rows.
/// The optional `Z` / `±HH:MM` suffix is explained by the hint under those rows.
pub fn field_label(idx: usize) -> String {
    let tz = local_tz_abbr();
    match idx {
        DUE_DATE => format!("3. Due Date (YYYY-MM-DD HH:MM {tz})"),
        SGDB_DATE => format!("4. SGDB Date (YYYY-MM-DD HH:MM {tz}, <= Due)"),
        _ => FIELDS[idx].label.to_string(),
    }
}

/// Display width of the label column (longest label), so list text and cursor position share one source of truth.
pub fn label_width() -> usize {
    (0..FIELD_COUNT)
        .map(|i| field_label(i).chars().count())
        .max()
        .unwrap_or(0)
}

/// `>> ` highlight symbol (3) + label column + `: ` (2).
pub fn value_column() -> usize {
    3 + label_width() + 2
}

/// Splits pasted or dragged text into file paths: handles quotes, backslash-escaped characters, `file://` URLs and
/// whitespace/newline separators.
pub fn parse_pasted_paths(raw: &str) -> Vec<String> {
    const ESCAPABLE: &str = " \'\"()[]&#!;,~$\\";
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = raw.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quote != Some('\'') && chars.peek().is_some_and(|n| ESCAPABLE.contains(*n)) => {
                cur.push(chars.next().unwrap_or(c));
                started = true;
            }
            '"' | '\'' if quote.is_none() => {
                quote = Some(c);
                started = true;
            }
            c if Some(c) == quote => quote = None,
            c if c.is_whitespace() && quote.is_none() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out.into_iter()
        .map(|p| match p.strip_prefix("file://") {
            Some(rest) => rest.replace("%20", " "),
            None => p,
        })
        .collect()
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// One `@user` / `@takenote:user` / `#ID` completion offered while typing a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentSuggestion {
    /// Text that replaces the trailing token (includes its `@` / `#` sigil).
    pub insert: String,
    /// Primary text shown in the suggestion list.
    pub label: String,
    /// Secondary hint: task status, "known user", or "new user".
    pub hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentTokenKind {
    Mention,
    TakeNote,
    TaskRef,
}

#[derive(Debug, Clone, Copy)]
pub struct TrailingCommentToken<'a> {
    pub raw: &'a str,
    pub query: &'a str,
    pub kind: CommentTokenKind,
}

fn is_comment_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '@' || c == '#' || c == ':'
}

/// Returns the incomplete `@…`, `@takenote:…`, or `#…` token at the end of `input`.
pub fn trailing_comment_token(input: &str) -> Option<TrailingCommentToken<'_>> {
    let start = input
        .char_indices()
        .rev()
        .find(|(_, c)| !is_comment_token_char(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let raw = &input[start..];
    if let Some(rest) = raw.strip_prefix('#') {
        return Some(TrailingCommentToken { raw, query: rest, kind: CommentTokenKind::TaskRef });
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("@takenote:") {
        return Some(TrailingCommentToken { raw, query: &raw["@takenote:".len()..], kind: CommentTokenKind::TakeNote });
    }
    raw.strip_prefix('@')
        .map(|rest| TrailingCommentToken { raw, query: rest, kind: CommentTokenKind::Mention })
}

/// What accepting a dropdown row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Relation keyword; fills `kind:` into the input.
    Kind(&'static str),
    /// Relation target task ID.
    Target(String),
    /// Replaces the input text (a known user or a path; paths ending in `/` keep the dropdown open).
    Text(String),
    /// Completes the trailing `@…` / `#…` token in place.
    Token(String),
    /// A committed list item in the manage dropdown.
    Item(usize),
}

#[derive(Debug, Clone)]
pub struct Dropdown {
    pub labels: Vec<String>,
    pub choices: Vec<Choice>,
    pub selected: usize,
    /// Lists committed items (Delete removes the highlighted one) rather than suggestions.
    pub manage: bool,
}

impl Dropdown {
    fn new(items: Vec<(String, Choice)>, manage: bool) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let (labels, choices) = items.into_iter().unzip();
        Some(Self { labels, choices, selected: 0, manage })
    }

    pub fn step(&mut self, delta: isize) {
        let len = self.labels.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }
}

#[derive(Debug, Clone)]
pub struct DataEntryForm {
    /// Text buffers. For the Relations and Attachments rows this is the input for the *next* item.
    pub fields: [String; FIELD_COUNT],
    pub errors: [Option<String>; FIELD_COUNT],
    /// Informational notes (for example "@quebert is new and will be created").
    pub notes: [Option<String>; FIELD_COUNT],
    pub relations: Vec<RelationDraft>,
    /// Task titles for `relations`, shown in the sub-rows.
    pub relation_titles: Vec<String>,
    pub attachments: Vec<AttachmentInput>,
    /// File awaiting its alt text; while set, the Attachments buffer holds that alt text.
    pub pending_attachment: Option<String>,
    pub dropdown: Option<Dropdown>,
    pub active_index: usize,
    pub state: ListState,
    /// Set on each keystroke; the active field is validated once it has been idle for [`VALIDATION_DEBOUNCE`].
    pub dirty_at: Option<Instant>,
    /// Undo/redo stacks, one per text field.
    histories: [TextHistory; FIELD_COUNT],
}

impl DataEntryForm {
    pub const SUBMIT_INDEX: usize = FIELD_COUNT;

    pub fn new(default_user: &str) -> Self {
        Self::new_for(default_user, wsid_core::DEFAULT_PROJECT)
    }

    /// A pristine form whose reporter/assignee/evaluator default to `default_user` and whose
    /// project defaults to `default_project`.
    pub fn new_for(default_user: &str, default_project: &str) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        let mut fields: [String; FIELD_COUNT] = std::array::from_fn(|_| String::new());
        for i in [5, 6, 7] {
            fields[i] = "3".to_string();
        }
        fields[8] = "false".to_string();
        for i in [12, 13, 14] {
            fields[i] = default_user.to_string();
        }
        fields[PROJECT] = default_project.to_string();
        Self {
            fields,
            errors: std::array::from_fn(|_| None),
            notes: std::array::from_fn(|_| None),
            relations: Vec::new(),
            relation_titles: Vec::new(),
            attachments: Vec::new(),
            pending_attachment: None,
            dropdown: None,
            active_index: 0,
            state,
            dirty_at: None,
            histories: std::array::from_fn(|_| TextHistory::default()),
        }
    }

    /// Re-points the user and project fields that still hold the previous active context at the
    /// new one. Fields the user changed to something else are left alone, so drafts survive.
    pub fn retarget_context(&mut self, from_user: &str, from_project: &str, to_user: &str, to_project: &str) {
        let mut changed = false;
        for i in [12, 13, 14] {
            if self.fields[i] == from_user && from_user != to_user {
                self.fields[i] = to_user.to_string();
                changed = true;
            }
        }
        if self.fields[PROJECT] == from_project && from_project != to_project {
            self.fields[PROJECT] = to_project.to_string();
            changed = true;
        }
        if changed {
            self.dirty_at = Some(Instant::now());
        }
    }

    /// Builds the same form as [`Self::new`] but pre-filled from an existing task so it can be
    /// edited. Deadlines are shown as local wall clock without a zone suffix (editing in local
    /// terms), and attachments/relations are left empty because those are managed from the
    /// task detail view.
    pub fn from_task(task: &wsid_core::Task, default_user: &str) -> Self {
        let mut form = Self::new(default_user);
        let local_wall_clock = |raw: Option<&str>| {
            raw.and_then(|s| parse_deadline_datetime("deadline", s).ok())
                .map(|(_, dt)| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default()
        };
        form.fields[0] = task.title.clone();
        form.fields[1] = task.description.clone();
        form.fields[2] = local_wall_clock(task.due_date.as_deref());
        form.fields[3] = local_wall_clock(task.sgdb_date.as_deref());
        form.fields[4] = task
            .time_estimate_minutes
            .map(|m| format_duration_minutes(m.max(0) as u32))
            .unwrap_or_default();
        form.fields[5] = task.difficulty.to_string();
        form.fields[6] = task.importance.to_string();
        form.fields[7] = task.manual_priority.to_string();
        form.fields[8] = task.urgent_override.to_string();
        form.fields[9] = task.tags.join(", ");
        form.fields[12] = task.reporter.clone();
        form.fields[13] = task.assignee.clone();
        form.fields[14] = task.evaluator.clone();
        form.fields[15] = task.project.clone();
        form
    }

    /// The raw draft the core validates and creates from.
    pub fn to_draft(&self) -> TaskDraft {
        let f = &self.fields;
        let scalar = |i: usize| opt(&f[i]).map(Scalar::Text);
        TaskDraft {
            title: f[0].trim().to_string(),
            description: opt(&f[1]),
            due_date: opt(&f[2]),
            sgdb_date: opt(&f[3]),
            time_estimate: scalar(4),
            difficulty: scalar(5),
            importance: scalar(6),
            manual_priority: scalar(7),
            urgent_override: scalar(8),
            tags: tokens(&f[9]).map(ToString::to_string).collect(),
            relations: self.relations.clone(),
            attachments: self.attachments.clone(),
            reporter: opt(&f[12]),
            assignee: opt(&f[13]),
            evaluator: opt(&f[14]),
            project: opt(&f[15]),
            ..Default::default()
        }
    }

    /// Moves focus, validating the field being left.
    pub fn set_active_index(&mut self, db: &Database, user: &str, index: usize) {
        if self.active_index < FIELD_COUNT {
            self.normalize_field(self.active_index);
            self.validate_field(db, user, self.active_index);
        }
        self.dirty_at = None;
        self.dropdown = None;
        self.active_index = index;
        self.state.select(Some(index));
        if index < FIELD_COUNT {
            // A new field always starts a fresh undo group.
            self.histories[index].reset();
        }
        self.refresh_dropdown(db, user);
        self.refresh_local_hints();
    }

    fn normalize_field(&mut self, idx: usize) {
        if idx == TIME_ESTIMATE
            && let Some(norm) = normalize_duration(&self.fields[TIME_ESTIMATE])
        {
            self.fields[TIME_ESTIMATE] = norm;
        }
    }

    pub fn type_char(&mut self, db: &Database, user: &str, ch: char) {
        let idx = self.active_index;
        if idx < FIELD_COUNT {
            let before = self.fields[idx].clone();
            if FIELDS[idx].mask.push(&mut self.fields[idx], ch) {
                self.histories[idx].record(&before, EditKind::Typing);
                self.edited(db, user);
            }
        }
    }

    pub fn backspace(&mut self, db: &Database, user: &str) {
        let idx = self.active_index;
        if idx < FIELD_COUNT {
            let before = self.fields[idx].clone();
            FIELDS[idx].mask.backspace(&mut self.fields[idx]);
            if self.fields[idx] != before {
                self.histories[idx].record(&before, EditKind::Deleting);
            }
            self.edited(db, user);
        }
    }

    /// Applies the editing chords (`Option+Backspace`, `Ctrl+W`, `Cmd+Z`, `Ctrl+Y`, ...) to the
    /// active row. Returns true when the key was consumed.
    pub fn apply_text_shortcut(&mut self, db: &Database, user: &str, key: &KeyEvent) -> bool {
        let idx = self.active_index;
        if idx >= FIELD_COUNT {
            return false;
        }
        let mask = FIELDS[idx].mask;
        let Some(changed) = text_edit::apply_text_shortcut_with(key, &mut self.fields[idx], &mut self.histories[idx], |value| {
            let before = value.clone();
            mask.delete_word_backward(value);
            *value != before
        }) else {
            return false;
        };
        if changed {
            self.edited(db, user);
        }
        true
    }

    fn edited(&mut self, db: &Database, user: &str) {
        let idx = self.active_index;
        self.errors[idx] = None;
        self.dirty_at = Some(Instant::now());
        self.dropdown = None;
        self.refresh_dropdown(db, user);
        if (12..=14).contains(&idx) {
            self.refresh_notes(db, user);
        }
        self.refresh_local_hints();
    }

    /// Handles bracketed paste: file paths on the Attachments row, plain text elsewhere.
    pub fn paste(&mut self, db: &Database, user: &str, text: &str) {
        let idx = self.active_index;
        if idx == ATTACHMENTS && self.pending_attachment.is_none() {
            let before = self.fields[ATTACHMENTS].clone();
            self.fields[ATTACHMENTS] = text.trim().to_string();
            self.record_discrete(ATTACHMENTS, &before);
            self.commit_attachment_buffer();
        } else if idx < FIELD_COUNT {
            let before = self.fields[idx].clone();
            for ch in text_edit::flatten_paste(text).chars() {
                self.type_char(db, user, ch);
            }
            // Keep the whole paste as one undo step instead of one per character.
            if self.fields[idx] != before {
                self.histories[idx].reset();
                self.histories[idx].record(&before, EditKind::Discrete);
            }
        }
    }

    /// Records an undo point for `idx` when the buffer actually changed.
    fn record_discrete(&mut self, idx: usize, before: &str) {
        if idx < FIELD_COUNT && self.fields[idx] != before {
            self.histories[idx].record(before, EditKind::Discrete);
        }
    }

    // ----- dropdowns ------------------------------------------------------------------------------------------

    pub fn refresh_dropdown(&mut self, db: &Database, user: &str) {
        self.dropdown = match self.active_index {
            DESCRIPTION => self.comment_dropdown(db, user),
            RELATIONS => self.relation_dropdown(db),
            ATTACHMENTS if self.pending_attachment.is_none() => self.path_dropdown(),
            12..=14 => {
                // `kind:partial` suggests on the user part and keeps the typed kind.
                let raw = self.fields[self.active_index]
                    .trim()
                    .trim_start_matches('@')
                    .to_string();
                let (typed_kind, prefix) = match raw.split_once(':') {
                    Some((kind, rest)) => (Some(kind.to_string()), rest.to_string()),
                    None => (None, raw),
                };
                let kinds = wsid_core::list_user_kinds(db).unwrap_or_default();
                let items = suggest_users(db, &prefix, &[user])
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|u| !u.eq_ignore_ascii_case(&prefix))
                    .take(MAX_DROPDOWN_ROWS)
                    .map(|u| {
                        let label = user_label(&kinds, &u);
                        let insert = match &typed_kind {
                            Some(kind) => format!("{kind}:{u}"),
                            None => u,
                        };
                        (label, Choice::Text(insert))
                    })
                    .collect();
                Dropdown::new(items, false)
            }
            _ => None,
        };
    }

    /// In-place `@user` / `@takenote:user` / `#ID` completion for the description field.
    fn comment_dropdown(&self, db: &Database, user: &str) -> Option<Dropdown> {
        let token = trailing_comment_token(&self.fields[DESCRIPTION])?;
        let mut items: Vec<(String, Choice)> = Vec::new();
        match token.kind {
            CommentTokenKind::TaskRef => {
                if token.query.trim().is_empty() {
                    return None;
                }
                for task in search_relation_targets(db, token.query, MAX_DROPDOWN_ROWS).unwrap_or_default() {
                    items.push((format!("#{}  {}", task.id, task.title), Choice::Token(format!("#{}", task.id))));
                }
            }
            kind => {
                let scope = if kind == CommentTokenKind::TakeNote { "@takenote:" } else { "@" };
                for name in suggest_users(db, token.query, &[user])
                    .unwrap_or_default()
                    .into_iter()
                    .take(MAX_DROPDOWN_ROWS)
                {
                    items.push((format!("{scope}{name}"), Choice::Token(format!("{scope}{name}"))));
                }
                let typed = token.query.trim();
                if !typed.is_empty()
                    && !items
                        .iter()
                        .any(|(label, _)| label.eq_ignore_ascii_case(&format!("{scope}{typed}")))
                {
                    items.push((format!("{scope}{typed}  (new user)"), Choice::Token(format!("{scope}{typed}"))));
                }
            }
        }
        Dropdown::new(items, false)
    }

    fn relation_dropdown(&self, db: &Database) -> Option<Dropdown> {
        let text = self.fields[RELATIONS].trim();
        if text.is_empty() {
            return None;
        }
        let (kind, query) = match text.split_once(':') {
            Some((k, q)) => (Some(k.trim().to_ascii_lowercase()), q.trim()),
            None => (None, text),
        };
        let mut items: Vec<(String, Choice)> = Vec::new();
        match &kind {
            Some(k) if RelationInputKind::from_str(k).is_err() => return None,
            Some(_) => {}
            None => {
                let lower = text.to_ascii_lowercase();
                for name in RelationInputKind::NAMES
                    .iter()
                    .filter(|n| n.starts_with(&lower))
                {
                    items.push(((*name).to_string(), Choice::Kind(name)));
                }
            }
        }
        if !query.is_empty() {
            for task in search_relation_targets(db, query, MAX_DROPDOWN_ROWS).unwrap_or_default() {
                items.push((format!("{}  {}", task.id, task.title), Choice::Target(task.id)));
            }
        }
        items.truncate(MAX_DROPDOWN_ROWS);
        Dropdown::new(items, false)
    }

    fn path_dropdown(&self) -> Option<Dropdown> {
        let text = self.fields[ATTACHMENTS].trim();
        if text.is_empty() || text.starts_with('\'') || text.starts_with('"') || text.starts_with("file://") {
            return None;
        }
        let (dir, prefix) = match text.rfind('/') {
            Some(i) => (&text[..=i], &text[i + 1..]),
            None => ("./", text),
        };
        let dir = if dir.starts_with("~/") {
            std::env::var("HOME").map_or_else(|_| dir.to_string(), |h| format!("{h}{}", &dir[1..]))
        } else {
            dir.to_string()
        };
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                (name.starts_with(prefix) && (prefix.starts_with('.') || !name.starts_with('.'))).then(|| (name, e.path().is_dir()))
            })
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let items = entries
            .into_iter()
            .take(MAX_DROPDOWN_ROWS)
            .map(|(name, is_dir)| {
                let shown = if text.starts_with("~/") || text.contains('/') {
                    format!("{}{name}", &text[..text.rfind('/').map_or(0, |i| i + 1)])
                } else {
                    name
                };
                let full = if is_dir { format!("{shown}/") } else { shown };
                (full.clone(), Choice::Text(full))
            })
            .collect();
        Dropdown::new(items, false)
    }

    fn open_manage(&mut self) {
        let items: Vec<(String, Choice)> = if self.active_index == RELATIONS {
            self.relations
                .iter()
                .zip(&self.relation_titles)
                .enumerate()
                .map(|(i, (r, t))| (format!("{}  {}  {t}", r.relation, r.target), Choice::Item(i)))
                .collect()
        } else {
            self.attachments
                .iter()
                .enumerate()
                .map(|(i, a)| (format!("{} - \"{}\"", file_name(&a.path), a.description.as_deref().unwrap_or("")), Choice::Item(i)))
                .collect()
        };
        self.dropdown = Dropdown::new(items, true);
    }

    /// Delete/Backspace in the manage dropdown removes the highlighted item. Returns true if it handled the key.
    pub fn delete_selected_item(&mut self) -> bool {
        let Some(dd) = self.dropdown.as_ref().filter(|d| d.manage) else {
            return false;
        };
        if let Some(Choice::Item(i)) = dd.choices.get(dd.selected).cloned() {
            if self.active_index == RELATIONS {
                self.relations.remove(i);
                self.relation_titles.remove(i);
            } else {
                self.attachments.remove(i);
            }
            let keep = dd.selected.saturating_sub(1);
            self.open_manage();
            if let Some(d) = self.dropdown.as_mut() {
                d.selected = keep.min(d.labels.len() - 1);
            }
        }
        true
    }

    /// Whether Esc should be consumed by the form (closing a dropdown or cancelling the alt-text step).
    pub fn wants_escape(&self) -> bool {
        self.dropdown.is_some() || self.pending_attachment.is_some()
    }

    pub fn escape(&mut self, db: &Database, user: &str) {
        if self.dropdown.take().is_some() {
            return;
        }
        if let Some(path) = self.pending_attachment.take() {
            self.fields[ATTACHMENTS] = path;
            self.refresh_dropdown(db, user);
        }
    }

    // ----- Enter handling ---------------------------------------------------------------------------------------

    /// Applies Enter to the active row. Returns true when focus should advance to the next row.
    pub fn enter(&mut self, db: &Database, user: &str) -> bool {
        let idx = self.active_index;
        if idx == RELATIONS && self.dropdown.as_ref().is_none_or(|d| !d.manage) && self.typed_target().is_some() {
            self.commit_relation(db, user);
            return false;
        }
        if let Some(dd) = self.dropdown.take() {
            if dd.manage {
                return false;
            }
            self.accept_choice(db, user, dd.choices[dd.selected].clone());
            return false;
        }
        match idx {
            RELATIONS => {
                if !self.fields[RELATIONS].trim().is_empty() {
                    self.commit_relation(db, user);
                    false
                } else if !self.relations.is_empty() {
                    self.open_manage();
                    false
                } else {
                    true
                }
            }
            ATTACHMENTS => {
                if let Some(path) = self.pending_attachment.take() {
                    let alt = opt(&self.fields[ATTACHMENTS]).unwrap_or_else(|| file_name(&path));
                    self.attachments
                        .push(AttachmentInput { path, description: Some(alt) });
                    self.fields[ATTACHMENTS].clear();
                    false
                } else if !self.fields[ATTACHMENTS].trim().is_empty() {
                    self.commit_attachment_buffer();
                    false
                } else if !self.attachments.is_empty() {
                    self.open_manage();
                    false
                } else {
                    true
                }
            }
            _ => {
                self.normalize_field(idx);
                self.refresh_local_hints();
                true
            }
        }
    }

    fn accept_choice(&mut self, db: &Database, user: &str, choice: Choice) {
        let idx = self.active_index;
        let before = if idx < FIELD_COUNT { self.fields[idx].clone() } else { String::new() };
        match choice {
            Choice::Kind(k) => {
                self.fields[RELATIONS] = format!("{k}:");
                self.refresh_dropdown(db, user);
            }
            Choice::Target(id) => {
                let kind = self.fields[RELATIONS]
                    .split_once(':')
                    .map_or("related", |(k, _)| k.trim())
                    .to_ascii_lowercase();
                self.fields[RELATIONS] = format!("{kind}:{id}");
                self.commit_relation(db, user);
            }
            Choice::Text(t) => {
                let keep_open = idx == ATTACHMENTS && t.ends_with('/');
                self.fields[idx] = t;
                self.errors[idx] = None;
                if keep_open {
                    self.refresh_dropdown(db, user);
                } else if (12..=14).contains(&idx) {
                    self.refresh_notes(db, user);
                }
            }
            Choice::Token(replacement) => {
                if let Some(keep) = trailing_comment_token(&self.fields[idx]).map(|t| self.fields[idx].len() - t.raw.len()) {
                    self.fields[idx].truncate(keep);
                }
                self.fields[idx].push_str(&replacement);
                self.fields[idx].push(' ');
                self.errors[idx] = None;
                self.refresh_dropdown(db, user);
            }
            Choice::Item(_) => {}
        }
        self.record_discrete(idx, &before);
    }

    /// The relation buffer's target when it is a complete task ID (`blocks:AB1234` or bare `AB1234`).
    fn typed_target(&self) -> Option<(String, String)> {
        let text = self.fields[RELATIONS].trim();
        let (kind, target) = text.split_once(':').unwrap_or(("related", text));
        let id = normalize_task_id(target.trim()).ok()?;
        Some((kind.trim().to_ascii_lowercase(), id))
    }

    /// Validates the relation buffer against the core rules (kind, existence, duplicates, self-links).
    fn check_relation(&self, db: &Database, user: &str, draft_item: &RelationDraft) -> Result<(), String> {
        let mut draft = self.to_draft();
        draft.title = "x".to_string();
        draft.relations.push(draft_item.clone());
        let at = draft.relations.len() - 1;
        validate_task_draft(db, &draft, user)
            .into_iter()
            .find(|i| i.is_error() && i.field == TaskField::Relations && i.index.is_none_or(|n| n == at))
            .map_or(Ok(()), |i| Err(clean(&i.message)))
    }

    fn relation_buffer_error(&self, db: &Database, user: &str) -> Option<String> {
        let text = self.fields[RELATIONS].trim();
        if text.is_empty() {
            return None;
        }
        let (kind, target) = text.split_once(':').unwrap_or(("related", text));
        let kind = kind.trim().to_ascii_lowercase();
        if RelationInputKind::from_str(&kind).is_err() {
            return Some(format!("Unknown relation '{kind}' ({})", RelationInputKind::NAMES.join(", ")));
        }
        let target = target.trim();
        if target.is_empty() {
            return None;
        }
        match normalize_task_id(target) {
            Ok(id) => self
                .check_relation(db, user, &RelationDraft { relation: kind, target: id })
                .err(),
            Err(_) => None,
        }
    }

    fn commit_relation(&mut self, db: &Database, user: &str) {
        let text = self.fields[RELATIONS].trim().to_string();
        let (kind, target) = text.split_once(':').unwrap_or(("related", text.as_str()));
        let kind = kind.trim().to_ascii_lowercase();
        let item = match normalize_task_id(target.trim()) {
            Ok(id) => RelationDraft { relation: kind.clone(), target: id },
            Err(e) => {
                self.errors[RELATIONS] = Some(clean(&e.to_string()));
                return;
            }
        };
        if RelationInputKind::from_str(&kind).is_err() {
            self.errors[RELATIONS] = self.relation_buffer_error(db, user);
            return;
        }
        match self.check_relation(db, user, &item) {
            Ok(()) => {
                let title = wsid_core::get_task(db, &item.target)
                    .map(|t| t.title)
                    .unwrap_or_default();
                self.relations.push(item);
                self.relation_titles.push(title);
                self.fields[RELATIONS].clear();
                self.errors[RELATIONS] = None;
                self.dropdown = None;
            }
            Err(msg) => self.errors[RELATIONS] = Some(msg),
        }
    }

    fn commit_attachment_buffer(&mut self) {
        let raw = self.fields[ATTACHMENTS].trim().to_string();
        let paths = if Path::new(&raw).is_file() { vec![raw.clone()] } else { parse_pasted_paths(&raw) };
        if paths.is_empty() {
            return;
        }
        if let Some(bad) = paths.iter().find(|p| !Path::new(p).is_file()) {
            self.errors[ATTACHMENTS] = Some(format!("File not found: {bad}"));
            return;
        }
        self.errors[ATTACHMENTS] = None;
        self.dropdown = None;
        if let [only] = paths.as_slice() {
            self.fields[ATTACHMENTS] = file_name(only);
            self.pending_attachment = Some(only.clone());
        } else {
            for p in paths {
                let alt = file_name(&p);
                self.attachments
                    .push(AttachmentInput { path: p, description: Some(alt) });
            }
            self.fields[ATTACHMENTS].clear();
        }
    }

    // ----- validation -------------------------------------------------------------------------------------------

    fn refresh_local_hints(&mut self) {
        let tz = local_tz_abbr();
        for idx in [DUE_DATE, SGDB_DATE] {
            let trimmed = self.fields[idx].trim();
            self.notes[idx] = if !trimmed.is_empty() && parse_deadline_datetime("deadline", trimmed).is_ok() {
                Some(format!("= {}", format_deadline_display(trimmed)))
            } else if self.active_index == idx {
                Some(format!("Local time zone: {tz} — add Z or ±HH:MM to override (date-only defaults to 20:00 {tz})"))
            } else {
                None
            };
        }
        self.notes[TIME_ESTIMATE] = normalize_duration(&self.fields[TIME_ESTIMATE]).map(|d| format!("= {d}"));
    }

    fn refresh_notes(&mut self, db: &Database, user: &str) {
        let issues = validate_task_draft(db, &self.to_draft(), user);
        for (i, note) in self.notes.iter_mut().enumerate() {
            *note = issues
                .iter()
                .find(|n| n.level == IssueLevel::Info && field_index(n.field) == i)
                .map(|n| n.message.clone());
        }
        self.refresh_local_hints();
        self.apply_kind_warnings(db);
    }

    /// Replaces the note on Reporter/Assignee/Evaluator with a kind-mismatch warning when one applies.
    fn apply_kind_warnings(&mut self, db: &Database) {
        let kinds = wsid_core::list_user_kinds(db).unwrap_or_default();
        for idx in PERSONA_FIELDS {
            if let Some(warning) = user_kind_mismatch(&self.fields[idx], &kinds) {
                self.notes[idx] = Some(warning);
            }
        }
    }

    fn apply(&mut self, issues: &[FieldIssue], rows: &[usize]) {
        for &i in rows {
            self.errors[i] = issues
                .iter()
                .find(|n| n.is_error() && field_index(n.field) == i)
                .map(|n| clean(&n.message));
        }
        for (i, note) in self.notes.iter_mut().enumerate() {
            *note = issues
                .iter()
                .find(|n| n.level == IssueLevel::Info && field_index(n.field) == i)
                .map(|n| n.message.clone());
        }
        self.refresh_local_hints();
    }

    /// Validates the active field if it has been idle past the debounce window. Returns true if state changed.
    pub fn validate_if_idle(&mut self, db: &Database, user: &str, now: Instant) -> bool {
        match self.dirty_at {
            Some(t) if now.duration_since(t) >= VALIDATION_DEBOUNCE && self.active_index < FIELD_COUNT => {
                self.dirty_at = None;
                self.validate_field(db, user, self.active_index);
                true
            }
            _ => false,
        }
    }

    /// Validates one field (and the Due/SGDB ordering rule) through the shared core validator.
    pub fn validate_field(&mut self, db: &Database, user: &str, idx: usize) {
        let issues = validate_task_draft(db, &self.to_draft(), user);
        let rows: &[usize] = if idx == 2 || idx == 3 { &[2, 3] } else { &[idx] };
        // A row you haven't visited keeps its own state, except the paired date row.
        self.apply(&issues, rows);
        self.apply_kind_warnings(db);
        if idx == RELATIONS {
            self.errors[RELATIONS] = self
                .relation_buffer_error(db, user)
                .or(self.errors[RELATIONS].take());
        }
    }

    /// Validates every field, returning `(field index, message)` for each problem.
    pub fn validate_all(&mut self, db: &Database, user: &str) -> Vec<(usize, String)> {
        self.normalize_field(TIME_ESTIMATE);
        let issues = validate_task_draft(db, &self.to_draft(), user);
        let all: Vec<usize> = (0..FIELD_COUNT).collect();
        self.apply(&issues, &all);
        self.apply_kind_warnings(db);
        if !self.fields[RELATIONS].trim().is_empty() {
            self.errors[RELATIONS] = self
                .relation_buffer_error(db, user)
                .or_else(|| Some("Press Enter to add this relation, or clear it".to_string()));
        }
        if self.pending_attachment.is_some() {
            self.errors[ATTACHMENTS] = Some("Press Enter to confirm the alt text".to_string());
        } else if !self.fields[ATTACHMENTS].trim().is_empty() {
            self.errors[ATTACHMENTS] = Some("Press Enter to add this file, or clear it".to_string());
        }
        self.dirty_at = None;
        self.errors
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.clone().map(|m| (i, m)))
            .collect()
    }

    /// Footer keymap for the active row.
    pub fn hint(&self) -> &'static str {
        if self.dropdown.as_ref().is_some_and(|d| d.manage) {
            return "[Up/Down] Select | [Delete] Remove | [Enter/Esc] Close";
        }
        if self.dropdown.is_some() {
            return "[Up/Down] Choose | [Enter] Pick | [Esc] Close | [Tab] Next field";
        }
        match self.active_index {
            RELATIONS => "[type] kind or task | [Enter] Add | [Enter on empty] Manage | [Tab] Next | [Ctrl+S] Submit",
            ATTACHMENTS if self.pending_attachment.is_some() => "[type] Alt text | [Enter] Add file | [Esc] Back",
            ATTACHMENTS => "[type/paste/drop] File path | [Enter] Add | [Enter on empty] Manage | [Tab] Next | [Ctrl+S] Submit",
            12..=14 => "[type] user or kind:user (new users are created) | [Tab] Next | [Ctrl+S] Submit",
            _ => "[Tab/Up/Down] Field | [Alt+Bksp] Del word | [Ctrl+Z] Undo | [Ctrl+S] Submit | [Esc] Cancel",
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    fn typed(mask: Mask, s: &str) -> String {
        let mut v = String::new();
        for ch in s.chars() {
            mask.push(&mut v, ch);
        }
        v
    }

    #[test]
    fn date_mask_deletes_whole_segments() {
        let mut v = typed(Mask::DateTime, "209901010930");
        assert_eq!(v, "2099-01-01 09:30");
        Mask::DateTime.delete_word_backward(&mut v);
        assert_eq!(v, "2099-01-01 09:");
        Mask::DateTime.delete_word_backward(&mut v);
        assert_eq!(v, "2099-01-01 ");
        Mask::DateTime.delete_word_backward(&mut v);
        assert_eq!(v, "2099-01-");
        Mask::DateTime.delete_word_backward(&mut v);
        assert_eq!(v, "2099-");
        Mask::DateTime.delete_word_backward(&mut v);
        assert_eq!(v, "");
    }

    #[test]
    fn word_delete_and_undo_apply_to_the_active_field() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("chords.db")).unwrap();
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", 0);
        for ch in "Address @davidb note".chars() {
            form.type_char(&db, "alice", ch);
        }

        let option_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT);
        assert!(form.apply_text_shortcut(&db, "alice", &option_backspace));
        assert_eq!(form.fields[0], "Address @davidb ");
        assert!(form.apply_text_shortcut(&db, "alice", &option_backspace));
        assert_eq!(form.fields[0], "Address ");

        let undo = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert!(form.apply_text_shortcut(&db, "alice", &undo));
        assert_eq!(form.fields[0], "Address @davidb ");
        assert!(form.apply_text_shortcut(&db, "alice", &undo));
        assert_eq!(form.fields[0], "Address @davidb note");

        // Plain characters and plain Backspace keep flowing through the normal handlers.
        let plain = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!form.apply_text_shortcut(&db, "alice", &plain));
        let typed_char = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(!form.apply_text_shortcut(&db, "alice", &typed_char));
    }

    #[test]
    fn date_mask_inserts_separators_and_handles_backspace_and_paste() {
        assert_eq!(typed(Mask::DateTime, "20990101"), "2099-01-01 ");
        assert_eq!(typed(Mask::DateTime, "209901010930"), "2099-01-01 09:30");
        assert_eq!(typed(Mask::DateTime, "2099"), "2099-");
        // Pasted ISO text with separators and a `T` normalises to the mask; extra digits are rejected.
        assert_eq!(typed(Mask::DateTime, "2099-01-01T09:00"), "2099-01-01 09:00");
        assert_eq!(typed(Mask::DateTime, "2099010109301234"), "2099-01-01 09:30");
        assert_eq!(typed(Mask::DateTime, "abc12"), "12");

        let mut v = typed(Mask::DateTime, "20990101");
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "2099-01-0");
        let mut v = typed(Mask::DateTime, "2099");
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "209");
    }

    #[test]
    fn date_mask_accepts_utc_and_numeric_offset_suffixes() {
        // `Z` marks the wall clock as UTC.
        assert_eq!(typed(Mask::DateTime, "2099-01-01 12:00Z"), "2099-01-01 12:00Z");

        // Signed offsets, with progressive formatting while typing.
        let mut v = typed(Mask::DateTime, "2099-01-01 12:00");
        assert!(Mask::DateTime.push(&mut v, '-'));
        assert_eq!(v, "2099-01-01 12:00-");
        for ch in "05".chars() {
            assert!(Mask::DateTime.push(&mut v, ch));
        }
        assert_eq!(v, "2099-01-01 12:00-05");
        for ch in "30".chars() {
            assert!(Mask::DateTime.push(&mut v, ch));
        }
        assert_eq!(v, "2099-01-01 12:00-05:30");
        // No fifth offset digit.
        assert!(!Mask::DateTime.push(&mut v, '9'));
        // No second designator.
        assert!(!Mask::DateTime.push(&mut v, 'Z'));

        // Backspace peels the offset, then the wall clock.
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "2099-01-01 12:00-05:3");
        Mask::DateTime.backspace(&mut v);
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "2099-01-01 12:00-0");
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "2099-01-01 12:00");
        Mask::DateTime.backspace(&mut v);
        assert_eq!(v, "2099-01-01 12:0");

        // A pasted full value keeps its suffix intact.
        assert_eq!(typed(Mask::DateTime, "2099-01-01T12:00+0530"), "2099-01-01 12:00+05:30");
    }

    #[test]
    fn form_shows_local_hint_for_utc_and_offset_deadlines() {
        let db = Database::open_in_memory().unwrap();
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", DUE_DATE);
        for ch in "2026-11-03 12:00Z".chars() {
            form.type_char(&db, "alice", ch);
        }
        assert_eq!(form.fields[DUE_DATE], "2026-11-03 12:00Z");
        assert_eq!(form.notes[DUE_DATE].as_deref(), Some(format!("= {}", format_deadline_display("2026-11-03T12:00:00Z")).as_str()));

        // A numeric offset also round-trips through core validation, and stores as UTC.
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", DUE_DATE);
        for ch in "2099-11-03 12:00-05:00".chars() {
            form.type_char(&db, "alice", ch);
        }
        assert_eq!(form.fields[DUE_DATE], "2099-11-03 12:00-05:00");
        assert!(
            form.validate_all(&db, "alice")
                .iter()
                .all(|(i, _)| *i != DUE_DATE)
        );
        assert_eq!(
            wsid_core::parse_deadline_datetime("due_date", &form.fields[DUE_DATE])
                .unwrap()
                .0,
            "2099-11-03T17:00:00Z"
        );
    }

    #[test]
    fn parses_pasted_and_dropped_paths() {
        assert_eq!(parse_pasted_paths("/tmp/a.png"), ["/tmp/a.png"]);
        assert_eq!(parse_pasted_paths("'/tmp/a b.png'"), ["/tmp/a b.png"]);
        assert_eq!(parse_pasted_paths("/tmp/a\\ b.png /tmp/c.txt"), ["/tmp/a b.png", "/tmp/c.txt"]);
        assert_eq!(parse_pasted_paths("\"/tmp/x y.txt\"\n/tmp/z.txt"), ["/tmp/x y.txt", "/tmp/z.txt"]);
        assert_eq!(parse_pasted_paths("file:///tmp/a%20b.png"), ["/tmp/a b.png"]);
        assert_eq!(parse_pasted_paths(r"C:\Users\me\a.txt"), [r"C:\Users\me\a.txt"]);
        assert!(parse_pasted_paths("   ").is_empty());
    }

    #[test]
    fn form_validates_through_core_after_debounce_on_blur_and_cross_field() {
        let db = Database::open_in_memory().unwrap();
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", 2);
        for ch in "01222000".chars() {
            form.type_char(&db, "alice", ch);
        }
        assert!(form.errors[2].is_none(), "no error before debounce");
        let t0 = form.dirty_at.unwrap();
        assert!(!form.validate_if_idle(&db, "alice", t0 + Duration::from_millis(100)));
        assert!(form.validate_if_idle(&db, "alice", t0 + VALIDATION_DEBOUNCE));
        assert!(form.errors[2].is_some());

        // Blur validates immediately; SGDB later than Due is flagged on the SGDB field.
        form.fields[2] = "2099-01-01".to_string();
        form.fields[3] = "2099-01-05".to_string();
        form.set_active_index(&db, "alice", 3);
        form.set_active_index(&db, "alice", 4);
        assert!(form.errors[3].as_deref().unwrap().contains("on or before"));

        let errs = form.validate_all(&db, "alice");
        assert!(errs.iter().any(|(i, _)| *i == 0), "title required");
    }

    #[test]
    fn normalizes_time_estimate_on_blur_and_shows_live_hint() {
        let db = Database::open_in_memory().unwrap();
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", 4);
        for ch in "200m".chars() {
            form.type_char(&db, "alice", ch);
        }
        assert_eq!(form.fields[4], "200m");
        assert_eq!(form.notes[4].as_deref(), Some("= 3h 20m"));

        form.set_active_index(&db, "alice", 5);
        assert_eq!(form.fields[4], "3h 20m");
    }

    #[test]
    fn detects_trailing_comment_tokens_at_word_boundaries() {
        let mention = trailing_comment_token("please review @bo").expect("mention token");
        assert_eq!((mention.raw, mention.query, mention.kind), ("@bo", "bo", CommentTokenKind::Mention));

        let takenote = trailing_comment_token("ping @takenote:ali").expect("takenote token");
        assert_eq!(takenote.query, "ali");
        assert_eq!(takenote.kind, CommentTokenKind::TakeNote);

        let reference = trailing_comment_token("see #YW43").expect("task token");
        assert_eq!((reference.raw, reference.query, reference.kind), ("#YW43", "YW43", CommentTokenKind::TaskRef));

        // A finished token followed by a space is not being typed any more.
        assert!(trailing_comment_token("@alice ").is_none());
        // Plain words and mid-sentence sigils that started a new word are not tokens.
        assert!(trailing_comment_token("hello world").is_none());
        assert!(trailing_comment_token("a#b").is_none());
    }

    #[test]
    fn description_dropdown_completes_user_and_task_tokens_in_place() {
        let mut db = Database::open_in_memory().unwrap();
        wsid_core::create_task(&mut db, wsid_core::CreateTaskInput { title: "Ship release".to_string(), ..Default::default() }, "alice")
            .unwrap();
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(&db, "alice", DESCRIPTION);
        form.fields[DESCRIPTION] = "please review @bo".to_string();
        form.refresh_dropdown(&db, "alice");
        let dd = form.dropdown.as_ref().expect("user suggestions");
        assert!(dd.labels.iter().any(|l| l.starts_with("@bo")));
        form.accept_choice(&db, "alice", Choice::Token("@bob".to_string()));
        assert_eq!(form.fields[DESCRIPTION], "please review @bob ");

        form.fields[DESCRIPTION] = "see #ship".to_string();
        form.refresh_dropdown(&db, "alice");
        let dd = form.dropdown.as_ref().expect("task suggestions");
        assert!(
            dd.labels
                .iter()
                .any(|l| l.starts_with('#') && l.contains("Ship release"))
        );
    }
}
