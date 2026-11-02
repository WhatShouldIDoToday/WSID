use std::path::{Path, PathBuf};

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use wsid_core::{
    AppConfig, BulkTaskPatch, CreateTaskInput, Database, DbChangeDetector, EisenhowerClassification, EisenhowerQuadrant, FocusPreset,
    FocusQueueResponse, RelationInputKind, SearchHit, SearchQuery, Task, TaskNote, TaskStatus, UpdateTaskInput, add_task_note,
    attach_existing_file, bulk_delete_tasks, bulk_update_tasks, compute_focus_queue, create_task, delete_task, delete_task_note,
    get_attachment, get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers,
    get_related_tasks, get_task, link_tasks, list_eisenhower_matrix, list_known_users, list_task_notes, list_tasks, normalize_user,
    open_file_in_os, register_user, save_clipboard_attachment, search_tasks, transition_task_status, unblock_or_dismiss_subtask,
    update_attachment_description, update_task,
};

use crate::args::{parse_date_arg, parse_duration_estimate, parse_relation_arg};
use crate::cli_runner::read_system_clipboard_attachment;

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
    DueDate,
    SgdbDate,
    Importance,
    Difficulty,
    Assignee,
}

impl DatabaseSortColumn {
    pub const fn next(self) -> Self {
        match self {
            Self::Relevance => Self::Id,
            Self::Id => Self::Title,
            Self::Title => Self::Status,
            Self::Status => Self::DueDate,
            Self::DueDate => Self::SgdbDate,
            Self::SgdbDate => Self::Importance,
            Self::Importance => Self::Difficulty,
            Self::Difficulty => Self::Assignee,
            Self::Assignee => Self::Relevance,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Score/Created",
            Self::Id => "ID",
            Self::Title => "Title",
            Self::Status => "Status",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailPromptKind {
    None,
    AddComment,
    AttachFile,
    AttachClipboard,
    EditAttachmentDesc,
    JumpToTask,
    EditField,
    ConfirmDeleteTask,
    ConfirmDeleteNote,
}

#[derive(Debug, Clone)]
pub struct DataEntryForm {
    pub fields: [String; 16],
    pub active_index: usize,
    pub state: ListState,
}

impl DataEntryForm {
    pub const SUBMIT_INDEX: usize = 16;

    pub const LABELS: [&'static str; 16] = [
        "1. Title *",
        "2. Description",
        "3. Due Date (YYYY-MM-DD or ISO)",
        "4. SGDB Date (YYYY-MM-DD or ISO, <= Due)",
        "5. Time Estimate (15m, 2h, 3d, 1w)",
        "6. Difficulty (1-5)",
        "7. Importance (1-5)",
        "8. Manual Priority (1-5)",
        "9. Urgent Override (true/false)",
        "10. Tags (single words)",
        "11. Relations (e.g. blocks:AB1234)",
        "12. Attach File Path",
        "13. Attach Alt Description",
        "14. Reporter",
        "15. Assignee",
        "16. Evaluator",
    ];

    pub fn new(default_user: &str) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            fields: [
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                "3".to_string(),
                "3".to_string(),
                "3".to_string(),
                "false".to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                default_user.to_string(),
                default_user.to_string(),
                default_user.to_string(),
            ],
            active_index: 0,
            state,
        }
    }

    pub fn set_active_index(&mut self, index: usize) {
        self.active_index = index;
        self.state.select(Some(index));
    }
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

    // Eisenhower View state
    pub eisenhower_items: Vec<EisenhowerClassification>,
    pub eisenhower_quadrant: usize,
    pub eisenhower_row: usize,
    pub eisenhower_states: [ListState; 4],

    // Database Search View state
    pub search_query: String,
    pub search_editing: bool,
    pub search_status_filter: Option<TaskStatus>,
    pub search_only_mine: bool,
    pub search_tag_filter: Option<String>,
    pub search_assignee_filter: Option<String>,
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
    pub detail_note_cursor: usize,

    // Active-user switching prompt
    pub user_switching: bool,
    pub user_input: String,

    // About & Third-Party Licenses Modal state
    pub about_modal_open: bool,
    pub about_scroll: u16,
}

impl TuiApp {
    pub fn new(db_path: &Path, config: AppConfig, user: String, accessible_mode: bool) -> wsid_core::Result<Self> {
        let db = Database::open(db_path)?;
        let change_detector = DbChangeDetector::new(&db)?;
        let preset = config.focus_preset;
        let today = Utc::now().date_naive();
        let focus_queue = compute_focus_queue(&db, &user, preset, config.custom_weights, Some(config.focus_limit), today, config.scoring)?;
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
            eisenhower_items,
            eisenhower_quadrant: 0,
            eisenhower_row: 0,
            eisenhower_states: std::array::from_fn(|_| ListState::default()),
            search_query: String::new(),
            search_editing: false,
            search_status_filter: None,
            search_only_mine: false,
            search_tag_filter: None,
            search_assignee_filter: None,
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
            detail_note_cursor: 0,
            user_switching: false,
            user_input: String::new(),
            about_modal_open: false,
            about_scroll: 0,
        };
        app.refresh_all()?;
        Ok(app)
    }

    /// Polls `PRAGMA data_version` on the SQLite database and reloads state if an external
    /// process (e.g. MCP server or CLI) committed changes.
    pub fn on_tick(&mut self) -> wsid_core::Result<bool> {
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
        self.focus_queue = compute_focus_queue(
            &self.db,
            &self.user,
            self.preset,
            custom_weights,
            Some(self.config.focus_limit),
            today,
            self.config.scoring,
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

        self.eisenhower_items = list_eisenhower_matrix(&self.db, Some(&self.user), today, self.config.scoring)?;
        self.clamp_eisenhower_selection();

        self.refresh_search()?;

        if let Some(ref current_detail) = self.detail_modal.clone() {
            if let Ok(updated_snapshot) = self.load_detail_snapshot(&current_detail.task.id, current_detail.scroll) {
                self.detail_modal = Some(updated_snapshot);
            } else {
                self.detail_modal = None;
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
                self.status_message = format!(
                    "Inspecting [{}] — c:comment, E:edit, D:delete, N/X:select/delete comment, f:attach, o:open, u:unblock, Esc:close",
                    snapshot.task.id
                );
                self.detail_modal = Some(snapshot);
                self.detail_note_cursor = 0;
                self.detail_prompt_kind = DetailPromptKind::None;
                self.detail_prompt_input.clear();
            }
            Err(err) => {
                self.status_message = format!("Error loading task {task_id}: {err}");
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global Ctrl+C always exits cleanly
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // 0. If the About / Licenses modal is open, route keys to it
        if self.about_modal_open {
            self.handle_about_modal_key(key);
            return;
        }

        // 1. If the Task Detail Modal is open, route keys to it
        if self.detail_modal.is_some() {
            self.handle_detail_modal_key(key);
            return;
        }

        // 2. If Database search bar is actively being edited, route typing to it
        if self.active_tab == ActiveTab::Database && self.search_editing {
            self.handle_search_input_key(key);
            return;
        }

        // 2a. Active-user switch prompt captures typing
        if self.user_switching {
            self.handle_user_input_key(key);
            return;
        }

        // 2b. Bulk-edit prompt captures typing
        if self.bulk_editing {
            self.handle_bulk_input_key(key);
            return;
        }

        // 3. If on DataEntry tab, route typing to form unless switching tabs via Alt/F-keys or Esc
        if self.active_tab == ActiveTab::DataEntry {
            if key.code == KeyCode::Esc {
                self.active_tab = ActiveTab::Focus;
                self.status_message = "Switched to Focus View".to_string();
                return;
            }
            self.handle_data_entry_key(key);
            return;
        }

        // 4. Global navigation keys across Focus, Kanban, Eisenhower, Database
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
                self.user_switching = true;
                self.user_input.clear();
                self.update_user_prompt_message();
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
            KeyCode::Char('?') => {
                self.about_modal_open = true;
                self.about_scroll = 0;
                self.status_message = "About WSID & Third-Party Licenses — [j/k/PgUp/PgDn] Scroll, [Esc/?/q] Close".to_string();
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
                let col_tasks = self.kanban_tasks_in_col(self.kanban_col);
                if let Some(task) = col_tasks.get(self.kanban_row)
                    && self.kanban_col > 0
                {
                    let id = task.id.clone();
                    let target_col = self.kanban_col - 1;
                    let target_status = TaskStatus::ALL[target_col];
                    self.apply_status_transition(&id, target_status);
                    self.kanban_col = target_col;
                    self.clamp_kanban_selection();
                }
            }
            KeyCode::Char('>') | KeyCode::Char('L') => {
                let col_tasks = self.kanban_tasks_in_col(self.kanban_col);
                if let Some(task) = col_tasks.get(self.kanban_row)
                    && self.kanban_col + 1 < TaskStatus::ALL.len()
                {
                    let id = task.id.clone();
                    let target_col = self.kanban_col + 1;
                    let target_status = TaskStatus::ALL[target_col];
                    self.apply_status_transition(&id, target_status);
                    self.kanban_col = target_col;
                    self.clamp_kanban_selection();
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
            .collect()
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

    fn handle_eisenhower_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
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
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    let next_imp = (item.task.importance + 1).min(5);
                    if let Err(err) = update_task(&mut self.db, &id, UpdateTaskInput { importance: Some(next_imp), ..Default::default() }) {
                        self.status_message = format!("Failed to update importance: {err}");
                    } else {
                        let _ = self.refresh_all();
                        self.status_message = format!("Updated [{id}] importance to {next_imp}/5");
                    }
                }
            }
            KeyCode::Char('-') => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    let next_imp = item.task.importance.saturating_sub(1).max(1);
                    if let Err(err) = update_task(&mut self.db, &id, UpdateTaskInput { importance: Some(next_imp), ..Default::default() }) {
                        self.status_message = format!("Failed to update importance: {err}");
                    } else {
                        let _ = self.refresh_all();
                        self.status_message = format!("Updated [{id}] importance to {next_imp}/5");
                    }
                }
            }
            KeyCode::Char('u') => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    let next_urgent = !item.task.urgent_override;
                    if let Err(err) =
                        update_task(&mut self.db, &id, UpdateTaskInput { urgent_override: Some(next_urgent), ..Default::default() })
                    {
                        self.status_message = format!("Failed to toggle urgency: {err}");
                    } else {
                        let _ = self.refresh_all();
                        self.status_message = format!("Updated [{id}] urgent_override to {next_urgent}");
                    }
                }
            }
            KeyCode::Char('s') => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    self.apply_status_transition(&id, TaskStatus::InProgress);
                }
            }
            KeyCode::Char('d') => {
                let items = self.eisenhower_tasks_in_quadrant(self.eisenhower_quadrant);
                if let Some(item) = items.get(self.eisenhower_row) {
                    let id = item.task.id.clone();
                    self.apply_status_transition(&id, TaskStatus::Completed);
                }
            }
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

    fn handle_database_key(&mut self, key: KeyEvent) {
        let len = self.search_hits.len();
        match key.code {
            KeyCode::Char('/') => {
                self.search_editing = true;
                self.status_message = "Editing search query — type to filter, Enter or Esc to finish".to_string();
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

        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                let next = (self.entry_form.active_index + 1) % (DataEntryForm::SUBMIT_INDEX + 1);
                self.entry_form.set_active_index(next);
            }
            KeyCode::BackTab | KeyCode::Up => {
                let prev = self
                    .entry_form
                    .active_index
                    .checked_sub(1)
                    .unwrap_or(DataEntryForm::SUBMIT_INDEX);
                self.entry_form.set_active_index(prev);
            }
            KeyCode::Enter => {
                if self.entry_form.active_index == DataEntryForm::SUBMIT_INDEX {
                    self.submit_data_entry_form();
                } else {
                    let next = (self.entry_form.active_index + 1) % (DataEntryForm::SUBMIT_INDEX + 1);
                    self.entry_form.set_active_index(next);
                }
            }
            KeyCode::Backspace => {
                if self.entry_form.active_index < DataEntryForm::SUBMIT_INDEX {
                    self.entry_form.fields[self.entry_form.active_index].pop();
                }
            }
            KeyCode::Char(ch) => {
                if self.entry_form.active_index < DataEntryForm::SUBMIT_INDEX {
                    self.entry_form.fields[self.entry_form.active_index].push(ch);
                } else if ch == ' ' {
                    self.submit_data_entry_form();
                }
            }
            _ => {}
        }
    }

    pub fn submit_data_entry_form(&mut self) {
        let f = &self.entry_form.fields;
        let title = f[0].trim().to_string();
        if title.is_empty() {
            self.status_message = "Validation error: Title is required".to_string();
            return;
        }

        let description = if f[1].trim().is_empty() { None } else { Some(f[1].trim().to_string()) };
        let due_date = match parse_date_arg("due_date", Some(&f[2])) {
            Ok(d) => d,
            Err(e) => {
                self.status_message = format!("Validation error: {e}");
                return;
            }
        };
        let sgdb_date = match parse_date_arg("sgdb_date", Some(&f[3])) {
            Ok(d) => d,
            Err(e) => {
                self.status_message = format!("Validation error: {e}");
                return;
            }
        };
        let time_estimate_minutes = if f[4].trim().is_empty() {
            None
        } else {
            match parse_duration_estimate(&f[4]) {
                Ok(m) => Some(m),
                Err(e) => {
                    self.status_message = format!("Validation error: {e}");
                    return;
                }
            }
        };
        let difficulty = match f[5].trim().parse::<u8>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.status_message = "Validation error: Difficulty must be an integer 1..=5".to_string();
                return;
            }
        };
        let importance = match f[6].trim().parse::<u8>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.status_message = "Validation error: Importance must be an integer 1..=5".to_string();
                return;
            }
        };
        let manual_priority = match f[7].trim().parse::<i8>() {
            Ok(v) if (1..=5).contains(&v) => Some(v),
            _ => {
                self.status_message = "Validation error: Manual Priority must be an integer 1..=5".to_string();
                return;
            }
        };
        let urgent_override = Some(matches!(f[8].trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "y"));

        let tags: Vec<String> = f[9]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .collect();

        // Parse relation entries like `blocks:AB1234, sub_task_of:CD5678` or plain `AB1234` (defaults to `related`)
        let mut parsed_relations: Vec<(RelationInputKind, String)> = Vec::new();
        for tok in f[10]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.trim().is_empty())
        {
            if let Some((rel_str, target_str)) = tok.split_once(':') {
                match parse_relation_arg(rel_str) {
                    Ok(kind) => parsed_relations.push((kind, target_str.to_string())),
                    Err(e) => {
                        self.status_message = format!("Validation error in relation '{tok}': {e}");
                        return;
                    }
                }
            } else {
                parsed_relations.push((RelationInputKind::Related, tok.to_string()));
            }
        }

        let attach_path_str = f[11].trim().to_string();
        let attach_desc_opt = if f[12].trim().is_empty() { None } else { Some(f[12].trim().to_string()) };

        let reporter = if f[13].trim().is_empty() { None } else { Some(f[13].trim().to_string()) };
        let assignee = if f[14].trim().is_empty() { None } else { Some(f[14].trim().to_string()) };
        let evaluator = if f[15].trim().is_empty() { None } else { Some(f[15].trim().to_string()) };

        match create_task(
            &mut self.db,
            CreateTaskInput {
                title,
                description,
                due_date,
                sgdb_date,
                time_estimate_minutes,
                difficulty,
                importance,
                manual_priority,
                urgent_override,
                tags,
                reporter,
                assignee,
                evaluator,
                ..Default::default()
            },
            &self.user,
        ) {
            Ok(created) => {
                for (kind, target_id) in parsed_relations {
                    if let Err(err) = link_tasks(&mut self.db, &created.id, kind, &target_id) {
                        self.status_message = format!("Created [{}], but linking '{target_id}' failed: {err}", created.id);
                        let _ = self.refresh_all();
                        return;
                    }
                }

                if !attach_path_str.is_empty()
                    && let Err(err) =
                        attach_existing_file(&mut self.db, &created.id, None, Path::new(&attach_path_str), attach_desc_opt.as_deref())
                {
                    self.status_message = format!("Created [{}], but attaching file failed: {err}", created.id);
                    let _ = self.refresh_all();
                    return;
                }

                self.entry_form = DataEntryForm::new(&self.user);
                let _ = self.refresh_all();
                self.status_message = format!("Created task [{}] {}", created.id, created.title);
            }
            Err(err) => {
                self.status_message = format!("Validation error: {err}");
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
            _ => {}
        }
    }

    fn handle_detail_modal_key(&mut self, key: KeyEvent) {
        if self.detail_prompt_kind != DetailPromptKind::None {
            match key.code {
                KeyCode::Esc => {
                    self.detail_prompt_kind = DetailPromptKind::None;
                    self.detail_prompt_input.clear();
                    self.status_message = "Cancelled prompt".to_string();
                }
                KeyCode::Backspace => {
                    self.detail_prompt_input.pop();
                }
                KeyCode::Enter => {
                    self.submit_detail_prompt();
                }
                KeyCode::Char(ch) => {
                    self.detail_prompt_input.push(ch);
                }
                _ => {}
            }
            return;
        }

        let Some(snapshot) = self.detail_modal.clone() else {
            return;
        };

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.detail_modal = None;
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
            KeyCode::Char('c') => {
                self.detail_prompt_kind = DetailPromptKind::AddComment;
                self.detail_prompt_input.clear();
                self.status_message = "Add comment (supports @user, @takenote:user, #AB1234) — Enter to post, Esc to cancel".to_string();
            }
            KeyCode::Char('f') => {
                self.detail_prompt_kind = DetailPromptKind::AttachFile;
                self.detail_prompt_input.clear();
                self.status_message = "Attach file: enter `/path/to/file | optional alt text` — Enter to attach, Esc to cancel".to_string();
            }
            KeyCode::Char('v') => {
                self.detail_prompt_kind = DetailPromptKind::AttachClipboard;
                self.detail_prompt_input.clear();
                self.status_message =
                    "Paste from clipboard into app-data: enter optional alt text (or leave empty for file name) — Enter to save"
                        .to_string();
            }
            KeyCode::Char('e') => {
                self.detail_prompt_kind = DetailPromptKind::EditAttachmentDesc;
                if let Some(first_att) = snapshot.task.attachments.first() {
                    self.detail_prompt_input = format!("{}: {}", first_att.id, first_att.alt_text());
                } else {
                    self.detail_prompt_input.clear();
                }
                self.status_message = "Edit attachment alt text: `<attachment_id>: <new description>` — Enter to save".to_string();
            }
            KeyCode::Char('o') => {
                let first_att = snapshot.task.attachments.first().or_else(|| {
                    snapshot
                        .notes
                        .iter()
                        .flat_map(|n| n.attachments.iter())
                        .next()
                });
                if let Some(att) = first_att {
                    match open_file_in_os(Path::new(&att.file_path)) {
                        Ok(()) => {
                            self.status_message = format!("Opened attachment #{} ({}) at {}", att.id, att.alt_text(), att.file_path);
                        }
                        Err(err) => {
                            self.status_message = format!("Failed to open {}: {err}", att.file_path);
                        }
                    }
                } else {
                    self.status_message = "No attachments on this task or its notes".to_string();
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
                self.detail_prompt_kind = DetailPromptKind::JumpToTask;
                let default_jump = snapshot
                    .blockers
                    .first()
                    .or_else(|| snapshot.parents.first())
                    .or_else(|| snapshot.subtasks.first())
                    .or_else(|| snapshot.related.first())
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
                self.detail_prompt_input = default_jump;
                self.status_message = "Jump to task ID: enter short ID (e.g. AB1234) and press Enter".to_string();
            }
            KeyCode::Char('s') => {
                self.apply_status_transition(&snapshot.task.id, TaskStatus::InProgress);
            }
            KeyCode::Char('E') => {
                self.detail_prompt_kind = DetailPromptKind::EditField;
                self.detail_prompt_input = format!("title: {}", snapshot.task.title);
                self.status_message = "Edit field: `field: value` — title, description, status, due, sgdb, estimate, difficulty, importance, priority, urgent, tags, assignee, reporter, evaluator (empty due/sgdb/estimate clears)".to_string();
            }
            KeyCode::Char('D') => {
                self.detail_prompt_kind = DetailPromptKind::ConfirmDeleteTask;
                self.detail_prompt_input.clear();
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
                        self.detail_prompt_kind = DetailPromptKind::ConfirmDeleteNote;
                        self.detail_prompt_input.clear();
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

    fn submit_detail_prompt(&mut self) {
        let Some(snapshot) = self.detail_modal.clone() else {
            return;
        };
        let input = self.detail_prompt_input.trim().to_string();
        let kind = self.detail_prompt_kind.clone();
        self.detail_prompt_kind = DetailPromptKind::None;
        self.detail_prompt_input.clear();

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
                let Some((id_str, new_desc)) = input.split_once(':') else {
                    self.status_message = "Expected format `<attachment_id>: <new description>`".to_string();
                    return;
                };
                let Ok(att_id) = id_str.trim().parse::<i64>() else {
                    self.status_message = format!("Invalid attachment ID '{}'", id_str.trim());
                    return;
                };
                match update_attachment_description(&mut self.db, att_id, new_desc.trim()) {
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
            DetailPromptKind::EditField => self.apply_field_edit(&snapshot.task, &input),
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

    /// Applies a single `field: value` edit to the task shown in the detail view.
    fn apply_field_edit(&mut self, task: &Task, input: &str) {
        let Some((field, value)) = input.split_once(':') else {
            self.status_message = "Expected `field: value`".to_string();
            return;
        };
        let (field, value) = (field.trim().to_ascii_lowercase(), value.trim());
        let clear = value.is_empty() || value.eq_ignore_ascii_case("none");

        let result: wsid_core::Result<String> = (|| {
            if field == "status" {
                let status: TaskStatus = value.parse()?;
                let outcome = transition_task_status(&mut self.db, &task.id, status)?;
                return Ok(format!("[{}] status -> {}", task.id, outcome.task.status));
            }
            let mut update = UpdateTaskInput::default();
            match field.as_str() {
                "title" => update.title = Some(value.to_string()),
                "description" | "desc" => update.description = Some(value.to_string()),
                "due" => update.due_date = Some(if clear { None } else { parse_date_arg("due", Some(value))? }),
                "sgdb" => update.sgdb_date = Some(if clear { None } else { parse_date_arg("sgdb", Some(value))? }),
                "estimate" => update.time_estimate_minutes = Some(if clear { None } else { Some(parse_duration_estimate(value)?) }),
                "difficulty" => update.difficulty = Some(parse_u8_field("difficulty", value)?),
                "importance" => update.importance = Some(parse_u8_field("importance", value)?),
                "priority" => {
                    update.manual_priority = Some(
                        value
                            .parse::<i8>()
                            .map_err(|_| wsid_core::WsidError::Validation(format!("invalid priority '{value}'")))?,
                    )
                }
                "urgent" => update.urgent_override = Some(matches!(value.to_ascii_lowercase().as_str(), "true" | "on" | "yes" | "1")),
                "tags" => {
                    update.tags = Some(
                        value
                            .split([',', ' '])
                            .filter(|t| !t.is_empty())
                            .map(str::to_string)
                            .collect(),
                    )
                }
                "assignee" => update.assignee = Some(value.to_string()),
                "reporter" => update.reporter = Some(value.to_string()),
                "evaluator" => update.evaluator = Some(value.to_string()),
                other => return Err(wsid_core::WsidError::Validation(format!("unknown field '{other}'"))),
            }
            let updated = update_task(&mut self.db, &task.id, update)?;
            Ok(format!("Updated [{}] {}", updated.id, field))
        })();

        match result {
            Ok(msg) => {
                let _ = self.refresh_all();
                self.status_message = msg;
            }
            Err(err) => self.status_message = format!("Edit failed: {err}"),
        }
    }

    fn update_user_prompt_message(&mut self) {
        let known = list_known_users(&self.db, &[self.user.as_str(), self.config.primary_user.as_str()])
            .unwrap_or_default()
            .join(", @");
        self.status_message = format!("Switch user (known: @{known}) — active @{}: @{}_", self.user, self.user_input);
    }

    fn handle_user_input_key(&mut self, key: KeyEvent) {
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
                        self.user = user.clone();
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

fn parse_u8_field(name: &str, value: &str) -> wsid_core::Result<u8> {
    value
        .parse::<u8>()
        .map_err(|_| wsid_core::WsidError::Validation(format!("invalid {name} '{value}'")))
}
