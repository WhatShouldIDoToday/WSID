//! Add Task form model: input masks, list fields (relations, attachments), typeahead dropdowns, and debounced
//! error state. All validation and creation go through `wsid_core::draft`, so the TUI, GUI, CLI, and MCP share
//! one set of rules.

use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;
use wsid_core::{
    AttachmentInput, Database, FieldIssue, IssueLevel, RelationDraft, RelationInputKind, Scalar, TaskDraft, TaskField, normalize_task_id,
    search_relation_targets, suggest_users, validate_task_draft,
};

/// How long the active field must be idle before it is validated.
pub const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(400);

const RELATIONS: usize = 10;
const ATTACHMENTS: usize = 11;
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

    fn digits(value: &str) -> String {
        value.chars().filter(char::is_ascii_digit).collect()
    }

    /// Applies a typed character; returns false if it was rejected.
    pub fn push(self, value: &mut String, ch: char) -> bool {
        match self {
            Self::None => {
                value.push(ch);
                true
            }
            Self::DateTime => {
                if !ch.is_ascii_digit() {
                    // Separators and a pasted ISO `T` are implied by the mask.
                    return matches!(ch, '-' | ':' | ' ' | 'T' | 't' | '/');
                }
                let mut digits = Self::digits(value);
                if digits.len() >= 12 {
                    return false;
                }
                digits.push(ch);
                *value = Self::format_digits(&digits);
                true
            }
        }
    }

    pub fn backspace(self, value: &mut String) {
        match self {
            Self::None => {
                value.pop();
            }
            Self::DateTime => {
                let mut digits = Self::digits(value);
                digits.pop();
                *value = Self::format_digits(&digits);
            }
        }
    }
}

pub struct FieldSpec {
    pub label: &'static str,
    pub mask: Mask,
    pub field: TaskField,
}

pub const FIELD_COUNT: usize = 15;

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
];

/// Row of the form that displays issues for `field`.
pub fn field_index(field: TaskField) -> usize {
    FIELDS.iter().position(|f| f.field == field).unwrap_or(0)
}

/// Display width of the label column (longest label), so list text and cursor position share one source of truth.
pub fn label_width() -> usize {
    FIELDS
        .iter()
        .map(|f| f.label.chars().count())
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

/// What accepting a dropdown row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Relation keyword; fills `kind:` into the input.
    Kind(&'static str),
    /// Relation target task ID.
    Target(String),
    /// Replaces the input text (a known user or a path; paths ending in `/` keep the dropdown open).
    Text(String),
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
}

impl DataEntryForm {
    pub const SUBMIT_INDEX: usize = FIELD_COUNT;

    pub fn new(default_user: &str) -> Self {
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
        }
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
            ..Default::default()
        }
    }

    /// Moves focus, validating the field being left.
    pub fn set_active_index(&mut self, db: &Database, user: &str, index: usize) {
        if self.active_index < FIELD_COUNT {
            self.validate_field(db, user, self.active_index);
        }
        self.dirty_at = None;
        self.dropdown = None;
        self.active_index = index;
        self.state.select(Some(index));
        self.refresh_dropdown(db, user);
    }

    pub fn type_char(&mut self, db: &Database, user: &str, ch: char) {
        let idx = self.active_index;
        if idx < FIELD_COUNT && FIELDS[idx].mask.push(&mut self.fields[idx], ch) {
            self.edited(db, user);
        }
    }

    pub fn backspace(&mut self, db: &Database, user: &str) {
        let idx = self.active_index;
        if idx < FIELD_COUNT {
            FIELDS[idx].mask.backspace(&mut self.fields[idx]);
            self.edited(db, user);
        }
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
    }

    /// Handles bracketed paste: file paths on the Attachments row, plain text elsewhere.
    pub fn paste(&mut self, db: &Database, user: &str, text: &str) {
        let idx = self.active_index;
        if idx == ATTACHMENTS && self.pending_attachment.is_none() {
            self.fields[ATTACHMENTS] = text.trim().to_string();
            self.commit_attachment_buffer();
        } else if idx < FIELD_COUNT {
            for ch in text
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            {
                self.type_char(db, user, ch);
            }
        }
    }

    // ----- dropdowns ------------------------------------------------------------------------------------------

    pub fn refresh_dropdown(&mut self, db: &Database, user: &str) {
        self.dropdown = match self.active_index {
            RELATIONS => self.relation_dropdown(db),
            ATTACHMENTS if self.pending_attachment.is_none() => self.path_dropdown(),
            12..=14 => {
                let prefix = self.fields[self.active_index].clone();
                let items = suggest_users(db, &prefix, &[user])
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|u| !u.eq_ignore_ascii_case(prefix.trim().trim_start_matches('@')))
                    .take(MAX_DROPDOWN_ROWS)
                    .map(|u| (format!("@{u}"), Choice::Text(u)))
                    .collect();
                Dropdown::new(items, false)
            }
            _ => None,
        };
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
            _ => true,
        }
    }

    fn accept_choice(&mut self, db: &Database, user: &str, choice: Choice) {
        let idx = self.active_index;
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
            Choice::Item(_) => {}
        }
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

    fn refresh_notes(&mut self, db: &Database, user: &str) {
        let issues = validate_task_draft(db, &self.to_draft(), user);
        for (i, note) in self.notes.iter_mut().enumerate() {
            *note = issues
                .iter()
                .find(|n| n.level == IssueLevel::Info && field_index(n.field) == i)
                .map(|n| n.message.clone());
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
        if idx == RELATIONS {
            self.errors[RELATIONS] = self
                .relation_buffer_error(db, user)
                .or(self.errors[RELATIONS].take());
        }
    }

    /// Validates every field, returning `(field index, message)` for each problem.
    pub fn validate_all(&mut self, db: &Database, user: &str) -> Vec<(usize, String)> {
        let issues = validate_task_draft(db, &self.to_draft(), user);
        let all: Vec<usize> = (0..FIELD_COUNT).collect();
        self.apply(&issues, &all);
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
            12..=14 => "[type] user (new users are created) | [Tab] Next | [Ctrl+S] Submit",
            _ => "[Tab/Up/Down] Field | [Ctrl+S] Submit | [Esc] Cancel | [?] Help & Status",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(mask: Mask, s: &str) -> String {
        let mut v = String::new();
        for ch in s.chars() {
            mask.push(&mut v, ch);
        }
        v
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
}
