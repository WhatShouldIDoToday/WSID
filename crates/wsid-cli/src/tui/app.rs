use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use wsid_core::{
    AppConfig, BulkTaskPatch, Database, DbChangeDetector, EisenhowerClassification, EisenhowerQuadrant, FieldIssue, FocusPreset,
    FocusQueueResponse, SearchHit, SearchQuery, Task, TaskAttachment, TaskNote, TaskStatus, UpdateTaskInput, UserDeletionPreview,
    WsidError, add_task_note, attach_existing_file, bulk_delete_tasks, bulk_update_tasks, compute_focus_queue_in_project,
    create_task_from_draft_with_notices, delete_task, delete_task_note, delete_user_from_db, get_attachment, get_direct_blocked_tasks,
    get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers, get_related_tasks, get_task, list_eisenhower_matrix,
    list_known_users, list_task_notes, list_tasks, normalize_project, normalize_user, open_file_in_os, parse_note_tokens,
    preview_user_deletion, register_user, remove_attachment, save_clipboard_attachment, search_relation_targets, search_tasks,
    suggest_users, transition_task_status, unblock_or_dismiss_subtask, update_attachment_description, update_task, validate_focus_limit,
};

use crate::cli_runner::read_system_clipboard_attachment;
pub use crate::tui::form::DataEntryForm;
use crate::tui::form::{self, CommentSuggestion, CommentTokenKind, trailing_comment_token};
use crate::tui::text_edit::{self, EditKind, TextHistory, apply_text_shortcut};
use crate::tui::user_ref::{user_kind_mismatch, user_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Focus,
    Kanban,
    Eisenhower,
    Database,
    DataEntry,
    Settings,
}

impl ActiveTab {
    pub const ALL: [Self; 6] = [Self::Focus, Self::Kanban, Self::Eisenhower, Self::Database, Self::DataEntry, Self::Settings];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Kanban => "Kanban",
            Self::Eisenhower => "Eisenhower",
            Self::Database => "Database",
            Self::DataEntry => "Add Task",
            Self::Settings => "Settings",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Focus => 0,
            Self::Kanban => 1,
            Self::Eisenhower => 2,
            Self::Database => 3,
            Self::DataEntry => 4,
            Self::Settings => 5,
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
    /// The Settings user-kind picker.
    KindModal,
    /// The user filter modal (per-view user scope) owns input while it is open.
    UserModal,
    /// A prompt inside the task detail modal (comment, attach, jump, delete...).
    DetailPrompt(DetailPromptKind),
    /// The attachment list inside the task detail modal has focus.
    DetailAttachments,
    /// The task detail modal itself, with no prompt open.
    DetailModal,
    /// The delete-user confirmation dialog opened from Settings.
    UserDeleteModal,
    SearchInput,
    BulkEditPrompt,
    Tab(ActiveTab),
}

/// How a user or project picker was opened.
///
/// `Filter` (`U` / `P`) scopes the current view and may offer `all`. `Switch` (`Ctrl+U` /
/// `Ctrl+P`) changes the session's active user or project, so it always names one concrete value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerMode {
    #[default]
    Filter,
    Switch,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsMode {
    #[default]
    Browse,
    EditFocusLimit,
    EditDatabasePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    SwitchUser,
    SwitchProject,
    CyclePreset,
    EditFocusLimit,
    OpenGlobalDatabase,
    OpenWorkspaceDatabase,
    OpenRecentDatabase(PathBuf),
    DeleteUser(String),
    About,
}

#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub section: &'static str,
    pub label: String,
    pub value: String,
    pub action: SettingsAction,
}

#[derive(Debug, Clone)]
pub struct UserDeleteConfirm {
    pub preview: UserDeletionPreview,
    pub delete_authored: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub selected: usize,
    pub mode: SettingsMode,
    pub focus_limit_input: String,
    pub database_path_input: String,
    pub user_kind_target: String,
    /// Search text of the kind picker.
    pub user_kind_input: String,
    pub kind_modal_open: bool,
    pub kind_selected: usize,
    pub delete_confirm: Option<UserDeleteConfirm>,
    focus_limit_history: TextHistory,
    database_path_history: TextHistory,
    user_kind_history: TextHistory,
}

pub struct TuiApp {
    pub db: Database,
    pub db_path: PathBuf,
    pub config: AppConfig,
    pub config_path: PathBuf,
    /// The acting user (`Ctrl+U`). Session-only.
    pub user: String,
    /// The session's project context (`Ctrl+P`). Session-only; defaults to `default`.
    pub active_project: String,
    pub preset: FocusPreset,
    pub accessible_mode: bool,
    pub should_quit: bool,
    pub active_tab: ActiveTab,
    pub status_message: String,
    pub settings: SettingsState,

    pub change_detector: DbChangeDetector,

    // Focus View state
    pub focus_queue: FocusQueueResponse,
    pub focus_selected: usize,
    pub focus_state: ListState,
    pub show_blocked_banner: bool,

    // Kanban View state
    pub all_tasks: Vec<Task>,
    /// Cached `list_user_kinds`, refreshed with the task lists.
    pub user_kinds: std::collections::BTreeMap<String, String>,
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

    // Project picker modal: a per-view filter (`P`) or the active project switcher (`Ctrl+P`)
    pub project_modal_open: bool,
    pub project_modal_mode: PickerMode,
    pub project_modal_query: String,
    pub project_modal_selected: usize,

    // User picker modal: a per-view filter (`U`) or the active user switcher (`Ctrl+U`)
    pub user_modal_open: bool,
    pub user_modal_mode: PickerMode,
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
    bulk_history: TextHistory,
    detail_prompt_history: TextHistory,
}

impl TuiApp {
    pub fn new(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<Self> {
        Self::new_with_config_path(db_path, config, user, accessible_mode, wsid_core::default_config_path())
    }

    pub fn new_with_config_path(
        db_path: &Path,
        config: AppConfig,
        user: String,
        accessible_mode: bool,
        config_path: PathBuf,
    ) -> wsid_core::Result<Self> {
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
        let user_kinds = wsid_core::list_user_kinds(&db)?;
        let eisenhower_items = list_eisenhower_matrix(&db, Some(&user), today, config.scoring)?;
        let search_hits = search_tasks(&db, &SearchQuery::default())?;
        let active_project = wsid_core::DEFAULT_PROJECT.to_string();
        let entry_form = DataEntryForm::new_for(&user, &active_project);

        let mut app = Self {
            db,
            db_path: db_path.to_path_buf(),
            config,
            config_path,
            user: user.clone(),
            active_project,
            preset,
            accessible_mode,
            should_quit: false,
            active_tab: ActiveTab::Focus,
            status_message: String::new(),
            settings: SettingsState::default(),
            change_detector,
            focus_queue,
            focus_selected: 0,
            focus_state: ListState::default(),
            show_blocked_banner: true,
            all_tasks,
            user_kinds,
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
            project_modal_mode: PickerMode::Filter,
            project_modal_query: String::new(),
            project_modal_selected: 0,
            user_modal_open: false,
            user_modal_mode: PickerMode::Filter,
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
            about_modal_open: false,
            about_scroll: 0,
            editing_task_id: None,
            editing_return_tab: None,
            project_modal_history: TextHistory::default(),
            user_modal_history: TextHistory::default(),
            search_history: TextHistory::default(),
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
        self.user_kinds = wsid_core::list_user_kinds(&self.db)?;
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
        } else if self.settings.kind_modal_open {
            FocusContext::KindModal
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
        } else if self.settings.delete_confirm.is_some() {
            FocusContext::UserDeleteModal
        } else if self.active_tab == ActiveTab::Database && self.search_editing {
            FocusContext::SearchInput
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

        let context = self.focus_context();
        if let FocusContext::Tab(tab) = context
            && self.handle_context_switch_key(tab, key)
        {
            return;
        }

        match context {
            FocusContext::FormErrorModal => self.dismiss_form_errors(key),
            FocusContext::AboutModal => self.handle_about_modal_key(key),
            FocusContext::ProjectModal => self.handle_project_modal_key(key),
            FocusContext::KindModal => self.handle_kind_modal_key(key),
            FocusContext::UserModal => self.handle_user_modal_key(key),
            FocusContext::DetailPrompt(_) | FocusContext::DetailAttachments | FocusContext::DetailModal => {
                self.handle_detail_modal_key(key);
            }
            FocusContext::UserDeleteModal => self.handle_user_delete_confirm_key(key),
            FocusContext::SearchInput => self.handle_search_input_key(key),
            FocusContext::BulkEditPrompt => self.handle_bulk_input_key(key),
            FocusContext::Tab(ActiveTab::DataEntry) => self.handle_entry_tab_key(key),
            FocusContext::Tab(ActiveTab::Settings) => self.handle_settings_key(key),
            FocusContext::Tab(_) => self.handle_list_tab_key(key),
        }
    }

    /// `Ctrl+U` / `Ctrl+P` switch the active user / project from any view, so they are handled
    /// before the view's own keys. Text inputs and Settings edit modes keep their own use of them.
    fn handle_context_switch_key(&mut self, tab: ActiveTab, key: KeyEvent) -> bool {
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        if tab == ActiveTab::Settings && self.settings.mode != SettingsMode::Browse {
            return false;
        }
        match key.code {
            KeyCode::Char('u') => {
                self.open_user_switcher();
                true
            }
            KeyCode::Char('p') => {
                self.open_project_switcher();
                true
            }
            _ => false,
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
                self.set_active_tab(ActiveTab::Focus);
                self.status_message = "Switched to Focus View".to_string();
            }
            return;
        }
        self.handle_data_entry_key(key);
    }

    /// Switch views through one path so stale multi-select marks never follow
    /// the user to another view. Returning to the same view keeps them.
    pub fn set_active_tab(&mut self, tab: ActiveTab) {
        if self.active_tab != tab {
            self.selected_ids.clear();
            self.bulk_editing = false;
            self.bulk_input.clear();
            self.active_tab = tab;
            self.apply_view_scope(tab);
        }
        self.active_tab = tab;
        self.status_message.clear();
    }

    /// Resets a view's temporary user/project scope when it is entered from another view:
    /// Focus and Database show all projects, Kanban and Eisenhower show the active project, and
    /// every view that scopes users follows the active user. Database search text, status, tag,
    /// and sort are kept. Add Task and Settings have no view scope.
    fn apply_view_scope(&mut self, tab: ActiveTab) {
        let user = self.user.clone();
        let project = self.active_project.clone();
        match tab {
            ActiveTab::Focus => {
                let changed = self.focus_user != user || self.focus_project != wsid_core::ALL_PROJECTS;
                self.focus_user = user;
                self.focus_project = wsid_core::ALL_PROJECTS.to_string();
                if changed {
                    let _ = self.refresh_all();
                }
            }
            ActiveTab::Kanban => {
                self.kanban_user = user;
                self.kanban_project = project;
                self.clamp_kanban_selection();
            }
            ActiveTab::Eisenhower => {
                let changed = self.eisenhower_user != user || self.eisenhower_project != project;
                self.eisenhower_user = user;
                self.eisenhower_project = project;
                if changed {
                    let _ = self.refresh_all();
                }
            }
            ActiveTab::Database => {
                let changed = self.search_assignee_filter.is_some() || self.search_project_filter.is_some() || self.search_only_mine;
                self.search_assignee_filter = None;
                self.search_project_filter = None;
                self.search_only_mine = false;
                if changed {
                    let _ = self.refresh_search();
                }
            }
            ActiveTab::DataEntry | ActiveTab::Settings => {}
        }
    }

    /// Handles the view-switch shortcuts shared by the list views and Settings.
    fn handle_tab_switch_key(&mut self, code: KeyCode) -> bool {
        let tab = match code {
            KeyCode::Char('1') => ActiveTab::Focus,
            KeyCode::Char('2') => ActiveTab::Kanban,
            KeyCode::Char('3') => ActiveTab::Eisenhower,
            KeyCode::Char('4') => ActiveTab::Database,
            KeyCode::Char('5') | KeyCode::Char('n') => ActiveTab::DataEntry,
            KeyCode::Char('6') | KeyCode::Char(',') => ActiveTab::Settings,
            _ => return false,
        };
        self.set_active_tab(tab);
        true
    }

    /// Shortcuts shared by every list view, then the active view's own keys.
    fn handle_list_tab_key(&mut self, key: KeyEvent) {
        // Modified characters belong to the terminal/OS, so they never fire
        // single-letter shortcuts (e.g. Ctrl+A does not toggle accessible mode).
        if is_modified_char(&key) {
            return;
        }
        if self.handle_tab_switch_key(key.code) {
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
            ActiveTab::DataEntry | ActiveTab::Settings => {}
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
                self.preset = self.preset.next();
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
            ActiveTab::Settings => wsid_core::ALL_PROJECTS.to_string(),
        }
    }

    /// Projects offered by a picker in `mode`: `all` only as a filter, then every known project.
    pub fn project_picker_options(&self, mode: PickerMode) -> Vec<(String, String)> {
        let mut options = Vec::new();
        if mode == PickerMode::Filter {
            options.push((wsid_core::ALL_PROJECTS.to_string(), format!("{} (every project)", wsid_core::ALL_PROJECTS)));
        }
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

    /// The project the open picker treats as current: the view's filter, or the active project.
    pub fn project_modal_current(&self) -> String {
        match self.project_modal_mode {
            PickerMode::Filter => self.current_project_filter(),
            PickerMode::Switch => self.active_project.clone(),
        }
    }

    /// `project_picker_options` for the open picker, narrowed by its live search text. A valid
    /// project name that is not known yet is offered too, so a new project can be selected before
    /// it has any tasks.
    pub(crate) fn filtered_project_options(&self) -> Vec<(String, String)> {
        let needle = self.project_modal_query.trim().to_ascii_lowercase();
        let mut options: Vec<(String, String)> = self
            .project_picker_options(self.project_modal_mode)
            .into_iter()
            .filter(|(value, label)| {
                needle.is_empty() || value.to_ascii_lowercase().contains(&needle) || label.to_ascii_lowercase().contains(&needle)
            })
            .collect();
        let typed = self.project_modal_query.trim();
        if !typed.is_empty()
            && let Ok(normalized) = normalize_project(typed)
            && !options.iter().any(|(value, _)| *value == normalized)
        {
            options.push((normalized.clone(), normalized));
        }
        options
    }

    fn open_project_modal(&mut self) {
        self.open_project_picker(PickerMode::Filter);
    }

    /// Opens the active project switcher (`Ctrl+P`): single projects only, no `all`.
    pub fn open_project_switcher(&mut self) {
        self.open_project_picker(PickerMode::Switch);
    }

    fn open_project_picker(&mut self, mode: PickerMode) {
        self.project_modal_open = true;
        self.project_modal_mode = mode;
        self.project_modal_query.clear();
        self.project_modal_history.clear();
        let current = self.project_modal_current();
        let options = self.filtered_project_options();
        self.project_modal_selected = options
            .iter()
            .position(|(value, _)| *value == current)
            .unwrap_or(0);
        self.status_message = match mode {
            PickerMode::Filter => "Project filter — type to search, Up/Down to move, Enter to apply, Esc to cancel".to_string(),
            PickerMode::Switch => format!("Switch active project (currently {}) — Enter to switch, Esc to cancel", self.active_project),
        };
    }

    /// Makes `raw` the active project. Kanban and Eisenhower move to it, and pristine Add Task
    /// fields follow it. Returns whether the switch happened.
    pub fn switch_active_project(&mut self, raw: &str) -> bool {
        let project = match normalize_project(raw) {
            Ok(project) => project,
            Err(err) => {
                self.status_message = format!("Cannot switch project: {err}");
                return false;
            }
        };
        let previous = std::mem::replace(&mut self.active_project, project.clone());
        self.kanban_project = project.clone();
        self.eisenhower_project = project.clone();
        if self.editing_task_id.is_none() {
            self.entry_form
                .retarget_context(&self.user, &previous, &self.user, &project);
        }
        self.clamp_kanban_selection();
        let _ = self.refresh_all();
        self.status_message = format!("Active project is now {project}");
        true
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
            ActiveTab::Settings => {}
        }
        self.status_message = format!("Project filter: {project}");
    }

    fn handle_project_modal_key(&mut self, key: KeyEvent) {
        // `Ctrl+P` inside the picker (re)opens the active project switcher.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            self.open_project_switcher();
            return;
        }
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
                self.status_message = match self.project_modal_mode {
                    PickerMode::Filter => "Cancelled project filter".to_string(),
                    PickerMode::Switch => "Cancelled project switch".to_string(),
                };
            }
            KeyCode::Enter => {
                if let Some((project, _)) = self
                    .filtered_project_options()
                    .get(self.project_modal_selected)
                    .cloned()
                {
                    self.project_modal_open = false;
                    match self.project_modal_mode {
                        PickerMode::Filter => self.apply_project_filter(project),
                        PickerMode::Switch => {
                            self.switch_active_project(&project);
                        }
                    }
                } else {
                    self.status_message = "No matching project — type a name to create one".to_string();
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
            ActiveTab::Settings => self.user.clone(),
        }
    }

    /// Focus shows one user's queue, so `all` is not offered there (nor on the form).
    pub fn user_filter_allows_all(&self) -> bool {
        matches!(self.active_tab, ActiveTab::Kanban | ActiveTab::Eisenhower | ActiveTab::Database)
    }

    /// Users offered by a picker in `mode`. `all` appears only in a view filter that allows it;
    /// the active user switcher (`Ctrl+U`) always names a single user.
    pub fn user_picker_options(&self, mode: PickerMode) -> Vec<(String, String)> {
        let allow_all = mode == PickerMode::Filter && self.user_filter_allows_all();
        let mut options = Vec::new();
        if allow_all {
            options.push((wsid_core::ALL_USERS.to_string(), format!("{} (every user)", wsid_core::ALL_USERS)));
        }
        let mut users = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()]).unwrap_or_default();
        users.sort();
        users.dedup();
        let kinds = wsid_core::list_user_kinds(&self.db).unwrap_or_default();
        for user in users {
            if user == wsid_core::ALL_USERS {
                continue;
            }
            let kind = kinds
                .get(&user)
                .map(String::as_str)
                .unwrap_or(wsid_core::DEFAULT_USER_KIND);
            let kind_tag = if kind == wsid_core::DEFAULT_USER_KIND { String::new() } else { format!(" · {kind}") };
            let label = if user == self.user { format!("@{user}{kind_tag} (active)") } else { format!("@{user}{kind_tag}") };
            options.push((user, label));
        }
        let current = self.user_modal_current_for(mode);
        if mode == PickerMode::Filter
            && !current.is_empty()
            && !options
                .iter()
                .any(|(value, _)| value.eq_ignore_ascii_case(&current))
        {
            options.push((current.clone(), format!("@{current}")));
        }
        options
    }

    /// The user the open picker treats as current: the view's filter, or the active user.
    pub fn user_modal_current(&self) -> String {
        self.user_modal_current_for(self.user_modal_mode)
    }

    fn user_modal_current_for(&self, mode: PickerMode) -> String {
        match mode {
            PickerMode::Filter => self.current_user_filter(),
            PickerMode::Switch => self.user.clone(),
        }
    }

    /// `user_picker_options` for the open picker, narrowed by its live search text. A valid user
    /// name that is not known yet is offered too, so `@someone` can be selected before their first task.
    pub(crate) fn filtered_user_options(&self) -> Vec<(String, String)> {
        let needle = self
            .user_modal_query
            .trim()
            .trim_start_matches('@')
            .to_ascii_lowercase();
        let mut options: Vec<(String, String)> = self
            .user_picker_options(self.user_modal_mode)
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
        self.open_user_picker(PickerMode::Filter);
    }

    /// Opens the active user switcher (`Ctrl+U`): single users only, no `all`.
    pub fn open_user_switcher(&mut self) {
        self.open_user_picker(PickerMode::Switch);
    }

    fn open_user_picker(&mut self, mode: PickerMode) {
        self.user_modal_open = true;
        self.user_modal_mode = mode;
        self.user_modal_query.clear();
        self.user_modal_history.clear();
        let current = self.user_modal_current();
        let options = self.filtered_user_options();
        self.user_modal_selected = options
            .iter()
            .position(|(value, _)| *value == current)
            .unwrap_or(0);
        self.status_message = match mode {
            PickerMode::Switch => format!("Switch active user (currently @{}) — Enter to switch, Esc to cancel", self.user),
            PickerMode::Filter if self.user_filter_allows_all() => {
                "User filter — type to search, Up/Down to move, Enter to apply, Esc to cancel".to_string()
            }
            PickerMode::Filter => format!("User filter (single user) — active @{}", self.current_user_filter()),
        };
    }

    /// Makes `raw` (registering it if new) the active user. Views that followed the previous
    /// active user follow the new one; explicit filters on other users stay. Returns whether the
    /// switch happened.
    pub fn switch_active_user(&mut self, raw: &str) -> bool {
        let user = match normalize_user(raw).and_then(|u| register_user(&mut self.db, &u)) {
            Ok(user) => user,
            Err(err) => {
                self.status_message = format!("Cannot switch user: {err}");
                return false;
            }
        };
        let previous = std::mem::replace(&mut self.user, user.clone());
        for filter in [&mut self.focus_user, &mut self.kanban_user, &mut self.eisenhower_user] {
            if *filter == previous {
                *filter = user.clone();
            }
        }
        if self.search_assignee_filter.as_deref() == Some(previous.as_str()) {
            self.search_assignee_filter = Some(user.clone());
        }
        if self.editing_task_id.is_none() {
            self.entry_form
                .retarget_context(&previous, &self.active_project, &user, &self.active_project);
        }
        let _ = self.refresh_all();
        self.status_message = format!("Active user is now @{user}");
        true
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
            ActiveTab::Settings => {}
        }
        self.status_message =
            if every_user { format!("User filter: {} (every user)", wsid_core::ALL_USERS) } else { format!("User filter: @{user}") };
    }

    fn handle_user_modal_key(&mut self, key: KeyEvent) {
        // `Ctrl+U` inside the picker (re)opens the active user switcher.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            self.open_user_switcher();
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
                self.status_message = match self.user_modal_mode {
                    PickerMode::Filter => "Cancelled user filter".to_string(),
                    PickerMode::Switch => "Cancelled user switch".to_string(),
                };
            }
            KeyCode::Enter => {
                if let Some((user, _)) = self
                    .filtered_user_options()
                    .get(self.user_modal_selected)
                    .cloned()
                {
                    self.user_modal_open = false;
                    match self.user_modal_mode {
                        PickerMode::Filter => self.apply_user_filter(user),
                        PickerMode::Switch => {
                            self.switch_active_user(&user);
                        }
                    }
                } else {
                    self.status_message = "No matching user — type a name to add one".to_string();
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
            ActiveTab::Settings => Vec::new(),
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
            FocusContext::KindModal => {
                let before = self.settings.user_kind_input.clone();
                self.settings
                    .user_kind_input
                    .push_str(&text_edit::flatten_paste(text));
                self.settings
                    .user_kind_history
                    .record(&before, EditKind::Discrete);
                self.settings.kind_selected = 0;
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
            FocusContext::Tab(ActiveTab::Settings) => {
                let flat = text_edit::flatten_paste(text);
                match self.settings.mode {
                    SettingsMode::EditFocusLimit => {
                        let digits: String = flat.chars().filter(char::is_ascii_digit).collect();
                        if !digits.is_empty() {
                            let before = self.settings.focus_limit_input.clone();
                            self.settings.focus_limit_input.push_str(&digits);
                            self.settings.focus_limit_input.truncate(2);
                            self.settings
                                .focus_limit_history
                                .record(&before, EditKind::Discrete);
                            self.update_focus_limit_prompt();
                        }
                    }
                    SettingsMode::EditDatabasePath => {
                        let before = self.settings.database_path_input.clone();
                        self.settings.database_path_input.push_str(&flat);
                        self.settings
                            .database_path_history
                            .record(&before, EditKind::Discrete);
                        self.update_database_path_prompt();
                    }
                    SettingsMode::Browse => {}
                }
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
        self.entry_form = DataEntryForm::new_for(&self.user, &self.active_project);
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
        match create_task_from_draft_with_notices(&mut self.db, &draft, &self.user) {
            Ok((created, notices)) => {
                self.entry_form = DataEntryForm::new_for(&self.user, &self.active_project);
                let _ = self.refresh_all();
                self.status_message = Self::with_user_notices(format!("Created task [{}] {}", created.id, created.title), &notices);
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

    /// Appends typed-reference notices (`@kind:user`) to a status message.
    fn with_user_notices(message: String, notices: &[wsid_core::UserNotice]) -> String {
        notices
            .iter()
            .fold(message, |acc, notice| format!("{acc}. {}", notice.message()))
    }

    /// Applies an edited form to the task it was opened for and returns to its detail modal.
    fn submit_edited_task(&mut self, id: &str, draft: &wsid_core::TaskDraft) {
        match wsid_core::update_task_from_draft_with_notices(&mut self.db, id, draft, &self.user) {
            Ok((updated, notices)) => {
                self.editing_task_id = None;
                self.entry_form = DataEntryForm::new_for(&self.user, &self.active_project);
                if let Some(tab) = self.editing_return_tab.take() {
                    self.active_tab = tab;
                }
                let _ = self.refresh_all();
                self.open_detail_for(&updated.id);
                self.status_message = Self::with_user_notices(format!("Updated task [{}] {}", updated.id, updated.title), &notices);
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
        let kinds = wsid_core::list_user_kinds(&self.db).unwrap_or_default();
        for user in users.into_iter().take(6) {
            let hint = match kinds.get(&user) {
                Some(kind) if kind != wsid_core::DEFAULT_USER_KIND => format!("known user · {kind}"),
                _ => "known user".to_string(),
            };
            self.detail_suggestions
                .push(CommentSuggestion { insert: format!("{scope}{user}"), label: format!("{scope}{user}"), hint });
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
                        self.status_message = Self::with_user_notices(
                            format!(
                                "Posted note #{} on [{}] (spawned {} task(s))",
                                outcome.note.id,
                                snapshot.task.id,
                                outcome.spawned_tasks.len()
                            ),
                            &outcome.user_notices,
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

    // ------------------------------------------------------------------
    // Settings View
    // ------------------------------------------------------------------

    pub fn is_primary_user(&self) -> bool {
        self.user.eq_ignore_ascii_case(&self.config.primary_user)
    }

    pub fn is_global_database(&self) -> bool {
        self.db_path == wsid_core::default_global_db_path()
    }

    /// `@user`, or `@user (kind)` for non-human users, for compact displays.
    pub fn user_label(&self, user: &str) -> String {
        user_label(&self.user_kinds, user)
    }

    /// `kind · ` for a non-default kind, otherwise empty.
    fn kind_prefix(kinds: &std::collections::BTreeMap<String, String>, user: &str) -> String {
        match kinds.get(user) {
            Some(kind) if kind != wsid_core::DEFAULT_USER_KIND => format!("{kind} · "),
            _ => String::new(),
        }
    }

    /// Rows rendered by the Settings view and navigated with `j`/`k`.
    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        let mut rows = Vec::new();

        rows.push(SettingsRow {
            section: "Active Context",
            label: "Active user".to_string(),
            value: format!("@{}  (Enter to switch)", self.user),
            action: SettingsAction::SwitchUser,
        });
        rows.push(SettingsRow {
            section: "Active Context",
            label: "Active project".to_string(),
            value: format!("{}  (Enter to switch)", self.active_project),
            action: SettingsAction::SwitchProject,
        });
        rows.push(SettingsRow {
            section: "Focus Queue",
            label: "Default focus preset".to_string(),
            value: self.preset.to_string(),
            action: SettingsAction::CyclePreset,
        });
        rows.push(SettingsRow {
            section: "Focus Queue",
            label: "Focus queue size".to_string(),
            value: format!("{} tasks", self.config.focus_limit),
            action: SettingsAction::EditFocusLimit,
        });

        let global_path = wsid_core::default_global_db_path();
        if self.is_global_database() {
            rows.push(SettingsRow {
                section: "Database Connection",
                label: "Global database".to_string(),
                value: format!("{}  (in use)", global_path.display()),
                action: SettingsAction::None,
            });
        } else {
            rows.push(SettingsRow {
                section: "Database Connection",
                label: "Use global database".to_string(),
                value: global_path.display().to_string(),
                action: SettingsAction::OpenGlobalDatabase,
            });
        }
        rows.push(SettingsRow {
            section: "Database Connection",
            label: "Open workspace database…".to_string(),
            value: "enter a .db file or a folder".to_string(),
            action: SettingsAction::OpenWorkspaceDatabase,
        });
        for recent in &self.config.recent_workspaces {
            let is_current = *recent == self.db_path;
            rows.push(SettingsRow {
                section: "Database Connection",
                label: "Recent workspace".to_string(),
                value: if is_current { format!("{}  (current)", recent.display()) } else { recent.display().to_string() },
                action: if is_current { SettingsAction::None } else { SettingsAction::OpenRecentDatabase(recent.clone()) },
            });
        }

        if self.is_primary_user() {
            let mut users = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()]).unwrap_or_default();
            users.sort_by_key(|user| user.to_ascii_lowercase());
            let kinds = wsid_core::list_user_kinds(&self.db).unwrap_or_default();
            if users.is_empty() {
                rows.push(SettingsRow {
                    section: "User Management",
                    label: "Known users".to_string(),
                    value: "none yet".to_string(),
                    action: SettingsAction::None,
                });
            }
            for known in users {
                if known.eq_ignore_ascii_case(&self.config.primary_user) {
                    rows.push(SettingsRow {
                        section: "User Management",
                        label: format!("@{known}"),
                        value: format!("{}primary", Self::kind_prefix(&kinds, &known)),
                        action: SettingsAction::None,
                    });
                } else {
                    rows.push(SettingsRow {
                        section: "User Management",
                        label: format!("@{known}"),
                        value: format!("{}Enter to delete", Self::kind_prefix(&kinds, &known)),
                        action: SettingsAction::DeleteUser(known),
                    });
                }
            }
        } else {
            rows.push(SettingsRow {
                section: "User Management",
                label: "Known users".to_string(),
                value: format!("only @{} can manage users", self.config.primary_user),
                action: SettingsAction::None,
            });
        }

        rows.push(SettingsRow {
            section: "About",
            label: "About WSID & licenses…".to_string(),
            value: String::new(),
            action: SettingsAction::About,
        });
        rows
    }

    pub(crate) fn clamp_settings_selection(&mut self, len: usize) {
        if len == 0 {
            self.settings.selected = 0;
        } else if self.settings.selected >= len {
            self.settings.selected = len - 1;
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match self.settings.mode {
            SettingsMode::EditFocusLimit => {
                self.handle_settings_focus_limit_key(key);
                return;
            }
            SettingsMode::EditDatabasePath => {
                self.handle_settings_database_path_key(key);
                return;
            }
            SettingsMode::Browse => {}
        }

        if is_modified_char(&key) {
            return;
        }
        if self.handle_tab_switch_key(key.code) {
            return;
        }

        let len = self.settings_rows().len();
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('a') => {
                self.accessible_mode = !self.accessible_mode;
                self.status_message = format!("Accessible linear layout mode: {}", if self.accessible_mode { "ON" } else { "OFF" });
            }
            KeyCode::Char('?') => {
                self.about_modal_open = true;
                self.about_scroll = 0;
                self.status_message = "About WSID & Third-Party Licenses".to_string();
            }
            KeyCode::Char('r') => {
                if let Err(err) = self.refresh_all() {
                    self.status_message = format!("Refresh failed: {err}");
                } else {
                    self.status_message = "Refreshed database state".to_string();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.settings.selected = (self.settings.selected + 1) % len;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if len > 0 {
                    self.settings.selected = self.settings.selected.checked_sub(1).unwrap_or(len - 1);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => self.settings.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.settings.selected = len.saturating_sub(1),
            KeyCode::Enter => self.activate_settings_row(),
            KeyCode::Char('t') => self.begin_user_kind_prompt(),
            _ => {}
        }
    }

    fn begin_user_kind_prompt(&mut self) {
        let rows = self.settings_rows();
        let Some(row) = rows.get(self.settings.selected) else {
            return;
        };
        let Some(user) = row
            .section
            .eq("User Management")
            .then(|| row.label.strip_prefix('@'))
            .flatten()
        else {
            return;
        };
        self.settings.user_kind_target = user.to_string();
        self.settings.kind_modal_open = false;
        self.settings.kind_selected = 0;
        self.settings.user_kind_input.clear();
        self.settings.user_kind_history.clear();
        self.settings.kind_modal_open = true;
        let current = self.kind_modal_current();
        self.settings.kind_selected = self
            .filtered_kind_options()
            .iter()
            .position(|(value, _)| *value == current)
            .unwrap_or(0);
        self.status_message = format!("Kind for @{user} — type to search or add a kind, Enter to apply, Esc to cancel");
    }

    /// The target user's current kind.
    pub fn kind_modal_current(&self) -> String {
        self.user_kinds
            .get(&self.settings.user_kind_target)
            .cloned()
            .unwrap_or_else(|| wsid_core::DEFAULT_USER_KIND.to_string())
    }

    /// Known kinds (always including `human`) narrowed by the search text, plus the typed text as a new kind when valid.
    pub(crate) fn filtered_kind_options(&self) -> Vec<(String, String)> {
        let mut kinds: Vec<String> = self.user_kinds.values().cloned().collect();
        kinds.push(wsid_core::DEFAULT_USER_KIND.to_string());
        kinds.sort();
        kinds.dedup();
        let typed = self.settings.user_kind_input.trim().to_ascii_lowercase();
        let mut options: Vec<(String, String)> = kinds
            .into_iter()
            .filter(|kind| typed.is_empty() || kind.contains(&typed))
            .map(|kind| {
                let count = self.user_kinds.values().filter(|k| **k == kind).count();
                let label = match count {
                    0 => kind.clone(),
                    1 => format!("{kind} · 1 user"),
                    n => format!("{kind} · {n} users"),
                };
                (kind, label)
            })
            .collect();
        if !typed.is_empty()
            && let Ok(normalized) = wsid_core::normalize_user_kind(&typed)
            && !options.iter().any(|(value, _)| *value == normalized)
        {
            options.push((normalized.clone(), format!("{normalized} · new kind")));
        }
        options
    }

    fn handle_kind_modal_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.settings.user_kind_input, &mut self.settings.user_kind_history) {
            if changed {
                self.settings.kind_selected = 0;
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.close_kind_modal();
                self.status_message = "Cancelled kind change".to_string();
            }
            KeyCode::Enter => {
                if let Some((kind, _)) = self
                    .filtered_kind_options()
                    .get(self.settings.kind_selected)
                    .cloned()
                {
                    self.submit_user_kind(&kind);
                } else {
                    self.status_message = "No matching kind — type a name to add one".to_string();
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                let len = self.filtered_kind_options().len();
                if len > 0 {
                    self.settings.kind_selected = (self.settings.kind_selected + 1) % len;
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                let len = self.filtered_kind_options().len();
                if len > 0 {
                    self.settings.kind_selected = self
                        .settings
                        .kind_selected
                        .checked_sub(1)
                        .unwrap_or(len - 1);
                }
            }
            KeyCode::Backspace => {
                self.settings.user_kind_input.pop();
                self.settings.kind_selected = 0;
            }
            KeyCode::Char(ch) => {
                self.settings.user_kind_input.push(ch);
                self.settings.kind_selected = 0;
            }
            _ => {}
        }
    }

    fn close_kind_modal(&mut self) {
        self.settings.kind_modal_open = false;
        self.settings.user_kind_input.clear();
        self.settings.kind_selected = 0;
        self.settings.user_kind_history.clear();
    }

    fn submit_user_kind(&mut self, kind: &str) {
        let user = self.settings.user_kind_target.clone();
        // Users known only from task fields have no row yet; register them first.
        let result = wsid_core::register_user(&mut self.db, &user).and_then(|_| wsid_core::set_user_kind(&mut self.db, &user, kind));
        match result {
            Ok(()) => {
                self.close_kind_modal();
                let _ = self.refresh_all();
                self.status_message = format!("Set @{user} kind to {kind}");
            }
            Err(err) => self.status_message = format!("Cannot set kind: {err}"),
        }
    }

    fn activate_settings_row(&mut self) {
        let rows = self.settings_rows();
        if rows.is_empty() {
            return;
        }
        let index = self.settings.selected.min(rows.len() - 1);
        self.settings.selected = index;
        match rows[index].action.clone() {
            SettingsAction::None => {}
            SettingsAction::SwitchUser => self.open_user_switcher(),
            SettingsAction::SwitchProject => self.open_project_switcher(),
            SettingsAction::CyclePreset => {
                self.preset = self.preset.next();
                self.config.focus_preset = self.preset;
                if let Err(err) = self.config.save_to_path(&self.config_path) {
                    self.status_message = format!("Failed to save settings: {err}");
                    return;
                }
                if let Err(err) = self.refresh_all() {
                    self.status_message = format!("Refresh failed: {err}");
                    return;
                }
                self.status_message = format!("Default focus preset set to '{}'", self.preset);
            }
            SettingsAction::EditFocusLimit => {
                self.settings.mode = SettingsMode::EditFocusLimit;
                self.settings.focus_limit_input = self.config.focus_limit.to_string();
                self.settings.focus_limit_history.clear();
                self.update_focus_limit_prompt();
            }
            SettingsAction::OpenGlobalDatabase => {
                let path = wsid_core::default_global_db_path();
                self.finish_database_switch(&path);
            }
            SettingsAction::OpenWorkspaceDatabase => self.begin_database_path_prompt(),
            SettingsAction::OpenRecentDatabase(path) => self.finish_database_switch(&path),
            SettingsAction::DeleteUser(username) => self.open_delete_user_confirm(&username),
            SettingsAction::About => {
                self.about_modal_open = true;
                self.about_scroll = 0;
                self.status_message = "About WSID & Third-Party Licenses".to_string();
            }
        }
    }

    fn finish_database_switch(&mut self, path: &Path) {
        match self.switch_database(path) {
            Ok(()) => {
                self.settings.mode = SettingsMode::Browse;
                self.settings.database_path_input.clear();
                self.status_message = format!("Using database {}", self.db_path.display());
            }
            Err(err) => {
                self.status_message = format!("Cannot open database '{}': {err}", path.display());
            }
        }
    }

    fn update_focus_limit_prompt(&mut self) {
        self.status_message = format!(
            "Focus queue size ({}-{}): {}_",
            wsid_core::MIN_FOCUS_LIMIT,
            wsid_core::MAX_FOCUS_LIMIT,
            self.settings.focus_limit_input
        );
    }

    fn handle_settings_focus_limit_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.settings.focus_limit_input, &mut self.settings.focus_limit_history) {
            if changed {
                self.update_focus_limit_prompt();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.settings.mode = SettingsMode::Browse;
                self.settings.focus_limit_input.clear();
                self.status_message = "Cancelled focus queue size change".to_string();
            }
            KeyCode::Backspace => {
                self.settings.focus_limit_input.pop();
                self.update_focus_limit_prompt();
            }
            KeyCode::Enter => self.submit_focus_limit(),
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                if self.settings.focus_limit_input.len() < 2 {
                    self.settings.focus_limit_input.push(ch);
                }
                self.update_focus_limit_prompt();
            }
            _ => {}
        }
    }

    fn submit_focus_limit(&mut self) {
        let raw = self.settings.focus_limit_input.trim().to_string();
        let limit = match raw.parse::<usize>() {
            Ok(limit) => limit,
            Err(_) => {
                self.status_message = format!(
                    "Focus queue size must be a whole number between {} and {}",
                    wsid_core::MIN_FOCUS_LIMIT,
                    wsid_core::MAX_FOCUS_LIMIT
                );
                return;
            }
        };
        if validate_focus_limit(limit).is_err() {
            self.status_message =
                format!("Focus queue size must be between {} and {}", wsid_core::MIN_FOCUS_LIMIT, wsid_core::MAX_FOCUS_LIMIT);
            return;
        }
        self.config.focus_limit = limit;
        if let Err(err) = self.config.save_to_path(&self.config_path) {
            self.status_message = format!("Failed to save settings: {err}");
            return;
        }
        self.settings.mode = SettingsMode::Browse;
        self.settings.focus_limit_input.clear();
        if let Err(err) = self.refresh_all() {
            self.status_message = format!("Refresh failed: {err}");
            return;
        }
        self.status_message = format!("Focus queue size set to {limit} tasks");
    }

    fn begin_database_path_prompt(&mut self) {
        self.settings.mode = SettingsMode::EditDatabasePath;
        self.settings.database_path_input.clear();
        self.settings.database_path_history.clear();
        self.settings.user_kind_input.clear();
        self.settings.user_kind_history.clear();
        self.update_database_path_prompt();
    }

    fn update_database_path_prompt(&mut self) {
        self.status_message =
            format!("Open database path or folder (Enter to open, Esc to cancel): {}_", self.settings.database_path_input);
    }

    fn handle_settings_database_path_key(&mut self, key: KeyEvent) {
        if let Some(changed) = apply_text_shortcut(&key, &mut self.settings.database_path_input, &mut self.settings.database_path_history) {
            if changed {
                self.update_database_path_prompt();
            }
            return;
        }
        if is_modified_char(&key) {
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.settings.mode = SettingsMode::Browse;
                self.settings.database_path_input.clear();
                self.status_message = "Cancelled database switch".to_string();
            }
            KeyCode::Backspace => {
                self.settings.database_path_input.pop();
                self.update_database_path_prompt();
            }
            KeyCode::Enter => self.submit_database_path(),
            KeyCode::Char(ch) => {
                self.settings.database_path_input.push(ch);
                self.update_database_path_prompt();
            }
            _ => {}
        }
    }

    fn submit_database_path(&mut self) {
        let raw = self.settings.database_path_input.trim().to_string();
        if raw.is_empty() {
            self.status_message = "Database path cannot be empty".to_string();
            return;
        }
        match self.switch_database(Path::new(&raw)) {
            Ok(()) => {
                self.settings.mode = SettingsMode::Browse;
                self.settings.database_path_input.clear();
                self.status_message = format!("Using database {}", self.db_path.display());
            }
            Err(err) => {
                self.status_message = format!("Cannot open database '{raw}': {err}");
            }
        }
    }

    /// Opens `path` and only then replaces the running session, so a bad path
    /// never leaves the TUI without a usable database.
    pub fn switch_database(&mut self, path: &Path) -> wsid_core::Result<()> {
        let next_path = if path.is_dir() { path.join(wsid_core::LOCAL_DB_FILENAME) } else { path.to_path_buf() };
        let db = Database::open(&next_path)?;
        let change_detector = DbChangeDetector::new(&db)?;

        let is_global = next_path == wsid_core::default_global_db_path();
        let mut config = self.config.clone();
        if !is_global {
            config.record_recent_workspace(next_path.clone());
            config.save_to_path(&self.config_path)?;
        }

        self.db = db;
        self.db_path = next_path;
        self.change_detector = change_detector;
        self.config = config;
        self.reset_after_database_switch();
        self.refresh_all()?;
        Ok(())
    }

    fn reset_after_database_switch(&mut self) {
        self.selected_ids.clear();
        self.bulk_editing = false;
        self.bulk_input.clear();
        self.bulk_history.clear();

        self.focus_project = wsid_core::ALL_PROJECTS.to_string();
        self.kanban_project = self.active_project.clone();
        self.eisenhower_project = self.active_project.clone();
        self.focus_user = self.user.clone();
        self.kanban_user = self.user.clone();
        self.eisenhower_user = self.user.clone();

        self.search_query.clear();
        self.search_editing = false;
        self.search_status_filter = None;
        self.search_only_mine = false;
        self.search_tag_filter = None;
        self.search_assignee_filter = None;
        self.search_project_filter = None;
        self.search_sort_col = DatabaseSortColumn::Relevance;
        self.search_sort_asc = true;
        self.search_selected = 0;
        self.search_table_state = TableState::default();
        self.search_list_state = ListState::default();
        self.search_history.clear();

        self.focus_selected = 0;
        self.focus_state = ListState::default();
        self.kanban_col = 1;
        self.kanban_row = 0;
        self.kanban_states = std::array::from_fn(|_| ListState::default());
        self.eisenhower_quadrant = 0;
        self.eisenhower_row = 0;
        self.eisenhower_states = std::array::from_fn(|_| ListState::default());

        self.project_modal_open = false;
        self.project_modal_query.clear();
        self.project_modal_selected = 0;
        self.project_modal_history.clear();
        self.user_modal_open = false;
        self.user_modal_query.clear();
        self.user_modal_selected = 0;
        self.user_modal_history.clear();
        self.about_modal_open = false;
        self.about_scroll = 0;

        self.detail_modal = None;
        self.detail_prompt_kind = DetailPromptKind::None;
        self.detail_prompt_input.clear();
        self.detail_suggestions.clear();
        self.detail_suggestion_index = 0;
        self.detail_suggestion_navigated = false;
        self.detail_comment_preview.clear();
        self.detail_note_cursor = 0;
        self.detail_prompt_history.clear();
        self.attachment_focused = false;
        self.attachment_cursor = 0;
        self.attachment_marked.clear();
        self.form_errors = None;

        self.editing_task_id = None;
        self.editing_return_tab = None;

        self.settings.selected = 0;
        self.settings.mode = SettingsMode::Browse;
        self.settings.focus_limit_input.clear();
        self.settings.focus_limit_history.clear();
        self.settings.database_path_input.clear();
        self.settings.database_path_history.clear();
        self.settings.delete_confirm = None;
    }

    fn open_delete_user_confirm(&mut self, username: &str) {
        if !self.is_primary_user() {
            self.status_message = format!("Only @{} can manage and delete users", self.config.primary_user);
            return;
        }
        match preview_user_deletion(&self.db, username, &self.config.primary_user) {
            Ok(preview) => {
                if preview
                    .username
                    .eq_ignore_ascii_case(&self.config.primary_user)
                {
                    self.status_message = format!("@{} is the main user and cannot be deleted", preview.username);
                    return;
                }
                self.status_message.clear();
                self.settings.delete_confirm = Some(UserDeleteConfirm { preview, delete_authored: false });
            }
            Err(err) => self.status_message = format!("Cannot delete @{username}: {err}"),
        }
    }

    fn handle_user_delete_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => {
                self.settings.delete_confirm = None;
                self.status_message = "Cancelled user deletion".to_string();
            }
            KeyCode::Char('a') | KeyCode::Char(' ') => {
                if let Some(confirm) = self.settings.delete_confirm.as_mut() {
                    confirm.delete_authored = !confirm.delete_authored;
                    self.status_message = format!("Also delete authored tasks: {}", if confirm.delete_authored { "ON" } else { "OFF" });
                }
            }
            KeyCode::Enter => {
                if let Some(confirm) = self.settings.delete_confirm.take() {
                    self.confirm_delete_user(confirm);
                }
            }
            _ => {}
        }
    }

    fn confirm_delete_user(&mut self, confirm: UserDeleteConfirm) {
        let username = confirm.preview.username.clone();
        let primary = self.config.primary_user.clone();
        match delete_user_from_db(&mut self.db, &username, &primary, confirm.delete_authored, None) {
            Ok(outcome) => {
                for filter in [&mut self.focus_user, &mut self.kanban_user, &mut self.eisenhower_user] {
                    if filter.eq_ignore_ascii_case(&outcome.deleted_user) {
                        *filter = primary.clone();
                    }
                }
                if self
                    .search_assignee_filter
                    .as_deref()
                    .is_some_and(|user| user.eq_ignore_ascii_case(&outcome.deleted_user))
                {
                    self.search_assignee_filter = None;
                }
                let _ = self.refresh_all();
                self.status_message = if outcome.deleted_task_count > 0 {
                    format!(
                        "Deleted @{}: {} task(s) reassigned to @{}, {} authored task(s) deleted",
                        outcome.deleted_user, outcome.reassigned_task_count, outcome.fallback_user, outcome.deleted_task_count
                    )
                } else {
                    format!(
                        "Deleted @{}: {} task(s) reassigned to @{}",
                        outcome.deleted_user, outcome.reassigned_task_count, outcome.fallback_user
                    )
                };
            }
            Err(err) => self.status_message = format!("Cannot delete @{username}: {err}"),
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
        let kinds = wsid_core::list_user_kinds(&self.db).unwrap_or_default();
        let warning = self
            .bulk_input
            .split_whitespace()
            .filter_map(|t| t.strip_prefix("assignee=").or_else(|| t.strip_prefix("a=")))
            .find_map(|v| user_kind_mismatch(v, &kinds));
        self.status_message = match warning {
            Some(w) => format!("{w} | Bulk edit {n} task(s): {}_", self.bulk_input),
            None => format!(
                "Bulk edit {n} task(s): status=todo assignee=[kind:]bob importance=4 difficulty=2 urgent=on +tag -tag delete | {}_",
                self.bulk_input
            ),
        };
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

    pub(crate) fn apply_bulk_input(&mut self, input: &str) {
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
                        "assignee" | "a" => {
                            wsid_core::parse_user_ref(v).map_err(|e| e.to_string())?;
                            patch.assignee = Some(v.to_string());
                        }
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
                self.status_message = Self::with_user_notices(std::mem::take(&mut self.status_message), &outcome.user_notices);
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
