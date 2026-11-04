//! Text-editing primitives shared by the Add Task form and the TUI's inline prompts:
//! readline-style backward-word deletion plus a coalescing undo/redo stack.
//!
//! Terminals disagree about how they report the editing chords, so [`apply_text_shortcut`]
//! accepts several encodings of the same intent: `Option+Backspace` (macOS) arrives as
//! `Alt+Backspace`, as a `DEL`/`BS` character with `ALT`, or as `Ctrl+W` depending on the
//! emulator, and `Cmd+Z` only reaches us when the terminal forwards `SUPER`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What produced an undo snapshot. Consecutive edits of the same kind collapse into one
/// undo step, so typing `todo` or holding Backspace undoes as a group instead of per key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    /// A jump that must stay its own undo step (completion accept, clear, paste, ...).
    Discrete,
}

/// Number of undo steps kept per field; old snapshots are dropped beyond this.
const MAX_UNDO: usize = 200;

/// Per-buffer undo/redo stack.
#[derive(Debug, Default, Clone)]
pub struct TextHistory {
    undo: Vec<String>,
    redo: Vec<String>,
    last: Option<EditKind>,
}

impl TextHistory {
    /// Records `value` (the text *before* the edit) as an undo point.
    ///
    /// Repeated [`EditKind::Typing`]/[`EditKind::Deleting`] edits extend the current group
    /// instead of pushing a new snapshot.
    pub fn record(&mut self, value: &str, kind: EditKind) {
        self.redo.clear();
        let coalesce = kind != EditKind::Discrete && self.last == Some(kind);
        self.last = Some(kind);
        if coalesce {
            return;
        }
        if self.undo.last().is_some_and(|snapshot| snapshot == value) {
            return;
        }
        self.undo.push(value.to_string());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    /// Ends the current coalescing group, so the next edit starts a fresh undo step.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Drops all undo/redo state, used when a prompt is opened or closed.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last = None;
    }

    /// Restores the previous snapshot; returns false when there is nothing to undo.
    pub fn undo(&mut self, value: &mut String) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::take(value));
        *value = previous;
        self.last = None;
        true
    }

    /// Re-applies an undone snapshot; returns false when there is nothing to redo.
    pub fn redo(&mut self, value: &mut String) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::take(value));
        *value = next;
        self.last = None;
        true
    }
}

/// Collapses pasted line breaks into spaces so multi-line clipboard content cannot break a
/// single-line prompt or form field.
pub fn flatten_paste(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

/// Deletes the word before the end of `value`, along with any whitespace between it and
/// the preceding word (readline's `backward-kill-word`). Returns true when it changed the text.
pub fn delete_word_backward(value: &mut String) -> bool {
    if value.is_empty() {
        return false;
    }
    value.truncate(value.trim_end().len());
    let start = value
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    value.truncate(start);
    true
}

/// Deletes everything before the cursor (the whole buffer, since prompts are append-only).
/// `Cmd+Backspace` on macOS clears a field this way.
pub fn kill_to_start(value: &mut String) -> bool {
    if value.is_empty() {
        return false;
    }
    value.clear();
    true
}

/// Which editing command a key event means, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    WordDelete,
    KillToStart,
    Undo,
    Redo,
}

/// Applies the editing chords shared by every text input.
///
/// Returns `Some(true)` when the key was consumed and changed the text, `Some(false)` when it
/// was consumed without changing anything (undo with an empty history), and `None` when it is
/// not one of these shortcuts and should fall through to plain typing.
pub fn apply_text_shortcut(key: &KeyEvent, value: &mut String, history: &mut TextHistory) -> Option<bool> {
    apply_text_shortcut_with(key, value, history, delete_word_backward)
}

/// [`apply_text_shortcut`] with a caller-supplied backward-word deletion, used by masked
/// fields whose segments are wider than one word (see `form::Mask::DateTime`).
pub fn apply_text_shortcut_with<F>(key: &KeyEvent, value: &mut String, history: &mut TextHistory, word_delete: F) -> Option<bool>
where
    F: Fn(&mut String) -> bool,
{
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // `Cmd` reaches us as `SUPER`; `HYPER`/`META` are what some non-Apple keyboards report.
    let command = key
        .modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    let trigger = match key.code {
        KeyCode::Backspace if command => Trigger::KillToStart,
        KeyCode::Backspace if ctrl || alt => Trigger::WordDelete,
        // `Ctrl+W` is the readline backward-kill-word that macOS Option+Backspace sends.
        KeyCode::Char('w') | KeyCode::Char('W') if ctrl && !alt && !command => Trigger::WordDelete,
        KeyCode::Char('z') | KeyCode::Char('Z') if ctrl || command => {
            if shift {
                Trigger::Redo
            } else {
                Trigger::Undo
            }
        }
        KeyCode::Char('y') | KeyCode::Char('Y') if ctrl || command => Trigger::Redo,
        // Some terminals deliver Option+Backspace as the DEL / BS control character.
        KeyCode::Char('\u{7f}') | KeyCode::Char('\u{8}') if alt || ctrl => Trigger::WordDelete,
        _ => return None,
    };

    if value.is_empty() && matches!(trigger, Trigger::WordDelete | Trigger::KillToStart) {
        return Some(false);
    }
    match trigger {
        Trigger::WordDelete => {
            history.record(value, EditKind::Discrete);
            Some(word_delete(value))
        }
        Trigger::KillToStart => {
            history.record(value, EditKind::Discrete);
            Some(kill_to_start(value))
        }
        Trigger::Undo => Some(history.undo(value)),
        Trigger::Redo => Some(history.redo(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn deletes_back_to_the_previous_separator() {
        let mut value = "Address @davidb note".to_string();
        assert!(delete_word_backward(&mut value));
        assert_eq!(value, "Address @davidb ");
        assert!(delete_word_backward(&mut value));
        assert_eq!(value, "Address ");
        delete_word_backward(&mut value);
        assert_eq!(value, "");
        assert!(!delete_word_backward(&mut value));
    }

    #[test]
    fn deletes_trailing_whitespace_with_the_word() {
        let mut value = "fix the   ".to_string();
        delete_word_backward(&mut value);
        assert_eq!(value, "fix ");
    }

    #[test]
    fn keeps_undo_groups_for_typing_and_deleting() {
        let mut value = String::new();
        let mut history = TextHistory::default();
        for ch in "todo".chars() {
            history.record(&value, EditKind::Typing);
            value.push(ch);
        }
        assert_eq!(value, "todo");
        assert!(history.undo(&mut value));
        assert_eq!(value, "");
        assert!(!history.undo(&mut value));
        assert!(history.redo(&mut value));
        assert_eq!(value, "todo");
    }

    #[test]
    fn word_delete_is_one_undo_step() {
        let mut value = "one two three".to_string();
        let mut history = TextHistory::default();
        assert_eq!(apply_text_shortcut(&key(KeyCode::Backspace, KeyModifiers::ALT), &mut value, &mut history), Some(true));
        assert_eq!(value, "one two ");
        assert!(history.undo(&mut value));
        assert_eq!(value, "one two three");
    }

    #[test]
    fn command_backspace_clears_and_control_z_round_trips() {
        let mut value = "AB1234".to_string();
        let mut history = TextHistory::default();
        assert_eq!(apply_text_shortcut(&key(KeyCode::Backspace, KeyModifiers::SUPER), &mut value, &mut history), Some(true));
        assert_eq!(value, "");
        assert_eq!(apply_text_shortcut(&key(KeyCode::Char('z'), KeyModifiers::CONTROL), &mut value, &mut history), Some(true));
        assert_eq!(value, "AB1234");
        assert_eq!(apply_text_shortcut(&key(KeyCode::Char('z'), KeyModifiers::CONTROL), &mut value, &mut history), Some(false));
    }

    #[test]
    fn plain_characters_and_plain_backspace_fall_through() {
        let mut value = "abc".to_string();
        let mut history = TextHistory::default();
        assert_eq!(apply_text_shortcut(&key(KeyCode::Char('x'), KeyModifiers::NONE), &mut value, &mut history), None);
        assert_eq!(apply_text_shortcut(&key(KeyCode::Backspace, KeyModifiers::NONE), &mut value, &mut history), None);
    }
}
