use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use wsid_core::{
    AppConfig, BulkTaskPatch, Database, DbChangeDetector, EisenhowerClassification, EisenhowerQuadrant, FieldIssue, FocusPreset,
    FocusQueueResponse, SearchHit, SearchQuery, Task, TaskAttachment, TaskNote, TaskStatus, UpdateTaskInput, WsidError, add_task_note,
    attach_existing_file, bulk_delete_tasks, bulk_update_tasks, compute_focus_queue_in_project, create_task_from_draft, delete_task,
    delete_task_note, get_attachment, get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks,
    get_parallel_peers, get_related_tasks, get_task, list_eisenhower_matrix, list_known_users, list_task_notes, list_tasks, normalize_user,
    open_file_in_os, parse_note_tokens, register_user, remove_attachment, save_clipboard_attachment, search_relation_targets, search_tasks,
    suggest_users, transition_task_status, unblock_or_dismiss_subtask, update_attachment_description, update_task,
};

use crate::cli_runner::read_system_clipboard_attachment;
pub use crate::tui::form::DataEntryForm;
use crate::tui::form::{self, CommentSuggestion, CommentTokenKind, trailing_comment_token};
use crate::tui::text_edit::{self, EditKind, TextHistory, apply_text_shortcut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Focus,
    Kanban,
    Eisenhower,
    Database,
    DataEntry,
}

impl ActiveTab {
    pub const ALL: [Self; 5] = [Self::Focus, Self::Kanban, Self::Eisenhower, Self::Database, Self::DataEntry];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Kanban => "Kanban",
            Self::Eisenhower => "Eisenhower",
            Self::Database => "Database",
            Self::DataEntry => "Add Task",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Focus => 0,
            Self::Kanban => 1,
            Self::Eisenhower => 2,
            Self::Database => 3,
            Self::DataEntry => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseSortColumn {
    Relevance,
    Id,
    Title,
    Status,
    Project,
    DueDate,
    SgdbDate,
    Importance,
    Difficulty,
    Assignee,
}

impl DatabaseSortColumn {
    /// Cycles in the same left-to-right order as the results table, so `o`
    /// always moves to the next visible column.
    pub const fn next(self) -> Self {
        match self {
            Self::Relevance => Self::Id,
            Self::Id => Self::Title,
            Self::Title => Self::Status,
            Self::Status => Self::Project,
            Self::Project => Self::Assignee,
            Self::Assignee => Self::Importance,
            Self::Importance => Self::Difficulty,
            Self::Difficulty => Self::SgdbDate,
            Self::SgdbDate => Self::DueDate,
            Self::DueDate => Self::Relevance,
        }
    }

    /// Filter-token field name used when `/` prefills from the highlighted column.
    pub const fn filter_field(self) -> Option<&'static str> {
        match self {
            Self::Relevance => None,
            Self::Id => Some("id"),
            Self::Title => Some("title"),
            Self::Status => Some("status"),
            Self::Project => Some("project"),
            Self::Assignee => Some("assignee"),
            Self::Importance => Some("imp"),
            Self::Difficulty => Some("diff"),
            Self::SgdbDate => Some("sgdb"),
            Self::DueDate => Some("due"),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Score/Created",
            Self::Id => "ID",
            Self::Title => "Title",
            Self::Status => "Status",
            Self::Project => "Project",
            Self::DueDate => "Due",
            Self::SgdbDate => "SGDB",
            Self::Importance => "Importance",
            Self::Difficulty => "Difficulty",
            Self::Assignee => "Assignee",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskDetailSnapshot {
    pub task: Task,
    pub blockers: Vec<Task>,
    pub blocking: Vec<Task>,
    pub parents: Vec<Task>,
    pub subtasks: Vec<Task>,
    pub parallel: Vec<Task>,
    pub related: Vec<Task>,
    pub notes: Vec<TaskNote>,
    pub scroll: u16,
}

impl TaskDetailSnapshot {
    pub fn all_attachments(&self) -> Vec<TaskAttachment> {
        self.task
            .attachments
            .iter()
            .chain(self.notes.iter().flat_map(|n| n.attachments.iter()))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailPromptKind {
    None,
    AddComment,
    AttachFile,
    AttachClipboard,
    EditAttachmentDesc,
    JumpToTask,
    ConfirmDeleteTask,
    ConfirmDeleteNote,
}

/// What currently owns the keyboard.
///
/// Every key press is routed to exactly one context, and the footer draws the
/// hints for that same context, so a modal never shows the bindings of the view
/// underneath it and never lists its own keys twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusContext {
    FormErrorModal,
    AboutModal,
    ProjectModal,
    /// The user filter modal (per-view user scope) owns input while it is open.
    UserModal,
    /// A prompt inside the task detail modal (comment, attach, jump, delete...).
    DetailPrompt(DetailPromptKind),
    /// The attachment list inside the task detail modal has focus.
    DetailAttachments,
    /// The task detail modal itself, with no prompt open.
    DetailModal,
    SearchInput,
    UserSwitchPrompt,
    BulkEditPrompt,
    Tab(ActiveTab),
}

/// Modifiers that turn a keystroke into a shortcut or a terminal/OS action
/// (`Ctrl+V` paste, `Alt+F`, `Cmd+K`, ...) rather than text.
const SHORTCUT_MODIFIERS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// True when the event is a printable character that may be inserted into a text
/// field: unmodified, or Shift-only.
fn is_text_input(key: &KeyEvent) -> bool {
    !key.modifiers.intersects(SHORTCUT_MODIFIERS) && matches!(key.code, KeyCode::Char(ch) if !ch.is_control())
}

/// True when a `Char` event carries shortcut modifiers, so it must neither be
/// typed nor trigger a single-letter action.
fn is_modified_char(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_)) && !is_text_input(key)
}

pub struct TuiApp {
    pub db: Database,
    pub db_path: PathBuf,
    pub config: AppConfig,
    pub user: String,
    pub preset: FocusPreset,
    pub accessible_mode: bool,
    pub should_quit: bool,
    pub active_tab: ActiveTab,
    pub status_message: String,

    pub change_detector: DbChangeDetector,

    // Focus View state
    pub focus_queue: FocusQueueResponse,
    pub focus_selected: usize,
    pub focus_state: ListState,
    pub show_blocked_banner: bool,

    // Kanban View state
    pub all_tasks: Vec<Task>,
    pub kanban_col: usize,
    pub kanban_row: usize,
    pub kanban_states: [ListState; 6],
    /// Project shown by the Kanban board (`all` = every project).
    pub kanban_project: String,

    // Focus View project scope (`all` by default, unlike Kanban/Eisenhower)
    pub focus_project: String,

    // Eisenhower View state
    pub eisenhower_items: Vec<EisenhowerClassification>,
    pub eisenhower_quadrant: usize,
    pub eisenhower_row: usize,
    pub eisenhower_states: [ListState; 4],
    /// Project shown by the Eisenhower matrix (`all` = every project).
    pub eisenhower_project: String,

    // Project filter modal (shared by Focus, Kanban, Eisenhower, and Database)
    pub project_modal_open: bool,
    pub project_modal_query: String,
    pub project_modal_selected: usize,

    // User filter modal (shared by Focus, Kanban, Eisenhower, and Database)
    pub user_modal_open: bool,
    pub user_modal_query: String,
    pub user_modal_selected: usize,

    // Per-view user scope: Focus names one user, the other views also allow `all`.
    pub focus_user: String,
    pub kanban_user: String,
    pub eisenhower_user: String,

    // Database Search View state
    pub search_query: String,
    pub search_editing: bool,
    pub search_status_filter: Option<TaskStatus>,
    pub search_only_mine: bool,
    pub search_tag_filter: Option<String>,
    pub search_assignee_filter: Option<String>,
    pub search_project_filter: Option<String>,
    pub search_sort_col: DatabaseSortColumn,
    pub search_sort_asc: bool,
    pub search_hits: Vec<SearchHit>,
    pub search_selected: usize,
    pub search_table_state: TableState,
    pub search_list_state: ListState,

    // Multi-select + bulk edit (Kanban, Eisenhower, Database)
    pub selected_ids: Vec<String>,
    pub bulk_editing: bool,
    pub bulk_input: String,

    // Data Entry View state
    pub entry_form: DataEntryForm,

    // Task Detail Drawer state
    pub detail_modal: Option<TaskDetailSnapshot>,
    pub detail_prompt_kind: DetailPromptKind,
    pub detail_prompt_input: String,
    /// `@` / `#` completions for the active comment prompt.
    pub detail_suggestions: Vec<CommentSuggestion>,
    pub detail_suggestion_index: usize,
    /// Set once the user navigates the list, so Enter completes instead of submitting.
    pub detail_suggestion_navigated: bool,
    /// Resolved mentions / task links shown under the suggestion list.
    pub detail_comment_preview: Vec<String>,
    pub detail_note_cursor: usize,
    pub attachment_focused: bool,
    pub attachment_cursor: usize,
    pub attachment_marked: BTreeSet<i64>,

    /// Validation problems shown in a modal after a failed submit: `(field index, message)`.
    pub form_errors: Option<Vec<(Option<usize>, String)>>,

    // Active-user switching prompt
    pub user_switching: bool,
    pub user_input: String,

    // About & Third-Party Licenses Modal state
    pub about_modal_open: bool,
    pub about_scroll: u16,

    // Edit-in-form state: set while the Add Task form is editing an existing task
    pub editing_task_id: Option<String>,
    pub editing_return_tab: Option<ActiveTab>,

    // Undo/redo stacks for the inline prompts (`Ctrl+Z` / `Cmd+Z`)
    project_modal_history: TextHistory,
    user_modal_history: TextHistory,
    search_history: TextHistory,
    user_history: TextHistory,
    bulk_history: TextHistory,
    detail_prompt_history: TextHistory,
}

impl TuiApp {
    pub fn new(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<Self> {
        let db = Database::open(db_path)?;
        let change_detector = DbChangeDetector::new(&db)?;
        let preset = config.focus_preset;
        let today = Utc::now().date_naive();
        let focus_queue = compute_focus_queue_in_project(
            &db,
            &user,
            preset,
            config.custom_weights,
            Some(config.focus_limit),
            today,
            config.scoring,
            Some(wsid_core::ALL_PROJECTS),
        )?;
        let all_tasks = list_tasks(&db)?;
        let eisenhower_items = list_eisenhower_matrix(&db, Some(&user), today, config.scoring)?;
        let search_hits = search_tasks(&db, &SearchQuery::default())?;
        let entry_form = DataEntryForm::new(&user);

        let mut app = Self {
            db,
            db_path: db_path.to_path_buf(),
            config,
            user: user.clone(),
            preset,
            accessible_mode,
            should_quit: false,
            active_tab: ActiveTab::Focus,
            status_message: String::new(),
            change_detector,
            focus_queue,
            focus_selected: 0,
            focus_state: ListState::default(),
            show_blocked_banner: true,
            all_tasks,
            kanban_col: 1, // Todo column by default
            kanban_row: 0,
            kanban_states: std::array::from_fn(|_| ListState::default()),
            kanban_project: wsid_core::DEFAULT_PROJECT.to_string(),
            focus_project: wsid_core::ALL_PROJECTS.to_string(),
            eisenhower_items,
            eisenhower_quadrant: 0,
            eisenhower_row: 0,
            eisenhower_states: std::array::from_fn(|_| ListState::default()),
            eisenhower_project: wsid_core::DEFAULT_PROJECT.to_string(),
            project_modal_open: false,
            project_modal_query: String::new(),
            project_modal_selected: 0,
            user_modal_open: false,
            user_modal_query: String::new(),
            user_modal_selected: 0,
            focus_user: user.clone(),
            kanban_user: user.clone(),
            eisenhower_user: user.clone(),
            search_query: String::new(),
            search_editing: false,
            search_status_filter: None,
            search_only_mine: false,
            search_tag_filter: None,
            search_assignee_filter: None,
            search_project_filter: None,
            search_sort_col: DatabaseSortColumn::Relevance,
            search_sort_asc: true,
            search_hits,
            search_selected: 0,
            search_table_state: TableState::default(),
            search_list_state: ListState::default(),
            selected_ids: Vec::new(),
            bulk_editing: false,
            bulk_input: String::new(),
            entry_form,
            detail_modal: None,
            detail_prompt_kind: DetailPromptKind::None,
            detail_prompt_input: String::new(),
            detail_suggestions: Vec::new(),
            detail_suggestion_index: 0,
            detail_suggestion_navigated: false,
            detail_comment_preview: Vec::new(),
            detail_note_cursor: 0,
            attachment_focused: false,
            attachment_cursor: 0,
            attachment_marked: BTreeSet::new(),
            form_errors: None,
            user_switching: false,
            user_input: String::new(),
            about_modal_open: false,
            about_scroll: 0,
            editing_task_id: None,
            editing_return_tab: None,
            project_modal_history: TextHistory::default(),
            user_modal_history: TextHistory::default(),
            search_history: TextHistory::default(),
            user_history: TextHistory::default(),
            bulk_history: TextHistory::default(),
            detail_prompt_history: TextHistory::default(),
        };
        app.refresh_all()?;
        Ok(app)
    }

    /// Polls `PRAGMA data_version` on the SQLite database and reloads state if an external
    /// process (e.g. MCP server or CLI) committed changes.
    pub fn on_tick(&mut self) -> wsid_core::Result<bool> {
        self.entry_form
            .validate_if_idle(&self.db, &self.user, std::time::Instant::now());
        if self.change_detector.has_external_changes(&self.db)? {
            self.refresh_all()?;
            self.status_message = "Live reload: detected external database update".to_string();
            return Ok(true);
        }
        Ok(false)
    }

    pub fn refresh_all(&mut self) -> wsid_core::Result<()> {
        let today = Utc::now().date_naive();
        let custom_weights = if self.preset == self.config.focus_preset { self.config.custom_weights } else { None };
        self.focus_queue = compute_focus_queue_in_project(
            &self.db,
            &self.focus_user,
            self.preset,
            custom_weights,
            Some(self.config.focus_limit),
            today,
            self.config.scoring,
            Some(self.focus_project.as_str()),
        )?;
        if self.focus_queue.actionable_tasks.is_empty() {
            self.focus_selected = 0;
            self.focus_state.select(None);
        } else {
            if self.focus_selected >= self.focus_queue.actionable_tasks.len() {
                self.focus_selected = self.focus_queue.actionable_tasks.len() - 1;
            }
            self.focus_state.select(Some(self.focus_selected));
        }

        self.all_tasks = list_tasks(&self.db)?;
        {
            let existing: std::collections::HashSet<&str> = self.all_tasks.iter().map(|t| t.id.as_str()).collect();
            self.selected_ids
                .retain(|id| existing.contains(id.as_str()));
        }
        self.clamp_kanban_selection();

        self.eisenhower_items = list_eisenhower_matrix(&self.db, Some(&self.eisenhower_user), today, self.config.scoring)?;
        self.clamp_eisenhower_selection();

        self.refresh_search()?;

        if let Some(ref current_detail) = self.detail_modal.clone() {
            if let Ok(updated_snapshot) = self.load_detail_snapshot(&current_detail.task.id, current_detail.scroll) {
                let atts = updated_snapshot.all_attachments();
                let valid_ids: BTreeSet<i64> = atts.iter().map(|a| a.id).collect();
                self.attachment_marked.retain(|id| valid_ids.contains(id));
                if atts.is_empty() {
                    self.attachment_cursor = 0;
                    self.attachment_focused = false;
                } else {
                    self.attachment_cursor = self.attachment_cursor.min(atts.len() - 1);
                }
                self.detail_modal = Some(updated_snapshot);
            } else {
                self.detail_modal = None;
                self.attachment_focused = false;
                self.attachment_cursor = 0;
                self.attachment_marked.clear();
            }
        }

        let _ = self.change_detector.has_external_changes(&self.db);
        Ok(())
    }

    pub fn refresh_search(&mut self) -> wsid_core::Result<()> {
        let assignee = if self.search_only_mine { Some(self.user.clone()) } else { self.search_assignee_filter.clone() };
        let mut hits = search_tasks(
            &self.db,
            &SearchQuery {
                query: self.search_query.clone(),
                status: self.search_status_filter,
                tag: self.search_tag_filter.clone(),
                assignee,
                reporter: None,
                evaluator: None,
                project: self.search_project_filter.clone(),
                limit: None,
            },
        )?;

        hits.sort_by(|a, b| {
            let ord = match self.search_sort_col {
                DatabaseSortColumn::Relevance => b
                    .score
                    .cmp(&a.score)
                    .then_with(|| a.task.id.cmp(&b.task.id)),
                DatabaseSortColumn::Id => a.task.id.cmp(&b.task.id),
                DatabaseSortColumn::Title => a
                    .task
                    .title
                    .to_lowercase()
                    .cmp(&b.task.title.to_lowercase()),
                DatabaseSortColumn::Status => a.task.status.as_str().cmp(b.task.status.as_str()),
                DatabaseSortColumn::Project => a.task.project.cmp(&b.task.project),
                DatabaseSortColumn::DueDate => a.task.due_date.cmp(&b.task.due_date),
                DatabaseSortColumn::SgdbDate => a.task.sgdb_date.cmp(&b.task.sgdb_date),
                DatabaseSortColumn::Importance => b.task.importance.cmp(&a.task.importance),
                DatabaseSortColumn::Difficulty => b.task.difficulty.cmp(&a.task.difficulty),
                DatabaseSortColumn::Assignee => a.task.assignee.cmp(&b.task.assignee),
            };
            if self.search_sort_asc { ord } else { ord.reverse() }
        });

        self.search_hits = hits;
        if self.search_hits.is_empty() {
            self.search_selected = 0;
            self.search_table_state.select(None);
            self.search_list_state.select(None);
        } else {
            if self.search_selected >= self.search_hits.len() {
                self.search_selected = self.search_hits.len() - 1;
            }
            self.search_table_state.select(Some(self.search_selected));
            self.search_list_state.select(Some(self.search_selected));
        }
        Ok(())
    }

    fn load_detail_snapshot(&self, task_id: &str, scroll: u16) -> wsid_core::Result<TaskDetailSnapshot> {
        let task = get_task(&self.db, task_id)?;
        let blockers = get_direct_blockers(&self.db, &task.id)?;
        let blocking = get_direct_blocked_tasks(&self.db, &task.id)?;
        let parents = get_direct_parents(&self.db, &task.id)?;
        let subtasks = get_direct_subtasks(&self.db, &task.id)?;
        let parallel = get_parallel_peers(&self.db, &task.id)?;
        let related = get_related_tasks(&self.db, &task.id)?;
        let notes = list_task_notes(&self.db, &task.id)?;

        Ok(TaskDetailSnapshot { task, blockers, blocking, parents, subtasks, parallel, related, notes, scroll })
    }

    pub fn open_detail_for(&mut self, task_id: &str) {
        match self.load_detail_snapshot(task_id, 0) {
            Ok(snapshot) => {
                self.status_message = format!("Inspecting [{}] {}", snapshot.task.id, snapshot.task.title);
                self.detail_modal = Some(snapshot);
                self.detail_note_cursor = 0;
                self.attachment_focused = false;
                self.attachment_cursor = 0;
                self.attachment_marked.clear();
                self.detail_prompt_kind = DetailPromptKind::None;
                self.detail_prompt_input.clear();
                self.clear_comment_suggestions();
            }
            Err(err) => {
                self.status_message = format!("Error loading task {task_id}: {err}");
            }
        }
    }

    /// The single authority for both key routing and footer hints.
    pub fn focus_context(&self) -> FocusContext {
        if self.form_errors.is_some() {
            FocusContext::FormErrorModal
        } else if self.about_modal_open {
            FocusContext::AboutModal
        } else if self.project_modal_open {
            FocusContext::ProjectModal
        } else if self.user_modal_open {
            FocusContext::UserModal
        } else if self.detail_modal.is_some() {
            if self.detail_prompt_kind != DetailPromptKind::None {
                FocusContext::DetailPrompt(self.detail_prompt_kind)
            } else if self.attachment_focused {
                FocusContext::DetailAttachments
            } else {
                FocusContext::DetailModal
            }
        } else if self.active_tab == ActiveTab::Database && self.search_editing {
            FocusContext::SearchInput
        } else if self.user_switching {
            FocusContext::UserSwitchPrompt
        } else if self.bulk_editing {
            FocusContext::BulkEditPrompt
        } else {
            FocusContext::Tab(self.active_tab)
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global Ctrl+C always exits cleanly
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match self.focus_context() {
            FocusContext::FormErrorModal => self.dismiss_form_errors(key),
            FocusContext::AboutModal => self.handle_about_modal_key(key),
            FocusContext::ProjectModal => self.handle_project_modal_key(key),
            FocusContext::UserModal => self.handle_user_modal_key(key),
            FocusContext::DetailPrompt(_) | FocusContext::DetailAttachments | FocusContext::DetailModal => {
                self.handle_detail_modal_key(key);
            }
            FocusContext::SearchInput => self.handle_search_input_key(key),
            FocusContext::UserSwitchPrompt => self.handle_user_input_key(key),
            FocusContext::BulkEditPrompt => self.handle_bulk_input_key(key),
            FocusContext::Tab(ActiveTab::DataEntry) => self.handle_entry_tab_key(key),
            FocusContext::Tab(_) => self.handle_list_tab_key(key),
        }
    }

    /// The validation modal swallows input until it is dismissed; Enter/Esc/q
    /// jump straight to the first field that needs fixing.
    fn dismiss_form_errors(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
            let first = self
                .form_errors
                .take()
                .and_then(|errs| errs.into_iter().find_map(|(i, _)| i));
            if let Some(i) = first {
                self.entry_form.set_active_index(&self.db, &self.user, i);
            }
        }
    }

    fn handle_entry_tab_key(&mut self, key: KeyEvent) {
        // Esc leaves the tab only when no dropdown wants it first.
        if key.code == KeyCode::Esc && !self.entry_form.wants_escape() {
            if self.editing_task_id.is_some() {
                self.cancel_edit_task();
            } else {
                self.active_tab = ActiveTab::Focus;
                self.status_message = "Switched to Focus View".to_string();
            }
            return;
        }
        self.handle_data_entry_key(key);
    }

    /// Shortcuts shared by every list view, then the active view's own keys.
    fn handle_list_tab_key(&mut self, key: KeyEvent) {
        // `Ctrl+U` switches the acting user; `U` opens the view's user filter.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.begin_user_switch();
            return;
        }
        // Modified characters belong to the terminal/OS, so they never fire
        // single-letter shortcuts (e.g. Ctrl+A does not toggle accessible mode).
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('a') => {
                self.accessible_mode = !self.accessible_mode;
                self.status_message = format!("Accessible linear layout mode: {}", if self.accessible_mode { "ON" } else { "OFF" });
                return;
            }
            KeyCode::Char('1') => {
                self.active_tab = ActiveTab::Focus;
                self.status_message.clear();
                return;
            }
            KeyCode::Char('2') => {
                self.active_tab = ActiveTab::Kanban;
                self.status_message.clear();
                return;
            }
            KeyCode::Char('3') => {
                self.active_tab = ActiveTab::Eisenhower;
                self.status_message.clear();
                return;
            }
            KeyCode::Char('4') => {
                self.active_tab = ActiveTab::Database;
                self.status_message.clear();
                return;
            }
            KeyCode::Char('5') | KeyCode::Char('n') => {
                self.active_tab = ActiveTab::DataEntry;
                self.status_message.clear();
                return;
            }
            KeyCode::Char('U') => {
                self.open_user_modal();
                return;
            }
            KeyCode::Char('r') => {
                if let Err(err) = self.refresh_all() {
                    self.status_message = format!("Refresh failed: {err}");
                } else {
                    self.status_message = "Refreshed database state".to_string();
                }
                return;
            }
            KeyCode::Char('P') => {
                self.open_project_modal();
                return;
            }
            KeyCode::Char('?') => {
                self.about_modal_open = true;
                self.about_scroll = 0;
                self.status_message = "About WSID & Third-Party Licenses".to_string();
                return;
            }
            _ => {}
        }

        if matches!(self.active_tab, ActiveTab::Kanban | ActiveTab::Eisenhower | ActiveTab::Database) && self.handle_selection_key(key) {
            return;
        }

        match self.active_tab {
            ActiveTab::Focus => self.handle_focus_key(key),
            ActiveTab::Kanban => self.handle_kanban_key(key),
            ActiveTab::Eisenhower => self.handle_eisenhower_key(key),
            ActiveTab::Database => self.handle_database_key(key),
            ActiveTab::DataEntry => {}
        }
    }

    fn handle_focus_key(&mut self, key: KeyEvent) {
        let len = self.focus_queue.actionable_tasks.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.focus_selected = (self.focus_selected + 1) % len;
                    self.focus_state.select(Some(self.focus_selected));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    self.focus_selected = self.focus_selected.checked_sub(1).unwrap_or(len - 1);
                    self.focus_state.select(Some(self.focus_selected));
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if len > 0 {
                    self.focus_selected = 0;
                    self.focus_state.select(Some(0));
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if len > 0 {
                    self.focus_selected = len - 1;
                    self.focus_state.select(Some(self.focus_selected));
                }
            }
            KeyCode::Char('b') => {
                self.show_blocked_banner = !self.show_blocked_banner;
                self.status_message = format!("Blocked summary banner: {}", if self.show_blocked_banner { "shown" } else { "hidden" });
            }
            KeyCode::Char('p') => {
                self.preset = match self.preset {
                    FocusPreset::Balanced => FocusPreset::QuickWins,
                    FocusPreset::QuickWins => FocusPreset::EatTheFrog,
                    FocusPreset::EatTheFrog => FocusPreset::UnblockOthers,
                    FocusPreset::UnblockOthers => FocusPreset::Balanced,
                };
                let _ = self.refresh_all();
                self.status_message = format!("Switched Focus preset to '{}'", self.preset);
            }
            KeyCode::Char('s') => {
                if let Some(item) = self.focus_queue.actionable_tasks.get(self.focus_selected) {
                    let id = item.task.id.clone();
                    self.apply_status_transition(&id, TaskStatus::InProgress);
                }
            }
            KeyCode::Char('d') => {
                if let Some(item) = self.focus_queue.actionable_tasks.get(self.focus_selected) {
                    let id = item.task.id.clone();
                    self.apply_status_transition(&id, TaskStatus::Completed);
                }
            }
            KeyCode::Char('x') => {
                if let Some(item) = self.focus_queue.actionable_tasks.get(self.focus_selected) {
                    let id = item.task.id.clone();
                    self.apply_status_transition(&id, TaskStatus::Stuck);
                }
            }
            KeyCode::Enter => {
                if let Some(item) = self.focus_queue.actionable_tasks.get(self.focus_selected) {
                    let id = item.task.id.clone();
                    self.open_detail_for(&id);
                }
            }
            _ => {}
        }
    }

    pub fn kanban_tasks_in_col(&self, col_idx: usize) -> Vec<&Task> {
        let status = TaskStatus::ALL[col_idx.min(TaskStatus::ALL.len() - 1)];
        self.all_tasks
            .iter()
            .filter(|t| t.status == status)
            .filter(|t| self.matches_project(&self.kanban_project, &t.project))
            .filter(|t| self.matches_user(&self.kanban_user, &t.assignee))
            .collect()
    }

    pub(crate) fn clamp_kanban_selection(&mut self) {
        for col_idx in 0..TaskStatus::ALL.len() {
            let col_len = self.kanban_tasks_in_col(col_idx).len();
            let current = self.kanban_states[col_idx].selected().unwrap_or(0);
            if col_len == 0 {
                self.kanban_states[col_idx].select(None);
            } else if current >= col_len {
                self.kanban_states[col_idx].select(Some(col_len - 1));
            } else {
                self.kanban_states[col_idx].select(Some(current));
            }
        }
        let col_tasks = self.kanban_tasks_in_col(self.kanban_col);
        if col_tasks.is_empty() {
            self.kanban_row = 0;
        } else {
            let row = self.kanban_states[self.kanban_col]
                .selected()
                .unwrap_or(self.kanban_row)
                .min(col_tasks.len() - 1);
            self.kanban_row = row;
            self.kanban_states[self.kanban_col].select(Some(row));
        }
    }

    fn handle_kanban_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.kanban_col = self
                    .kanban_col
                    .checked_sub(1)
                    .unwrap_or(TaskStatus::ALL.len() - 1);
                self.kanban_row = self.kanban_states[self.kanban_col].selected().unwrap_or(0);
                self.clamp_kanban_selection();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.kanban_col = (self.kanban_col + 1) % TaskStatus::ALL.len();
                self.kanban_row = self.kanban_states[self.kanban_col].selected().unwrap_or(0);
                self.clamp_kanban_selection();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.kanban_tasks_in_col(self.kanban_col).len();
                if len > 0 {
                    self.kanban_row = (self.kanban_row + 1) % len;
                    self.kanban_states[self.kanban_col].select(Some(self.kanban_row));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.kanban_tasks_in_col(self.kanban_col).len();
                if len > 0 {
                    self.kanban_row = self.kanban_row.checked_sub(1).unwrap_or(len - 1);
                    self.kanban_states[self.kanban_col].select(Some(self.kanban_row));
                }
            }
            KeyCode::Char('<') | KeyCode::Char('H') => {
                if self.kanban_col > 0 {
                    self.transition_targets(TaskStatus::ALL[self.kanban_col - 1]);
                }
            }
            KeyCode::Char('>') | KeyCode::Char('L') => {
                if self.kanban_col + 1 < TaskStatus::ALL.len() {
                    self.transition_targets(TaskStatus::ALL[self.kanban_col + 1]);
                }
            }
            KeyCode::Enter => {
                let col_tasks = self.kanban_tasks_in_col(self.kanban_col);
                if let Some(task) = col_tasks.get(self.kanban_row) {
                    let id = task.id.clone();
                    self.open_detail_for(&id);
                }
            }
            _ => {}
        }
    }

    pub fn eisenhower_tasks_in_quadrant(&self, q_idx: usize) -> Vec<&EisenhowerClassification> {
        let quadrant = match q_idx {
            0 => EisenhowerQuadrant::Q1DoFirst,
            1 => EisenhowerQuadrant::Q2Schedule,
            2 => EisenhowerQuadrant::Q3DelegateQuick,
            _ => EisenhowerQuadrant::Q4Later,
        };
        self.eisenhower_items
            .iter()
            .filter(|i| i.quadrant == quadrant)
            .filter(|i| self.matches_project(&self.eisenhower_project, &i.task.project))
            .collect()
    }

    /// Whether a task's project passes the active filter (`all` / empty = every project).
    pub fn matches_project(&self, filter: &str, project: &str) -> bool {
        filter.is_empty() || filter == wsid_core::ALL_PROJECTS || filter == project
    }

    /// Whether an assignee passes a user filter (`all` / empty = every user).
    pub fn matches_user(&self, filter: &str, user: &str) -> bool {
        let filter = filter.trim();
        filter.is_empty() || filter.eq_ignore_ascii_case(wsid_core::ALL_USERS) || filter.eq_ignore_ascii_case(user)
    }

    /// Project filter currently applied by the active view (`all` = every project).
    pub fn current_project_filter(&self) -> String {
        match self.active_tab {
            ActiveTab::Focus => self.focus_project.clone(),
            ActiveTab::Kanban => self.kanban_project.clone(),
            ActiveTab::Eisenhower => self.eisenhower_project.clone(),
            ActiveTab::Database => self
                .search_project_filter
                .clone()
                .unwrap_or_else(|| wsid_core::ALL_PROJECTS.to_string()),
            ActiveTab::DataEntry => wsid_core::ALL_PROJECTS.to_string(),
        }
    }

    /// Every project that can be filtered to, `all` first.
    pub fn project_filter_options(&self) -> Vec<(String, String)> {
        let mut options = vec![(wsid_core::ALL_PROJECTS.to_string(), format!("{} (every project)", wsid_core::ALL_PROJECTS))];
        let mut projects = wsid_core::list_known_projects(&self.db).unwrap_or_default();
        projects.sort();
        projects.dedup();
        for project in projects {
            if project != wsid_core::ALL_PROJECTS {
                options.push((project.clone(), project));
            }
        }
        options
    }

    /// `project_filter_options` narrowed by the modal's live search text.
    pub(crate) fn filtered_project_options(&self) -> Vec<(String, String)> {
        let needle = self.project_modal_query.trim().to_ascii_lowercase();
        self.project_filter_options()
            .into_iter()
            .filter(|(value, label)| {
                needle.is_empty() || value.to_ascii_lowercase().contains(&needle) || label.to_ascii_lowercase().contains(&needle)
            })
            .collect()
    }

    fn open_project_modal(&mut self) {
        self.project_modal_open = true;
        self.project_modal_query.clear();
        self.project_modal_history.clear();
        let current = self.current_project_filter();
        let options = self.filtered_project_options();
        self.project_modal_selected = options
            .iter()
            .position(|(value, _)| *value == current)
            .unwrap_or(0);
        self.status_message = "Project filter — type to search, Up/Down to move, Enter to apply, Esc to cancel".to_string();
    }

    fn apply_project_filter(&mut self, project: String) {
        match self.active_tab {
            ActiveTab::Focus => {
                self.focus_project = project.clone();
                let _ = self.refresh_all();
            }
            ActiveTab::Kanban => {
                self.kanban_project = project.clone();
                self.clamp_kanban_selection();
            }
            ActiveTab::Eisenhower => {
                self.eisenhower_project = project.clone();
                self.clamp_eisenhower_selection();
            }
            ActiveTab::Database => {
                self.search_project_filter = if project == wsid_core::ALL_PROJECTS { None } else { Some(project.clone()) };
                let _ = self.refresh_search();
            }
            ActiveTab::DataEntry => {}
        }
        self.status_message = format!("Project filter: {project}");
    }

    fn handle_project_modal_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.project_modal_query, &mut self.project_modal_history) {
            if changed {
                self.project_modal_selected = 0;
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.project_modal_open = false;
                self.status_message = "Cancelled project filter".to_string();
            }
            KeyCode::Enter => {
                if let Some((project, _)) = self
                    .filtered_project_options()
                    .get(self.project_modal_selected)
                    .cloned()
                {
                    self.project_modal_open = false;
                    self.apply_project_filter(project);
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                let len = self.filtered_project_options().len();
                if len > 0 {
                    self.project_modal_selected = (self.project_modal_selected + 1) % len;
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                let len = self.filtered_project_options().len();
                if len > 0 {
                    self.project_modal_selected = self
                        .project_modal_selected
                        .checked_sub(1)
                        .unwrap_or(len - 1);
                }
            }
            KeyCode::Backspace => {
                self.project_modal_query.pop();
                self.project_modal_selected = 0;
            }
            KeyCode::Char(ch) => {
                self.project_modal_query.push(ch);
                self.project_modal_selected = 0;
            }
            _ => {}
        }
    }

    /// User scope currently applied by the active view (`all` = every user).
    pub fn current_user_filter(&self) -> String {
        match self.active_tab {
            ActiveTab::Focus => self.focus_user.clone(),
            ActiveTab::Kanban => self.kanban_user.clone(),
            ActiveTab::Eisenhower => self.eisenhower_user.clone(),
            ActiveTab::Database => self
                .search_assignee_filter
                .clone()
                .unwrap_or_else(|| wsid_core::ALL_USERS.to_string()),
            ActiveTab::DataEntry => self.user.clone(),
        }
    }

    /// Focus shows one user's queue, so `all` is not offered there (nor on the form).
    pub fn user_filter_allows_all(&self) -> bool {
        matches!(self.active_tab, ActiveTab::Kanban | ActiveTab::Eisenhower | ActiveTab::Database)
    }

    /// Every user that can be filtered to: `all` first when the view allows it, then known users.
    pub fn user_filter_options(&self) -> Vec<(String, String)> {
        let mut options = Vec::new();
        if self.user_filter_allows_all() {
            options.push((wsid_core::ALL_USERS.to_string(), format!("{} (every user)", wsid_core::ALL_USERS)));
        }
        let mut users = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()]).unwrap_or_default();
        users.sort();
        users.dedup();
        for user in users {
            if user == wsid_core::ALL_USERS {
                continue;
            }
            let label = if user == self.user { format!("@{user} (active)") } else { format!("@{user}") };
            options.push((user, label));
        }
        let current = self.current_user_filter();
        if !current.is_empty()
            && !options
                .iter()
                .any(|(value, _)| value.eq_ignore_ascii_case(&current))
        {
            options.push((current.clone(), format!("@{current}")));
        }
        options
    }

    /// `user_filter_options` narrowed by the modal's live search text. A valid user name that is
    /// not known yet is offered too, so `@someone` can be filtered to before their first task.
    pub(crate) fn filtered_user_options(&self) -> Vec<(String, String)> {
        let needle = self
            .user_modal_query
            .trim()
            .trim_start_matches('@')
            .to_ascii_lowercase();
        let mut options: Vec<(String, String)> = self
            .user_filter_options()
            .into_iter()
            .filter(|(value, label)| {
                needle.is_empty() || value.to_ascii_lowercase().contains(&needle) || label.to_ascii_lowercase().contains(&needle)
            })
            .collect();
        let typed = self.user_modal_query.trim();
        if !typed.is_empty()
            && let Ok(normalized) = wsid_core::normalize_user(typed)
            && !options
                .iter()
                .any(|(value, _)| value.eq_ignore_ascii_case(&normalized))
        {
            options.push((normalized.clone(), format!("@{normalized}")));
        }
        options
    }

    fn open_user_modal(&mut self) {
        self.user_modal_open = true;
        self.user_modal_query.clear();
        self.user_modal_history.clear();
        let current = self.current_user_filter();
        let options = self.filtered_user_options();
        self.user_modal_selected = options
            .iter()
            .position(|(value, _)| *value == current)
            .unwrap_or(0);
        self.status_message = if self.user_filter_allows_all() {
            "User filter — type to search, Up/Down to move, Enter to apply, Esc to cancel".to_string()
        } else {
            format!("User filter (single user) — active @{}", self.current_user_filter())
        };
    }

    fn apply_user_filter(&mut self, user: String) {
        let every_user = user.eq_ignore_ascii_case(wsid_core::ALL_USERS);
        if every_user && !self.user_filter_allows_all() {
            self.status_message = format!("{} shows a single user, not '{}'", self.active_tab.title(), wsid_core::ALL_USERS);
            return;
        }
        match self.active_tab {
            ActiveTab::Focus => {
                self.focus_user = user.clone();
                let _ = self.refresh_all();
            }
            ActiveTab::Kanban => {
                self.kanban_user = user.clone();
                self.clamp_kanban_selection();
            }
            ActiveTab::Eisenhower => {
                self.eisenhower_user = user.clone();
                let _ = self.refresh_all();
            }
            ActiveTab::Database => {
                self.search_assignee_filter = if every_user { None } else { Some(user.clone()) };
                let _ = self.refresh_search();
            }
            ActiveTab::DataEntry => {}
        }
        self.status_message =
            if every_user { format!("User filter: {} (every user)", wsid_core::ALL_USERS) } else { format!("User filter: @{user}") };
    }

    fn handle_user_modal_key(&mut self, key: KeyEvent) {
        // `Ctrl+U` inside the filter modal switches the acting user instead.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.user_modal_open = false;
            self.begin_user_switch();
            return;
        }
        if let Some(changed) = apply_text_shortcut(&key, &mut self.user_modal_query, &mut self.user_modal_history) {
            if changed {
                self.user_modal_selected = 0;
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.user_modal_open = false;
                self.status_message = "Cancelled user filter".to_string();
            }
            KeyCode::Enter => {
                if let Some((user, _)) = self
                    .filtered_user_options()
                    .get(self.user_modal_selected)
                    .cloned()
                {
                    self.user_modal_open = false;
                    self.apply_user_filter(user);
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                let len = self.filtered_user_options().len();
                if len > 0 {
                    self.user_modal_selected = (self.user_modal_selected + 1) % len;
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                let len = self.filtered_user_options().len();
                if len > 0 {
                    self.user_modal_selected = self.user_modal_selected.checked_sub(1).unwrap_or(len - 1);
                }
            }
            KeyCode::Backspace => {
                self.user_modal_query.pop();
                self.user_modal_selected = 0;
            }
            KeyCode::Char(ch) => {
                self.user_modal_query.push(ch);
                self.user_modal_selected = 0;
            }
            _ => {}
        }
    }

    pub(crate) fn clamp_eisenhower_selection(&mut self) {
        for q_idx in 0..4 {
            let q_len = self.eisenhower_tasks_in_quadrant(q_idx).len();
            let current = self.eisenhower_states[q_idx].selected().unwrap_or(0);
            if q_len == 0 {
                self.eisenhower_states[q_idx].select(None);
            } else if current >= q_len {
                self.eisenhower_states[q_idx].select(Some(q_len - 1));
            } else {
                self.eisenhower_states[q_idx].select(Some(current));
            }
        }
        let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
        if items.is_empty() {
            self.eisenhower_row = 0;
        } else {
            let row = self.eisenhower_states[self.eisenhower_quadrant]
                .selected()
                .unwrap_or(self.eisenhower_row)
                .min(items.len() - 1);
            self.eisenhower_row = row;
            self.eisenhower_states[self.eisenhower_quadrant].select(Some(row));
        }
    }

    /// Tasks the active view currently shows, in display order.
    fn visible_tasks(&self) -> Vec<&Task> {
        match self.active_tab {
            ActiveTab::Focus => self
                .focus_queue
                .actionable_tasks
                .iter()
                .map(|f| &f.task)
                .collect(),
            ActiveTab::Kanban => self
                .all_tasks
                .iter()
                .filter(|t| self.matches_project(&self.kanban_project, &t.project))
                .filter(|t| self.matches_user(&self.kanban_user, &t.assignee))
                .collect(),
            ActiveTab::Eisenhower => self
                .eisenhower_items
                .iter()
                .filter(|i| self.matches_project(&self.eisenhower_project, &i.task.project))
                .map(|i| &i.task)
                .collect(),
            ActiveTab::Database => self.search_hits.iter().map(|h| &h.task).collect(),
            ActiveTab::DataEntry => Vec::new(),
        }
    }

    /// Task snapshots the next edit applies to: every checked task this view still shows,
    /// otherwise the focused task. The focused task is rotated to the front so focus can follow it.
    fn edit_targets(&self) -> Vec<Task> {
        let focused = self.highlighted_task_id();
        let visible = self.visible_tasks();
        let mut targets: Vec<Task> = visible
            .iter()
            .filter(|t| self.selected_ids.contains(&t.id))
            .map(|t| (*t).clone())
            .collect();
        if targets.is_empty() {
            targets = visible
                .into_iter()
                .filter(|t| Some(t.id.as_str()) == focused.as_deref())
                .cloned()
                .collect();
        }
        if let Some(pos) = focused
            .as_deref()
            .and_then(|id| targets.iter().position(|t| t.id == id))
        {
            targets.rotate_left(pos);
        }
        targets
    }

    /// Re-focuses the active view on `task_id` after an edit, so a task that moved quadrant,
    /// column, or result row keeps the cursor instead of stranding it in an emptied list.
    fn follow_edit(&mut self, task_id: &str) {
        match self.active_tab {
            ActiveTab::Eisenhower => {
                self.focus_eisenhower_task(task_id);
                self.fallback_eisenhower_focus();
            }
            ActiveTab::Kanban => {
                self.focus_kanban_task(task_id);
                self.fallback_kanban_focus();
            }
            ActiveTab::Database => self.focus_database_task(task_id),
            _ => {}
        }
    }

    /// Puts the cursor on `task_id` in the Eisenhower matrix (`no-op` when it left the matrix).
    pub fn focus_eisenhower_task(&mut self, task_id: &str) {
        let found = (0..4).find_map(|q| {
            self.eisenhower_tasks_in_quadrant(q)
                .iter()
                .position(|c| c.task.id == task_id)
                .map(|row| (q, row))
        });
        if let Some((quadrant, row)) = found {
            self.eisenhower_quadrant = quadrant;
            self.eisenhower_row = row;
            self.eisenhower_states[quadrant].select(Some(row));
        }
    }

    /// Puts the cursor on `task_id` in the Kanban board (`no-op` when the task is not shown).
    pub fn focus_kanban_task(&mut self, task_id: &str) {
        let found = (0..TaskStatus::ALL.len()).find_map(|col| {
            self.kanban_tasks_in_col(col)
                .iter()
                .position(|t| t.id == task_id)
                .map(|row| (col, row))
        });
        if let Some((col, row)) = found {
            self.kanban_col = col;
            self.kanban_row = row;
            self.kanban_states[col].select(Some(row));
        }
    }

    /// Puts the cursor on `task_id` in the Database results (`no-op` when it was filtered out).
    pub fn focus_database_task(&mut self, task_id: &str) {
        if let Some(row) = self.search_hits.iter().position(|h| h.task.id == task_id) {
            self.search_selected = row;
            self.search_table_state.select(Some(row));
            self.search_list_state.select(Some(row));
        }
    }

    /// Moves the cursor off an emptied quadrant after an edit: a checked task first, then the
    /// first non-empty quadrant. Navigating into an empty quadrant is deliberately left alone.
    fn fallback_eisenhower_focus(&mut self) {
        if !self
            .eisenhower_tasks_in_quadrant(self.eisenhower_quadrant)
            .is_empty()
        {
            return;
        }
        let checked = (0..4).find(|&q| {
            self.eisenhower_tasks_in_quadrant(q)
                .iter()
                .any(|c| self.selected_ids.contains(&c.task.id))
        });
        let target = checked.or_else(|| (0..4).find(|&q| !self.eisenhower_tasks_in_quadrant(q).is_empty()));
        let Some(quadrant) = target else {
            self.eisenhower_row = 0;
            return;
        };
        self.eisenhower_quadrant = quadrant;
        self.eisenhower_row = 0;
        self.eisenhower_states[quadrant].select(Some(0));
    }

    /// Moves the cursor off an empty column after an edit, preferring a checked task's column.
    fn fallback_kanban_focus(&mut self) {
        if !self.kanban_tasks_in_col(self.kanban_col).is_empty() {
            return;
        }
        let checked = (0..TaskStatus::ALL.len()).find(|&c| {
            self.kanban_tasks_in_col(c)
                .iter()
                .any(|t| self.selected_ids.contains(&t.id))
        });
        let target = checked.or_else(|| (0..TaskStatus::ALL.len()).find(|&c| !self.kanban_tasks_in_col(c).is_empty()));
        let Some(col) = target else {
            self.kanban_row = 0;
            return;
        };
        self.kanban_col = col;
        self.kanban_row = 0;
        self.kanban_states[col].select(Some(0));
    }

    /// Applies the planned updates to every edit target, refreshes once, and returns focus to the
    /// first target. Returns that task's id and how many tasks were updated.
    fn update_targets(&mut self, plan: impl Fn(&Task) -> Option<UpdateTaskInput>) -> Option<(String, usize)> {
        let targets = self.edit_targets();
        if targets.is_empty() {
            return None;
        }
        let planned: Vec<(String, UpdateTaskInput)> = targets
            .iter()
            .filter_map(|task| plan(task).map(|input| (task.id.clone(), input)))
            .collect();
        if planned.is_empty() {
            return None;
        }
        let mut failure = None;
        let mut updated = 0;
        for (id, input) in planned {
            match update_task(&mut self.db, &id, input) {
                Ok(_) => updated += 1,
                Err(err) => failure = Some(format!("Failed to update [{id}]: {err}")),
            }
        }
        let primary = targets[0].id.clone();
        let _ = self.refresh_all();
        self.follow_edit(&primary);
        if let Some(err) = failure {
            self.status_message = err;
            return None;
        }
        Some((primary, updated))
    }

    /// Raises (`1`) or lowers (`-1`) importance for every edit target, clamped to `1..=5`.
    fn nudge_target_importance(&mut self, delta: i8) {
        let Some((primary, count)) = self.update_targets(|task| {
            let next = (i16::from(task.importance) + i16::from(delta)).clamp(1, 5) as u8;
            (next != task.importance).then_some(UpdateTaskInput { importance: Some(next), ..Default::default() })
        }) else {
            return;
        };
        let importance = self
            .all_tasks
            .iter()
            .find(|t| t.id == primary)
            .map_or(0, |t| t.importance);
        self.status_message = if count == 1 {
            format!("Updated [{primary}] importance to {importance}/5")
        } else {
            format!("Updated importance of {count} task(s) — [{primary}] is now {importance}/5")
        };
    }

    /// Toggles the urgency override on every edit target.
    fn toggle_target_urgency(&mut self) {
        let Some((primary, count)) =
            self.update_targets(|task| Some(UpdateTaskInput { urgent_override: Some(!task.urgent_override), ..Default::default() }))
        else {
            return;
        };
        let urgent = self
            .all_tasks
            .iter()
            .find(|t| t.id == primary)
            .is_some_and(|t| t.urgent_override);
        self.status_message = if count == 1 {
            format!("Updated [{primary}] urgent_override to {urgent}")
        } else {
            format!("Updated urgent_override of {count} task(s) — [{primary}] is {urgent}")
        };
    }

    /// Forces the urgency of every edit target, so the matrix moves left (`false`) or right (`true`).
    fn set_target_urgency(&mut self, urgent: bool) {
        let Some((primary, count)) = self.update_targets(|task| {
            (task.urgent_override != urgent).then_some(UpdateTaskInput { urgent_override: Some(urgent), ..Default::default() })
        }) else {
            return;
        };
        let placed = self
            .eisenhower_items
            .iter()
            .find(|c| c.task.id == primary)
            .map(|c| (c.is_urgent, c.slack_days.unwrap_or(0.0), c.quadrant.number()));
        self.status_message = match placed {
            Some((is_urgent, slack, _)) if is_urgent != urgent => {
                format!("[{primary}] stays urgent: only {slack:.1} day(s) of slack, inside the urgent horizon")
            }
            Some((_, _, number)) if count == 1 => format!("Moved [{primary}] to Q{number}"),
            Some((_, _, number)) => format!("Moved {count} task(s) — [{primary}] is now in Q{number}"),
            None => format!("Moved {count} task(s)"),
        };
    }

    /// Applies `status` to every edit target. A single target keeps the detailed
    /// spawn/auto-complete messaging; several report one summary line.
    fn transition_targets(&mut self, status: TaskStatus) {
        let targets = self.edit_targets();
        if targets.is_empty() {
            return;
        }
        let primary = targets[0].id.clone();
        if targets.len() == 1 {
            self.apply_status_transition(&primary, status);
            self.follow_edit(&primary);
            return;
        }
        let mut failure = None;
        let mut moved = 0;
        for task in &targets {
            match transition_task_status(&mut self.db, &task.id, status) {
                Ok(_) => moved += 1,
                Err(err) => failure = Some(format!("Failed to move [{}]: {err}", task.id)),
            }
        }
        let _ = self.refresh_all();
        self.follow_edit(&primary);
        self.status_message = match failure {
            Some(err) => err,
            None => format!("Status -> {status} for {moved} task(s)"),
        };
    }

    fn handle_eisenhower_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                self.eisenhower_quadrant = self.eisenhower_quadrant.checked_sub(1).unwrap_or(3);
                self.eisenhower_row = self.eisenhower_states[self.eisenhower_quadrant]
                    .selected()
                    .unwrap_or(0);
                self.clamp_eisenhower_selection();
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.eisenhower_quadrant = (self.eisenhower_quadrant + 1) % 4;
                self.eisenhower_row = self.eisenhower_states[self.eisenhower_quadrant]
                    .selected()
                    .unwrap_or(0);
                self.clamp_eisenhower_selection();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self
                    .eisenhower_tasks_in_quadrant(self.eisenhower_quadrant)
                    .len();
                if len > 0 {
                    self.eisenhower_row = (self.eisenhower_row + 1) % len;
                    self.eisenhower_states[self.eisenhower_quadrant].select(Some(self.eisenhower_row));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self
                    .eisenhower_tasks_in_quadrant(self.eisenhower_quadrant)
                    .len();
                if len > 0 {
                    self.eisenhower_row = self.eisenhower_row.checked_sub(1).unwrap_or(len - 1);
                    self.eisenhower_states[self.eisenhower_quadrant].select(Some(self.eisenhower_row));
                }
            }
            KeyCode::Char('<') => self.set_target_urgency(false),
            KeyCode::Char('>') => self.set_target_urgency(true),
            KeyCode::Char('+') | KeyCode::Char('=') => self.nudge_target_importance(1),
            KeyCode::Char('-') => self.nudge_target_importance(-1),
            KeyCode::Char('u') => self.toggle_target_urgency(),
            KeyCode::Char('s') => self.transition_targets(TaskStatus::InProgress),
            KeyCode::Char('d') => self.transition_targets(TaskStatus::Completed),
            KeyCode::Enter => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    self.open_detail_for(&id);
                }
            }
            _ => {}
        }
    }

    fn search_selected_task(&self) -> Option<&Task> {
        self.search_hits
            .get(self.search_selected)
            .map(|hit| &hit.task)
    }

    /// Filter token for the highlighted column, prefilled into the search bar by `/`.
    fn search_prefill_for_column(&self) -> String {
        let Some(field) = self.search_sort_col.filter_field() else {
            return String::new();
        };
        let value = self
            .search_selected_task()
            .and_then(|task| match self.search_sort_col {
                DatabaseSortColumn::Id => Some(task.id.clone()),
                DatabaseSortColumn::Title => Some(task.title.clone()),
                DatabaseSortColumn::Status => Some(task.status.as_str().to_string()),
                DatabaseSortColumn::Project => Some(task.project.clone()),
                DatabaseSortColumn::Assignee => Some(task.assignee.clone()),
                DatabaseSortColumn::Importance => Some(task.importance.to_string()),
                DatabaseSortColumn::Difficulty => Some(task.difficulty.to_string()),
                DatabaseSortColumn::SgdbDate => task
                    .sgdb_date
                    .as_deref()
                    .map(wsid_core::format_deadline_date),
                DatabaseSortColumn::DueDate => task
                    .due_date
                    .as_deref()
                    .map(wsid_core::format_deadline_date),
                DatabaseSortColumn::Relevance => None,
            });
        match value {
            Some(value) if !value.trim().is_empty() => {
                let quoted = if value.chars().any(char::is_whitespace) { format!("\"{value}\"") } else { value };
                format!("{field}=={quoted}")
            }
            // No value in that cell: leave the operator ready for typing (`due>=`).
            _ => format!("{field}=="),
        }
    }

    fn handle_database_key(&mut self, key: KeyEvent) {
        let len = self.search_hits.len();
        match key.code {
            KeyCode::Char('/') => {
                self.search_editing = true;
                self.search_history.clear();
                let prefill = self.search_prefill_for_column();
                if prefill.is_empty() {
                    self.status_message =
                        "Editing search — words match fuzzily; try assignee==davidb, imp>=3, diff<=2, due<=2026-11-03, est<=2h. Enter/Esc to finish"
                            .to_string();
                } else {
                    if self.search_query.trim().is_empty() {
                        self.search_query = prefill.clone();
                    } else {
                        self.search_query.push(' ');
                        self.search_query.push_str(&prefill);
                    }
                    let _ = self.refresh_search();
                    self.status_message = format!("Filtering by '{prefill}' — swap == for >=/<=/!= to compare, Enter or Esc to finish");
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.search_selected = (self.search_selected + 1) % len;
                    self.search_table_state.select(Some(self.search_selected));
                    self.search_list_state.select(Some(self.search_selected));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    self.search_selected = self.search_selected.checked_sub(1).unwrap_or(len - 1);
                    self.search_table_state.select(Some(self.search_selected));
                    self.search_list_state.select(Some(self.search_selected));
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if len > 0 {
                    self.search_selected = 0;
                    self.search_table_state.select(Some(0));
                    self.search_list_state.select(Some(0));
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if len > 0 {
                    self.search_selected = len - 1;
                    self.search_table_state.select(Some(self.search_selected));
                    self.search_list_state.select(Some(self.search_selected));
                }
            }
            KeyCode::Char('f') => {
                self.search_status_filter = match self.search_status_filter {
                    None => Some(TaskStatus::Todo),
                    Some(TaskStatus::Todo) => Some(TaskStatus::InProgress),
                    Some(TaskStatus::InProgress) => Some(TaskStatus::InReview),
                    Some(TaskStatus::InReview) => Some(TaskStatus::Stuck),
                    Some(TaskStatus::Stuck) => Some(TaskStatus::Backlog),
                    Some(TaskStatus::Backlog) => Some(TaskStatus::Completed),
                    Some(TaskStatus::Completed) => None,
                };
                let _ = self.refresh_search();
            }
            KeyCode::Char('t') => {
                let mut tags: Vec<String> = self.all_tasks.iter().flat_map(|t| t.tags.clone()).collect();
                tags.sort();
                tags.dedup();
                self.search_tag_filter = match &self.search_tag_filter {
                    None => tags.first().cloned(),
                    Some(cur) => tags
                        .iter()
                        .position(|t| t == cur)
                        .and_then(|i| tags.get(i + 1).cloned()),
                };
                let _ = self.refresh_search();
            }
            KeyCode::Char('u') => {
                let mut users: Vec<String> = self.all_tasks.iter().map(|t| t.assignee.clone()).collect();
                users.sort();
                users.dedup();
                self.search_assignee_filter = match &self.search_assignee_filter {
                    None => users.first().cloned(),
                    Some(cur) => users
                        .iter()
                        .position(|u| u == cur)
                        .and_then(|i| users.get(i + 1).cloned()),
                };
                let _ = self.refresh_search();
            }
            KeyCode::Char('m') => {
                self.search_only_mine = !self.search_only_mine;
                let _ = self.refresh_search();
            }
            KeyCode::Char('o') => {
                self.search_sort_col = self.search_sort_col.next();
                let _ = self.refresh_search();
            }
            KeyCode::Char('O') => {
                self.search_sort_asc = !self.search_sort_asc;
                let _ = self.refresh_search();
            }
            KeyCode::Enter => {
                if let Some(hit) = self.search_hits.get(self.search_selected) {
                    let id = hit.task.id.clone();
                    self.open_detail_for(&id);
                }
            }
            _ => {}
        }
    }

    fn handle_search_input_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.search_query, &mut self.search_history) {
            if changed {
                let _ = self.refresh_search();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search_editing = false;
                self.status_message = format!("Search found {} matching task(s)", self.search_hits.len());
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                let _ = self.refresh_search();
            }
            KeyCode::Char(ch) => {
                self.search_query.push(ch);
                let _ = self.refresh_search();
            }
            _ => {}
        }
    }

    fn handle_data_entry_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.submit_data_entry_form();
            return;
        }
        if self
            .entry_form
            .apply_text_shortcut(&self.db, &self.user, &key)
        {
            return;
        }
        if is_modified_char(&key) {
            return;
        }

        let rows = DataEntryForm::SUBMIT_INDEX + 1;
        let (db, user) = (&self.db, self.user.as_str());
        let form = &mut self.entry_form;
        let mut submit = false;
        match key.code {
            KeyCode::Esc => form.escape(db, user),
            KeyCode::Tab => form.set_active_index(db, user, (form.active_index + 1) % rows),
            KeyCode::BackTab => form.set_active_index(db, user, form.active_index.checked_sub(1).unwrap_or(rows - 1)),
            KeyCode::Down | KeyCode::Up => {
                let delta: isize = if key.code == KeyCode::Down { 1 } else { -1 };
                if let Some(dd) = form.dropdown.as_mut() {
                    dd.step(delta);
                } else {
                    let next = (form.active_index as isize + delta).rem_euclid(rows as isize) as usize;
                    form.set_active_index(db, user, next);
                }
            }
            KeyCode::Enter => {
                if form.active_index == DataEntryForm::SUBMIT_INDEX {
                    submit = true;
                } else if form.enter(db, user) {
                    form.set_active_index(db, user, (form.active_index + 1) % rows);
                }
            }
            KeyCode::Delete => {
                form.delete_selected_item();
            }
            KeyCode::Backspace => {
                if !form.delete_selected_item() {
                    form.backspace(db, user);
                }
            }
            KeyCode::Char(ch) => {
                if form.active_index < DataEntryForm::SUBMIT_INDEX {
                    form.type_char(db, user, ch);
                } else if ch == ' ' {
                    submit = true;
                }
            }
            _ => {}
        }
        if submit {
            self.submit_data_entry_form();
        }
    }

    /// Bracketed paste (also how terminals deliver drag-and-dropped file paths).
    pub fn handle_paste(&mut self, text: &str) {
        match self.focus_context() {
            FocusContext::ProjectModal => {
                let before = self.project_modal_query.clone();
                self.project_modal_query
                    .push_str(&text_edit::flatten_paste(text));
                self.project_modal_history
                    .record(&before, EditKind::Discrete);
                self.project_modal_selected = 0;
            }
            FocusContext::UserModal => {
                let before = self.user_modal_query.clone();
                self.user_modal_query
                    .push_str(&text_edit::flatten_paste(text));
                self.user_modal_history.record(&before, EditKind::Discrete);
                self.user_modal_selected = 0;
            }
            FocusContext::SearchInput => {
                let before = self.search_query.clone();
                self.search_query.push_str(&text_edit::flatten_paste(text));
                self.search_history.record(&before, EditKind::Discrete);
                let _ = self.refresh_search();
            }
            FocusContext::UserSwitchPrompt => {
                let before = self.user_input.clone();
                self.user_input.push_str(&text_edit::flatten_paste(text));
                self.user_history.record(&before, EditKind::Discrete);
                self.update_user_prompt_message();
            }
            FocusContext::BulkEditPrompt => {
                let before = self.bulk_input.clone();
                self.bulk_input.push_str(&text_edit::flatten_paste(text));
                self.bulk_history.record(&before, EditKind::Discrete);
                self.update_bulk_prompt_message();
            }
            FocusContext::DetailPrompt(_) => {
                let before = self.detail_prompt_input.clone();
                self.detail_prompt_input
                    .push_str(&text_edit::flatten_paste(text));
                self.detail_prompt_history
                    .record(&before, EditKind::Discrete);
                self.refresh_comment_suggestions();
            }
            FocusContext::Tab(ActiveTab::DataEntry) if self.form_errors.is_none() => {
                self.entry_form.paste(&self.db, &self.user, text);
            }
            _ => {}
        }
    }

    /// Opens the full data entry form pre-filled with the task's current values (`E` in the
    /// detail modal), so editing uses the same validation and pickers as task creation.
    fn begin_edit_task(&mut self) {
        let Some(snapshot) = self.detail_modal.as_ref() else {
            return;
        };
        let task = snapshot.task.clone();
        self.entry_form = DataEntryForm::from_task(&task, &self.user);
        self.entry_form.set_active_index(&self.db, &self.user, 0);
        self.editing_task_id = Some(task.id.clone());
        self.editing_return_tab = Some(self.active_tab);
        self.detail_modal = None;
        self.detail_prompt_kind = DetailPromptKind::None;
        self.detail_prompt_input.clear();
        self.clear_comment_suggestions();
        self.active_tab = ActiveTab::DataEntry;
        self.status_message = format!("Editing [{}] {} — Ctrl+S to save, Esc to cancel", task.id, task.title);
    }

    /// Leaves the pre-filled form and returns to the task detail modal without saving.
    fn cancel_edit_task(&mut self) {
        let id = self.editing_task_id.take();
        if let Some(tab) = self.editing_return_tab.take() {
            self.active_tab = tab;
        }
        self.entry_form = DataEntryForm::new(&self.user);
        if let Some(id) = id {
            self.open_detail_for(&id);
            self.status_message = format!("Cancelled editing [{id}]");
        }
    }

    pub fn submit_data_entry_form(&mut self) {
        let problems = self.entry_form.validate_all(&self.db, &self.user);
        if !problems.is_empty() {
            self.form_errors = Some(problems.into_iter().map(|(i, m)| (Some(i), m)).collect());
            return;
        }

        let draft = self.entry_form.to_draft();
        if let Some(id) = self.editing_task_id.clone() {
            self.submit_edited_task(&id, &draft);
            return;
        }
        match create_task_from_draft(&mut self.db, &draft, &self.user) {
            Ok(created) => {
                self.entry_form = DataEntryForm::new(&self.user);
                let _ = self.refresh_all();
                self.status_message = format!("Created task [{}] {}", created.id, created.title);
            }
            Err(WsidError::DraftRejected(issues)) => {
                self.form_errors = Some(
                    issues
                        .into_iter()
                        .filter(FieldIssue::is_error)
                        .map(|i| (Some(form::field_index(i.field)), i.message))
                        .collect(),
                );
            }
            Err(err) => {
                let msg = err.to_string();
                let msg = msg
                    .strip_prefix("validation error: ")
                    .unwrap_or(&msg)
                    .to_string();
                self.form_errors = Some(vec![(None, msg)]);
            }
        }
    }

    /// Applies an edited form to the task it was opened for and returns to its detail modal.
    fn submit_edited_task(&mut self, id: &str, draft: &wsid_core::TaskDraft) {
        match wsid_core::update_task_from_draft(&mut self.db, id, draft, &self.user) {
            Ok(updated) => {
                self.editing_task_id = None;
                self.entry_form = DataEntryForm::new(&self.user);
                if let Some(tab) = self.editing_return_tab.take() {
                    self.active_tab = tab;
                }
                let _ = self.refresh_all();
                self.open_detail_for(&updated.id);
                self.status_message = format!("Updated task [{}] {}", updated.id, updated.title);
            }
            Err(WsidError::DraftRejected(issues)) => {
                self.form_errors = Some(
                    issues
                        .into_iter()
                        .filter(FieldIssue::is_error)
                        .map(|i| (Some(form::field_index(i.field)), i.message))
                        .collect(),
                )
            }
            Err(err) => {
                let msg = err.to_string();
                let msg = msg
                    .strip_prefix("validation error: ")
                    .unwrap_or(&msg)
                    .to_string();
                self.form_errors = Some(vec![(None, msg)]);
            }
        }
    }

    fn handle_about_modal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.about_modal_open = false;
                self.status_message = "Closed About & Licenses modal".to_string();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.about_scroll = self.about_scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.about_scroll = self.about_scroll.saturating_sub(1);
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.about_scroll = self.about_scroll.saturating_add(15);
            }
            KeyCode::PageUp => {
                self.about_scroll = self.about_scroll.saturating_sub(15);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.about_scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                // Clamped against the wrapped line count when rendered.
                self.about_scroll = u16::MAX;
            }
            _ => {}
        }
    }

    /// Opens a detail prompt with `initial` text and a fresh undo history.
    fn begin_detail_prompt(&mut self, kind: DetailPromptKind, initial: String) {
        self.detail_prompt_kind = kind;
        self.detail_prompt_input = initial;
        self.detail_prompt_history.clear();
    }

    fn handle_detail_modal_key(&mut self, key: KeyEvent) {
        if self.detail_prompt_kind != DetailPromptKind::None
            && let Some(changed) = apply_text_shortcut(&key, &mut self.detail_prompt_input, &mut self.detail_prompt_history)
        {
            if changed {
                self.refresh_comment_suggestions();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        if self.detail_prompt_kind != DetailPromptKind::None {
            let suggesting = self.detail_prompt_kind == DetailPromptKind::AddComment && !self.detail_suggestions.is_empty();
            match key.code {
                KeyCode::Esc => {
                    if suggesting {
                        self.clear_comment_suggestions();
                        self.status_message = "Dismissed suggestions".to_string();
                    } else {
                        self.detail_prompt_kind = DetailPromptKind::None;
                        self.detail_prompt_input.clear();
                        self.clear_comment_suggestions();
                        self.status_message = "Cancelled prompt".to_string();
                    }
                }
                KeyCode::Backspace => {
                    self.detail_prompt_input.pop();
                    self.refresh_comment_suggestions();
                }
                KeyCode::Tab => {
                    if suggesting {
                        self.accept_comment_suggestion();
                    }
                }
                KeyCode::Up => {
                    if suggesting {
                        self.detail_suggestion_index = self.detail_suggestion_index.saturating_sub(1);
                        self.detail_suggestion_navigated = true;
                        self.announce_highlighted_suggestion();
                    }
                }
                KeyCode::Down => {
                    if suggesting {
                        self.detail_suggestion_index = (self.detail_suggestion_index + 1).min(self.detail_suggestions.len() - 1);
                        self.detail_suggestion_navigated = true;
                        self.announce_highlighted_suggestion();
                    }
                }
                KeyCode::Enter => {
                    if suggesting && self.detail_suggestion_navigated {
                        self.accept_comment_suggestion();
                    } else {
                        self.submit_detail_prompt();
                    }
                }
                KeyCode::Char(ch) => {
                    self.detail_prompt_input.push(ch);
                    self.refresh_comment_suggestions();
                }
                _ => {}
            }
            return;
        }

        let Some(snapshot) = self.detail_modal.clone() else {
            return;
        };
        let atts = snapshot.all_attachments();

        if self.attachment_focused {
            match key.code {
                KeyCode::Esc => {
                    self.attachment_focused = false;
                    self.status_message = "Left attachments list".to_string();
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !atts.is_empty() {
                        if self.attachment_cursor + 1 < atts.len() {
                            self.attachment_cursor += 1;
                        }
                        let att = &atts[self.attachment_cursor];
                        self.status_message =
                            format!("Attachment {}/{}: {} ({})", self.attachment_cursor + 1, atts.len(), att.file_name, att.file_path);
                    }
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if !atts.is_empty() {
                        self.attachment_cursor = self.attachment_cursor.saturating_sub(1);
                        let att = &atts[self.attachment_cursor.min(atts.len() - 1)];
                        self.status_message =
                            format!("Attachment {}/{}: {} ({})", self.attachment_cursor + 1, atts.len(), att.file_name, att.file_path);
                    }
                    return;
                }
                KeyCode::Char(' ') => {
                    if let Some(att) = atts.get(self.attachment_cursor.min(atts.len().saturating_sub(1))) {
                        if !self.attachment_marked.remove(&att.id) {
                            self.attachment_marked.insert(att.id);
                            self.status_message = format!("Marked attachment #{} ({})", att.id, att.file_name);
                        } else {
                            self.status_message = format!("Unmarked attachment #{} ({})", att.id, att.file_name);
                        }
                    }
                    return;
                }
                KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace => {
                    if let Some(att) = atts.get(self.attachment_cursor.min(atts.len().saturating_sub(1))) {
                        match remove_attachment(&mut self.db, att.id) {
                            Ok(_) => {
                                self.attachment_marked.remove(&att.id);
                                let _ = self.refresh_all();
                                self.status_message = format!("Removed attachment #{} ({})", att.id, att.file_name);
                            }
                            Err(err) => {
                                self.status_message = format!("Failed to remove attachment: {err}");
                            }
                        }
                    } else {
                        self.status_message = "No attachments to remove".to_string();
                    }
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.detail_modal = None;
                self.attachment_focused = false;
                self.status_message = "Closed task detail drawer".to_string();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut modal) = self.detail_modal {
                    modal.scroll = modal.scroll.saturating_add(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut modal) = self.detail_modal {
                    modal.scroll = modal.scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('a') => {
                if atts.is_empty() {
                    self.status_message = "No attachments on this task or its notes".to_string();
                } else {
                    self.attachment_focused = true;
                    self.attachment_cursor = self.attachment_cursor.min(atts.len() - 1);
                    let att = &atts[self.attachment_cursor];
                    self.status_message =
                        format!("Attachments focused ({}/{}) — {}", self.attachment_cursor + 1, atts.len(), att.file_name);
                }
            }
            KeyCode::Char('c') => {
                self.begin_detail_prompt(DetailPromptKind::AddComment, String::new());
                self.clear_comment_suggestions();
                self.status_message = "New comment — @user, @takenote:user, and #AB1234 are linked".to_string();
            }
            KeyCode::Char('f') => {
                self.begin_detail_prompt(DetailPromptKind::AttachFile, String::new());
                self.status_message = "Attach file: /path/to/file | optional alt text".to_string();
            }
            KeyCode::Char('v') => {
                self.begin_detail_prompt(DetailPromptKind::AttachClipboard, String::new());
                self.status_message = "Paste from clipboard: optional alt text (empty uses the file name)".to_string();
            }
            KeyCode::Char('e') => {
                if let Some(att) = atts.get(self.attachment_cursor.min(atts.len().saturating_sub(1))) {
                    self.begin_detail_prompt(DetailPromptKind::EditAttachmentDesc, att.description.clone());
                    self.status_message = format!("Edit alt text for {} (#{})", att.file_name, att.id);
                } else {
                    self.status_message = "No attachments on this task or its notes".to_string();
                }
            }
            KeyCode::Char('o') => {
                if atts.is_empty() {
                    self.status_message = "No attachments on this task or its notes".to_string();
                } else {
                    let targets: Vec<&TaskAttachment> = if !self.attachment_marked.is_empty() {
                        atts.iter()
                            .filter(|a| self.attachment_marked.contains(&a.id))
                            .collect()
                    } else {
                        vec![&atts[self.attachment_cursor.min(atts.len() - 1)]]
                    };
                    self.open_attachment_targets(&targets);
                }
            }
            KeyCode::Char('O') => {
                if atts.is_empty() {
                    self.status_message = "No attachments on this task or its notes".to_string();
                } else {
                    let targets: Vec<&TaskAttachment> = atts.iter().collect();
                    self.open_attachment_targets(&targets);
                }
            }
            KeyCode::Char('u') => {
                let target_id =
                    if let Some(first_blocker) = snapshot.blockers.first() { first_blocker.id.clone() } else { snapshot.task.id.clone() };
                match unblock_or_dismiss_subtask(&mut self.db, &target_id, true) {
                    Ok(dismissed) => {
                        let _ = self.refresh_all();
                        self.status_message = format!("Dismissed blocker subtask [{}] {}", dismissed.id, dismissed.title);
                    }
                    Err(err) => {
                        self.status_message = format!("Failed to dismiss blocker: {err}");
                    }
                }
            }
            KeyCode::Char('g') => {
                let default_jump = snapshot
                    .blockers
                    .first()
                    .or_else(|| snapshot.parents.first())
                    .or_else(|| snapshot.subtasks.first())
                    .or_else(|| snapshot.related.first())
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
                self.begin_detail_prompt(DetailPromptKind::JumpToTask, default_jump);
                self.status_message = "Jump to task ID (e.g. AB1234)".to_string();
            }
            KeyCode::Char('s') => {
                self.apply_status_transition(&snapshot.task.id, TaskStatus::InProgress);
            }
            KeyCode::Char('E') => self.begin_edit_task(),
            KeyCode::Char('D') => {
                self.begin_detail_prompt(DetailPromptKind::ConfirmDeleteTask, String::new());
                self.status_message = format!("Delete [{}] {}? Type `yes` and press Enter", snapshot.task.id, snapshot.task.title);
            }
            KeyCode::Char('N') => {
                if snapshot.notes.is_empty() {
                    self.status_message = "No comments to select".to_string();
                } else {
                    self.detail_note_cursor = (self.detail_note_cursor + 1) % snapshot.notes.len();
                    let n = &snapshot.notes[self.detail_note_cursor];
                    self.status_message = format!("Selected comment #{} by @{} — press X to delete", n.id, n.author);
                }
            }
            KeyCode::Char('X') => {
                match snapshot.notes.get(
                    self.detail_note_cursor
                        .min(snapshot.notes.len().saturating_sub(1)),
                ) {
                    Some(n) => {
                        self.begin_detail_prompt(DetailPromptKind::ConfirmDeleteNote, String::new());
                        self.status_message = format!("Delete comment #{} by @{}? Type `yes` and press Enter", n.id, n.author);
                    }
                    None => self.status_message = "No comments to delete (select one with N)".to_string(),
                }
            }
            KeyCode::Char('r') => {
                self.apply_status_transition(&snapshot.task.id, TaskStatus::InReview);
            }
            KeyCode::Char('d') => {
                self.apply_status_transition(&snapshot.task.id, TaskStatus::Completed);
            }
            KeyCode::Char('x') => {
                self.apply_status_transition(&snapshot.task.id, TaskStatus::Stuck);
            }
            _ => {}
        }
    }

    /// Recomputes `@user` / `@takenote:user` / `#ID` completions and the resolved-token
    /// preview for the active comment prompt. Safe to call on every keystroke.
    pub fn refresh_comment_suggestions(&mut self) {
        self.detail_suggestions.clear();
        self.detail_suggestion_index = 0;
        self.detail_suggestion_navigated = false;
        self.detail_comment_preview.clear();
        if self.detail_prompt_kind != DetailPromptKind::AddComment {
            return;
        }

        let input = self.detail_prompt_input.clone();
        let trailing = trailing_comment_token(&input);
        // Preview only completed tokens: the trailing token is still being typed.
        let completed = match trailing {
            Some(token) => &input[..input.len() - token.raw.len()],
            None => input.as_str(),
        };
        self.detail_comment_preview = self.describe_comment_tokens(completed);

        let Some(token) = trailing else {
            return;
        };
        if token.kind == CommentTokenKind::TaskRef {
            let excluded = self.detail_modal.as_ref().map(|d| d.task.id.clone());
            let mut tasks = if token.query.trim().is_empty() {
                list_tasks(&self.db).unwrap_or_default()
            } else {
                search_relation_targets(&self.db, token.query, 6).unwrap_or_default()
            };
            if let Some(excluded) = excluded {
                tasks.retain(|t| t.id != excluded);
            }
            for task in tasks.into_iter().take(6) {
                self.detail_suggestions.push(CommentSuggestion {
                    insert: format!("#{}", task.id),
                    label: format!("#{} — {}", task.id, task.title),
                    hint: task.status.to_string(),
                });
            }
            return;
        }

        let scope = match token.kind {
            CommentTokenKind::TakeNote => "@takenote:",
            _ => "@",
        };
        let users = suggest_users(&self.db, token.query, &[self.user.as_str(), self.config.primary_user.as_str()]).unwrap_or_default();
        for user in users.into_iter().take(6) {
            self.detail_suggestions.push(CommentSuggestion {
                insert: format!("{scope}{user}"),
                label: format!("{scope}{user}"),
                hint: "known user".to_string(),
            });
        }
        let typed = token.query.trim();
        if !typed.is_empty()
            && !self
                .detail_suggestions
                .iter()
                .any(|s| s.label.eq_ignore_ascii_case(&format!("{scope}{typed}")))
        {
            self.detail_suggestions.push(CommentSuggestion {
                insert: format!("{scope}{typed}"),
                label: format!("{scope}{typed}"),
                hint: "new user".to_string(),
            });
        }
    }

    /// Human-readable resolution of mentions and task links already in the comment.
    fn describe_comment_tokens(&self, body: &str) -> Vec<String> {
        let tokens = parse_note_tokens(body);
        let mut preview = Vec::new();
        let known = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()])
            .unwrap_or_default()
            .into_iter()
            .map(|u| u.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        for user in tokens.blocking_mentions {
            preview.push(if known.contains(&user.to_ascii_lowercase()) {
                format!("@{user} → blocking subtask")
            } else {
                format!("@{user} → new user (blocking subtask)")
            });
        }
        for user in tokens.takenote_mentions {
            preview.push(format!("@takenote:{user} → non-blocking note"));
        }
        for id in tokens.task_references {
            match get_task(&self.db, &id) {
                Ok(task) => preview.push(format!("#{id} → {} ({})", task.title, task.status)),
                Err(_) => preview.push(format!("#{id} → unknown task")),
            }
        }
        preview
    }

    /// Completes the highlighted suggestion and leaves the cursor after a trailing space.
    pub fn accept_comment_suggestion(&mut self) {
        let Some(suggestion) = self
            .detail_suggestions
            .get(self.detail_suggestion_index)
            .cloned()
        else {
            return;
        };
        if let Some(token) = trailing_comment_token(&self.detail_prompt_input) {
            let keep = self.detail_prompt_input.len() - token.raw.len();
            self.detail_prompt_input.truncate(keep);
        }
        self.detail_prompt_input.push_str(&suggestion.insert);
        self.detail_prompt_input.push(' ');
        self.refresh_comment_suggestions();
        self.status_message = format!("Inserted {} (Tab to complete, Enter to post)", suggestion.insert);
    }

    /// Announces the highlighted completion (the accessible-mode narration for the list).
    fn announce_highlighted_suggestion(&mut self) {
        if let Some(suggestion) = self.detail_suggestions.get(self.detail_suggestion_index) {
            self.status_message = format!(
                "Suggestion {}/{}: {} ({}) — Tab to complete",
                self.detail_suggestion_index + 1,
                self.detail_suggestions.len(),
                suggestion.label,
                suggestion.hint
            );
        }
    }

    fn clear_comment_suggestions(&mut self) {
        self.detail_suggestions.clear();
        self.detail_suggestion_index = 0;
        self.detail_suggestion_navigated = false;
        self.detail_comment_preview.clear();
    }

    fn submit_detail_prompt(&mut self) {
        let Some(snapshot) = self.detail_modal.clone() else {
            return;
        };
        let input = self.detail_prompt_input.trim().to_string();
        let kind = self.detail_prompt_kind;
        self.detail_prompt_kind = DetailPromptKind::None;
        self.detail_prompt_input.clear();
        self.clear_comment_suggestions();

        match kind {
            DetailPromptKind::None => {}
            DetailPromptKind::AddComment => {
                if input.is_empty() {
                    self.status_message = "Comment cannot be empty".to_string();
                    return;
                }
                match add_task_note(&mut self.db, &snapshot.task.id, &self.user, &input, true) {
                    Ok(outcome) => {
                        let _ = self.refresh_all();
                        self.status_message = format!(
                            "Posted note #{} on [{}] (spawned {} task(s))",
                            outcome.note.id,
                            snapshot.task.id,
                            outcome.spawned_tasks.len()
                        );
                    }
                    Err(err) => {
                        self.status_message = format!("Failed to add note: {err}");
                    }
                }
            }
            DetailPromptKind::AttachFile => {
                if input.is_empty() {
                    self.status_message = "File path cannot be empty".to_string();
                    return;
                }
                let (path_part, desc_opt) = match input.split_once('|') {
                    Some((p, d)) => (p.trim(), if d.trim().is_empty() { None } else { Some(d.trim().to_string()) }),
                    None => (input.as_str(), None),
                };
                match attach_existing_file(&mut self.db, &snapshot.task.id, None, Path::new(path_part), desc_opt.as_deref()) {
                    Ok(att) => {
                        let _ = self.refresh_all();
                        self.status_message =
                            format!("Attached [att #{}] {} (alt=\"{}\") -> {}", att.id, att.file_name, att.alt_text(), att.file_path);
                    }
                    Err(err) => {
                        self.status_message = format!("Failed to attach file: {err}");
                    }
                }
            }
            DetailPromptKind::AttachClipboard => {
                let desc_opt = if input.is_empty() { None } else { Some(input) };
                match read_system_clipboard_attachment().and_then(|(bytes, fname, _media)| {
                    save_clipboard_attachment(&mut self.db, &snapshot.task.id, None, &bytes, Some(&fname), desc_opt.as_deref())
                }) {
                    Ok(att) => {
                        let _ = self.refresh_all();
                        self.status_message = format!("Saved clipboard attachment [att #{}] -> {}", att.id, att.file_path);
                    }
                    Err(err) => {
                        self.status_message = format!("Clipboard attach failed: {err}");
                    }
                }
            }
            DetailPromptKind::EditAttachmentDesc => {
                let atts = snapshot.all_attachments();
                let Some(att) = atts.get(self.attachment_cursor.min(atts.len().saturating_sub(1))) else {
                    self.status_message = "No attachments on this task or its notes".to_string();
                    return;
                };
                match update_attachment_description(&mut self.db, att.id, &input) {
                    Ok(updated) => {
                        let _ = self.refresh_all();
                        let _ = get_attachment(&self.db, updated.id);
                        self.status_message = format!("Updated attachment #{} alt description to \"{}\"", updated.id, updated.alt_text());
                    }
                    Err(err) => {
                        self.status_message = format!("Failed to update attachment: {err}");
                    }
                }
            }
            DetailPromptKind::JumpToTask => {
                if !input.is_empty() {
                    self.open_detail_for(&input);
                }
            }
            DetailPromptKind::ConfirmDeleteTask => {
                if !input.eq_ignore_ascii_case("yes") {
                    self.status_message = "Task delete cancelled".to_string();
                    return;
                }
                match delete_task(&mut self.db, &snapshot.task.id) {
                    Ok(()) => {
                        self.detail_modal = None;
                        let _ = self.refresh_all();
                        self.status_message = format!("Deleted task [{}] {}", snapshot.task.id, snapshot.task.title);
                    }
                    Err(err) => self.status_message = format!("Failed to delete task: {err}"),
                }
            }
            DetailPromptKind::ConfirmDeleteNote => {
                if !input.eq_ignore_ascii_case("yes") {
                    self.status_message = "Comment delete cancelled".to_string();
                    return;
                }
                let idx = self
                    .detail_note_cursor
                    .min(snapshot.notes.len().saturating_sub(1));
                let Some(note) = snapshot.notes.get(idx) else {
                    return;
                };
                match delete_task_note(&mut self.db, note.id) {
                    Ok(()) => {
                        let _ = self.refresh_all();
                        self.detail_note_cursor = 0;
                        self.status_message = format!("Deleted comment #{}", note.id);
                    }
                    Err(err) => self.status_message = format!("Failed to delete comment: {err}"),
                }
            }
        }
    }

    fn open_attachment_targets(&mut self, targets: &[&TaskAttachment]) {
        let mut opened = Vec::new();
        let mut errors = Vec::new();
        for att in targets {
            match open_file_in_os(Path::new(&att.file_path)) {
                Ok(()) => opened.push(format!("#{} ({})", att.id, att.file_name)),
                Err(err) => errors.push(format!("{}: {err}", att.file_path)),
            }
        }
        if errors.is_empty() {
            if let [att] = targets {
                self.status_message = format!("Opened attachment #{} ({}) at {}", att.id, att.alt_text(), att.file_path);
            } else {
                self.status_message = format!("Opened {} attachments: {}", opened.len(), opened.join(", "));
            }
        } else {
            self.status_message = format!("Failed to open: {}", errors.join("; "));
        }
    }

    /// Opens the acting-user prompt (`Ctrl+U`); `U` is the per-view filter.
    fn begin_user_switch(&mut self) {
        self.user_switching = true;
        self.user_input.clear();
        self.user_history.clear();
        self.update_user_prompt_message();
    }

    fn update_user_prompt_message(&mut self) {
        let known = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()])
            .unwrap_or_default()
            .join(", @");
        self.status_message = format!("Switch user (known: @{known}) — active @{}: @{}_", self.user, self.user_input);
    }

    fn handle_user_input_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.user_input, &mut self.user_history) {
            if changed {
                self.update_user_prompt_message();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.user_switching = false;
                self.user_input.clear();
                self.status_message = "Cancelled user switch".to_string();
            }
            KeyCode::Backspace => {
                self.user_input.pop();
                self.update_user_prompt_message();
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.user_input);
                self.user_switching = false;
                match normalize_user(&input).and_then(|u| register_user(&mut self.db, &u)) {
                    Ok(user) => {
                        let previous = std::mem::replace(&mut self.user, user.clone());
                        // Views filtered to the previous identity follow the new one.
                        for filter in [&mut self.focus_user, &mut self.kanban_user, &mut self.eisenhower_user] {
                            if *filter == previous {
                                *filter = user.clone();
                            }
                        }
                        if self.search_assignee_filter.as_deref() == Some(previous.as_str()) {
                            self.search_assignee_filter = Some(user.clone());
                        }
                        let _ = self.refresh_all();
                        self.status_message = format!("Active user is now @{user}");
                    }
                    Err(err) => self.status_message = format!("Cannot switch user: {err}"),
                }
            }
            KeyCode::Char(ch) => {
                self.user_input.push(ch);
                self.update_user_prompt_message();
            }
            _ => {}
        }
    }

    /// `✓ ` when `id` is part of the multi-selection, otherwise an empty string.
    pub fn mark(&self, id: &str) -> &'static str {
        if self.selected_ids.iter().any(|x| x == id) { "✓ " } else { "" }
    }

    fn highlighted_task_id(&self) -> Option<String> {
        match self.active_tab {
            ActiveTab::Kanban => self
                .kanban_tasks_in_col(self.kanban_col)
                .get(self.kanban_row)
                .map(|t| t.id.clone()),
            ActiveTab::Eisenhower => self
                .eisenhower_tasks_in_quadrant(self.eisenhower_quadrant)
                .get(self.eisenhower_row)
                .map(|c| c.task.id.clone()),
            ActiveTab::Database => self
                .search_hits
                .get(self.search_selected)
                .map(|h| h.task.id.clone()),
            _ => None,
        }
    }

    /// Space toggles selection, `A` selects all Database results, Esc clears, `b` opens the bulk-edit prompt.
    fn handle_selection_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(' ') => {
                if let Some(id) = self.highlighted_task_id() {
                    if let Some(pos) = self.selected_ids.iter().position(|x| *x == id) {
                        self.selected_ids.remove(pos);
                    } else {
                        self.selected_ids.push(id);
                    }
                    self.status_message = format!("{} selected", self.selected_ids.len());
                }
                true
            }
            KeyCode::Char('A') if self.active_tab == ActiveTab::Database => {
                self.selected_ids = self.search_hits.iter().map(|h| h.task.id.clone()).collect();
                self.status_message = format!("{} selected", self.selected_ids.len());
                true
            }
            KeyCode::Esc if !self.selected_ids.is_empty() => {
                self.selected_ids.clear();
                self.status_message = "Selection cleared".to_string();
                true
            }
            KeyCode::Char('b') => {
                if self.selected_ids.is_empty() && self.highlighted_task_id().is_none() {
                    self.status_message = "Nothing to edit".to_string();
                    return true;
                }
                self.bulk_editing = true;
                self.bulk_input.clear();
                self.bulk_history.clear();
                self.update_bulk_prompt_message();
                true
            }
            _ => false,
        }
    }

    fn update_bulk_prompt_message(&mut self) {
        let n = self.selected_ids.len().max(1);
        self.status_message = format!(
            "Bulk edit {n} task(s): status=todo assignee=bob importance=4 difficulty=2 urgent=on +tag -tag delete | {}_",
            self.bulk_input
        );
    }

    fn handle_bulk_input_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.bulk_input, &mut self.bulk_history) {
            if changed {
                self.update_bulk_prompt_message();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.bulk_editing = false;
                self.bulk_input.clear();
                self.status_message = "Cancelled bulk edit".to_string();
            }
            KeyCode::Backspace => {
                self.bulk_input.pop();
                self.update_bulk_prompt_message();
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut self.bulk_input);
                self.bulk_editing = false;
                self.apply_bulk_input(&input);
            }
            KeyCode::Char(ch) => {
                self.bulk_input.push(ch);
                self.update_bulk_prompt_message();
            }
            _ => {}
        }
    }

    fn apply_bulk_input(&mut self, input: &str) {
        let ids: Vec<String> =
            if self.selected_ids.is_empty() { self.highlighted_task_id().into_iter().collect() } else { self.selected_ids.clone() };
        if ids.is_empty() {
            return;
        }

        let mut patch = BulkTaskPatch::default();
        let mut delete = false;
        for token in input.split_whitespace() {
            let parsed: Result<(), String> = (|| {
                if token == "delete" {
                    delete = true;
                } else if let Some(tag) = token.strip_prefix('+') {
                    patch.add_tags.push(tag.to_string());
                } else if let Some(tag) = token.strip_prefix('-') {
                    patch.remove_tags.push(tag.to_string());
                } else if let Some((k, v)) = token.split_once('=') {
                    match k {
                        "status" | "s" => patch.status = Some(v.parse::<TaskStatus>().map_err(|e| e.to_string())?),
                        "assignee" | "a" => patch.assignee = Some(v.to_string()),
                        "importance" | "i" => patch.importance = Some(v.parse().map_err(|_| format!("bad importance '{v}'"))?),
                        "difficulty" | "d" => patch.difficulty = Some(v.parse().map_err(|_| format!("bad difficulty '{v}'"))?),
                        "urgent" | "u" => patch.urgent_override = Some(matches!(v, "on" | "true" | "yes" | "1")),
                        _ => return Err(format!("unknown field '{k}'")),
                    }
                } else {
                    return Err(format!("unrecognized '{token}'"));
                }
                Ok(())
            })();
            if let Err(msg) = parsed {
                self.status_message = format!("Bulk edit: {msg}");
                return;
            }
        }

        let result = if delete { bulk_delete_tasks(&mut self.db, &ids) } else { bulk_update_tasks(&mut self.db, &ids, &patch) };
        match result {
            Ok(outcome) => {
                let ok = outcome.updated.len() + outcome.deleted.len();
                let failed = outcome.failures.len();
                self.status_message = if failed == 0 {
                    format!("{} {ok} task(s)", if delete { "Deleted" } else { "Updated" })
                } else {
                    format!(
                        "{} {ok} task(s); {failed} failed ({})",
                        if delete { "Deleted" } else { "Updated" },
                        outcome
                            .failures
                            .iter()
                            .take(2)
                            .map(|f| format!("{}: {}", f.id, f.error))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                };
                if failed == 0 {
                    self.selected_ids.clear();
                }
                let _ = self.refresh_all();
            }
            Err(err) => self.status_message = format!("Bulk edit failed: {err}"),
        }
    }

    fn apply_status_transition(&mut self, task_id: &str, new_status: TaskStatus) {
        match transition_task_status(&mut self.db, task_id, new_status) {
            Ok(outcome) => {
                let _ = self.refresh_all();
                if let Some(ref rev) = outcome.spawned_review_task {
                    self.status_message =
                        format!("[{}] moved to in_review — spawned review subtask [{}] for @{}", outcome.task.id, rev.id, rev.assignee);
                } else if !outcome.auto_completed_parent_tasks.is_empty() {
                    let parents: Vec<String> = outcome
                        .auto_completed_parent_tasks
                        .iter()
                        .map(|p| p.id.clone())
                        .collect();
                    self.status_message =
                        format!("[{}] completed — auto-completed reviewed parent(s): {}", outcome.task.id, parents.join(", "));
                } else {
                    self.status_message = format!("[{}] status -> {}", outcome.task.id, outcome.task.status);
                }
            }
            Err(err) => {
                self.status_message = format!("Status transition blocked: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn modified_characters_are_never_text_or_shortcuts() {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            let event = key(KeyCode::Char('v'), modifiers);
            assert!(!is_text_input(&event), "{modifiers:?}+v must not be typed");
            assert!(is_modified_char(&event), "{modifiers:?}+v must not trigger a shortcut");
        }
    }

    #[test]
    fn plain_and_shifted_characters_are_text_input() {
        for event in [key(KeyCode::Char('v'), KeyModifiers::NONE), key(KeyCode::Char('V'), KeyModifiers::SHIFT)] {
            assert!(is_text_input(&event));
            assert!(!is_modified_char(&event));
        }
    }

    #[test]
    fn control_codepoints_are_not_text_input() {
        let event = key(KeyCode::Char('\u{8}'), KeyModifiers::NONE);
        assert!(!is_text_input(&event));
        assert!(is_modified_char(&event));
    }

    #[test]
    fn non_character_keys_are_neither_text_nor_modified_chars() {
        for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Backspace, KeyCode::Up] {
            let event = key(code, KeyModifiers::NONE);
            assert!(!is_text_input(&event));
            assert!(!is_modified_char(&event));
        }
    }

    #[test]
    fn database_sort_columns_cycle_in_table_order() {
        let mut column = DatabaseSortColumn::Relevance;
        let mut seen = vec![column];
        for _ in 0..10 {
            column = column.next();
            seen.push(column);
        }
        assert_eq!(
            seen[1..10],
            [
                DatabaseSortColumn::Id,
                DatabaseSortColumn::Title,
                DatabaseSortColumn::Status,
                DatabaseSortColumn::Project,
                DatabaseSortColumn::Assignee,
                DatabaseSortColumn::Importance,
                DatabaseSortColumn::Difficulty,
                DatabaseSortColumn::SgdbDate,
                DatabaseSortColumn::DueDate,
            ]
        );
        assert_eq!(seen[10], DatabaseSortColumn::Relevance);
    }
}
