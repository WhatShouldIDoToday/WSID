//! Small in-repo form model for the Add Task view: per-field validators (mirroring the shape of
//! `ratatui_form::validation::Validator`), input masks, and debounced error state.
//!
//! Validators delegate to `wsid-core` so the TUI, GUI, CLI, and MCP all enforce identical rules.

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;
use wsid_core::{
    WsidError, normalize_tags, normalize_task_id, normalize_user, parse_deadline_datetime, parse_duration_minutes, validate_dates,
    validate_not_past,
};

use crate::args::parse_relation_arg;

/// How long the active field must be idle before it is validated.
pub const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(400);

/// Validates a single field value. Mirrors `ratatui_form::validation::Validator`.
pub trait Validator: Send + Sync {
    fn validate(&self, value: &str) -> Result<(), String>;
}

fn clean(err: &WsidError) -> String {
    let msg = err.to_string();
    msg.strip_prefix("validation error: ")
        .unwrap_or(&msg)
        .to_string()
}

pub struct Required;
impl Validator for Required {
    fn validate(&self, value: &str) -> Result<(), String> {
        if value.trim().is_empty() { Err("Title is required".to_string()) } else { Ok(()) }
    }
}

/// Free text; always valid.
pub struct Text;
impl Validator for Text {
    fn validate(&self, _value: &str) -> Result<(), String> {
        Ok(())
    }
}

/// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM` that is not in the past.
pub struct DateTime(pub &'static str);
impl Validator for DateTime {
    fn validate(&self, value: &str) -> Result<(), String> {
        let (normalized, _) = parse_deadline_datetime(self.0, value.trim()).map_err(|e| clean(&e))?;
        validate_not_past(self.0, &normalized).map_err(|e| clean(&e))
    }
}

pub struct Estimate;
impl Validator for Estimate {
    fn validate(&self, value: &str) -> Result<(), String> {
        parse_duration_minutes(value)
            .map(|_| ())
            .map_err(|e| clean(&e))
    }
}

pub struct IntRange(pub i64, pub i64);
impl Validator for IntRange {
    fn validate(&self, value: &str) -> Result<(), String> {
        match value.trim().parse::<i64>() {
            Ok(v) if (self.0..=self.1).contains(&v) => Ok(()),
            _ => Err(format!("Must be an integer from {} to {}", self.0, self.1)),
        }
    }
}

pub struct Bool;
impl Validator for Bool {
    fn validate(&self, value: &str) -> Result<(), String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "true" | "false" | "yes" | "no" | "y" | "n" | "1" | "0" => Ok(()),
            _ => Err("Must be true or false".to_string()),
        }
    }
}

fn tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
}

pub struct Tags;
impl Validator for Tags {
    fn validate(&self, value: &str) -> Result<(), String> {
        let list: Vec<String> = tokens(value).map(ToString::to_string).collect();
        normalize_tags(&list).map(|_| ()).map_err(|e| clean(&e))
    }
}

/// `blocks:AB1234`, `sub_task_of:CD5678`, or a bare task ID (related).
pub struct Relations;
impl Validator for Relations {
    fn validate(&self, value: &str) -> Result<(), String> {
        for tok in tokens(value) {
            let (kind, id) = tok.split_once(':').unwrap_or(("related", tok));
            parse_relation_arg(kind).map_err(|e| format!("'{tok}': {}", clean(&e)))?;
            normalize_task_id(id).map_err(|e| format!("'{tok}': {}", clean(&e)))?;
        }
        Ok(())
    }
}

pub struct User;
impl Validator for User {
    fn validate(&self, value: &str) -> Result<(), String> {
        normalize_user(value).map(|_| ()).map_err(|e| clean(&e))
    }
}

pub struct FilePath;
impl Validator for FilePath {
    fn validate(&self, value: &str) -> Result<(), String> {
        if Path::new(value.trim()).is_file() { Ok(()) } else { Err("File not found".to_string()) }
    }
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
    pub validator: &'static dyn Validator,
}

pub const FIELD_COUNT: usize = 16;

pub static FIELDS: [FieldSpec; FIELD_COUNT] = [
    FieldSpec { label: "1. Title *", mask: Mask::None, validator: &Required },
    FieldSpec { label: "2. Description", mask: Mask::None, validator: &Text },
    FieldSpec { label: "3. Due Date (YYYY-MM-DD HH:MM)", mask: Mask::DateTime, validator: &DateTime("due_date") },
    FieldSpec { label: "4. SGDB Date (YYYY-MM-DD HH:MM, <= Due)", mask: Mask::DateTime, validator: &DateTime("sgdb_date") },
    FieldSpec { label: "5. Time Estimate (15m, 2h, 3d, 1w)", mask: Mask::None, validator: &Estimate },
    FieldSpec { label: "6. Difficulty (1-5)", mask: Mask::None, validator: &IntRange(1, 5) },
    FieldSpec { label: "7. Importance (1-5)", mask: Mask::None, validator: &IntRange(1, 5) },
    FieldSpec { label: "8. Manual Priority (1-5)", mask: Mask::None, validator: &IntRange(1, 5) },
    FieldSpec { label: "9. Urgent Override (true/false)", mask: Mask::None, validator: &Bool },
    FieldSpec { label: "10. Tags (single words)", mask: Mask::None, validator: &Tags },
    FieldSpec { label: "11. Relations (e.g. blocks:AB1234)", mask: Mask::None, validator: &Relations },
    FieldSpec { label: "12. Attach File Path", mask: Mask::None, validator: &FilePath },
    FieldSpec { label: "13. Attach Alt Description", mask: Mask::None, validator: &Text },
    FieldSpec { label: "14. Reporter", mask: Mask::None, validator: &User },
    FieldSpec { label: "15. Assignee", mask: Mask::None, validator: &User },
    FieldSpec { label: "16. Evaluator", mask: Mask::None, validator: &User },
];

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

#[derive(Debug, Clone)]
pub struct DataEntryForm {
    pub fields: [String; FIELD_COUNT],
    pub errors: [Option<String>; FIELD_COUNT],
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
        for i in [13, 14, 15] {
            fields[i] = default_user.to_string();
        }
        Self { fields, errors: std::array::from_fn(|_| None), active_index: 0, state, dirty_at: None }
    }

    /// Moves focus, validating the field being left.
    pub fn set_active_index(&mut self, index: usize) {
        if self.active_index < FIELD_COUNT {
            self.validate_field(self.active_index);
        }
        self.dirty_at = None;
        self.active_index = index;
        self.state.select(Some(index));
    }

    pub fn type_char(&mut self, ch: char) {
        let idx = self.active_index;
        if idx < FIELD_COUNT && FIELDS[idx].mask.push(&mut self.fields[idx], ch) {
            self.errors[idx] = None;
            self.dirty_at = Some(Instant::now());
        }
    }

    pub fn backspace(&mut self) {
        let idx = self.active_index;
        if idx < FIELD_COUNT {
            FIELDS[idx].mask.backspace(&mut self.fields[idx]);
            self.errors[idx] = None;
            self.dirty_at = Some(Instant::now());
        }
    }

    /// Validates the active field if it has been idle past the debounce window. Returns true if state changed.
    pub fn validate_if_idle(&mut self, now: Instant) -> bool {
        match self.dirty_at {
            Some(t) if now.duration_since(t) >= VALIDATION_DEBOUNCE && self.active_index < FIELD_COUNT => {
                self.dirty_at = None;
                self.validate_field(self.active_index);
                true
            }
            _ => false,
        }
    }

    /// Validates one field (empty optional fields are always valid) plus the Due/SGDB ordering rule.
    pub fn validate_field(&mut self, idx: usize) {
        let value = self.fields[idx].trim();
        // Title and the 1-5 / boolean fields always need a value; everything else may be left empty.
        let optional = !(idx == 0 || (5..=8).contains(&idx));
        self.errors[idx] = if value.is_empty() && optional { None } else { FIELDS[idx].validator.validate(value).err() };
        if idx == 2 || idx == 3 {
            self.validate_date_order();
        }
    }

    fn validate_date_order(&mut self) {
        let (due, sgdb) = (self.fields[2].trim(), self.fields[3].trim());
        if due.is_empty() || sgdb.is_empty() || self.errors[3].is_some() {
            return;
        }
        let due_ok = parse_deadline_datetime("due_date", due).is_ok();
        let sgdb_ok = parse_deadline_datetime("sgdb_date", sgdb).is_ok();
        if due_ok
            && sgdb_ok
            && let Err(e) = validate_dates(Some(sgdb), Some(due))
        {
            self.errors[3] = Some(clean(&e));
        }
    }

    /// Validates every field, returning `(field index, message)` for each problem.
    pub fn validate_all(&mut self) -> Vec<(usize, String)> {
        for idx in 0..FIELD_COUNT {
            self.validate_field(idx);
        }
        self.dirty_at = None;
        self.errors
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.clone().map(|m| (i, m)))
            .collect()
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
    fn validators_match_core_rules() {
        assert!(DateTime("due_date").validate("2099-01-01").is_ok());
        assert!(DateTime("due_date").validate("2099-01-01 09:30").is_ok());
        assert!(
            DateTime("due_date")
                .validate("2000-01-01")
                .unwrap_err()
                .contains("past")
        );
        assert!(DateTime("due_date").validate("01/22/2000").is_err());
        assert!(
            !DateTime("due_date")
                .validate("01/22/2000")
                .unwrap_err()
                .starts_with("validation error")
        );
        assert!(Estimate.validate("1h30m").is_ok());
        assert!(Estimate.validate("20m 10m").is_err());
        assert!(IntRange(1, 5).validate("5").is_ok());
        assert!(IntRange(1, 5).validate("6").is_err());
        assert!(Tags.validate("a b-c").is_ok());
        assert!(Tags.validate("no!").is_err());
        assert!(Relations.validate("blocks:AB1234 CD5678").is_ok());
        assert!(Relations.validate("bogus:AB1234").is_err());
        assert!(User.validate("two words").is_err());
    }

    #[test]
    fn form_validates_after_debounce_on_blur_and_cross_field() {
        let mut form = DataEntryForm::new("alice");
        form.set_active_index(2);
        for ch in "01222000".chars() {
            form.type_char(ch);
        }
        assert!(form.errors[2].is_none(), "no error before debounce");
        let t0 = form.dirty_at.unwrap();
        assert!(!form.validate_if_idle(t0 + Duration::from_millis(100)));
        assert!(form.validate_if_idle(t0 + VALIDATION_DEBOUNCE));
        assert!(form.errors[2].is_some());

        // Blur validates immediately; SGDB later than Due is flagged on the SGDB field.
        form.fields[2] = "2099-01-01".to_string();
        form.fields[3] = "2099-01-05".to_string();
        form.set_active_index(3);
        form.set_active_index(4);
        assert!(form.errors[3].as_deref().unwrap().contains("on or before"));

        let errs = form.validate_all();
        assert!(errs.iter().any(|(i, _)| *i == 0), "title required");
    }
}
