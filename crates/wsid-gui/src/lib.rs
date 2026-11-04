use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::menu::{HELP_SUBMENU_ID, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu, WINDOW_SUBMENU_ID};
use tauri::{AppHandle, Emitter, State};
use wsid_core::{
    AboutMetadata, AddTaskNoteOutcome, AppConfig, AttachmentInput, BlockedTaskEntry, BulkEditOutcome, BulkTaskPatch, Database,
    DbChangeDetector, EisenhowerClassification, FieldIssue, FocusPreset, FocusQueueResponse, RelationDraft, RelationInputKind, Scalar,
    SearchHit, SearchQuery, StatusTransitionOutcome, Task, TaskAttachment, TaskDraft, TaskNote, TaskRelation, TaskStatus, UpdateTaskInput,
    UserDeletionOutcome, UserDeletionPreview, UserNotice, add_task_note, attach_existing_file, bulk_delete_tasks, bulk_update_tasks,
    compute_focus_queue_in_project, create_task_from_draft_with_notices, default_config_path, default_global_db_path, delete_task,
    delete_task_note, delete_user_from_db, get_about_metadata, get_attachment, get_blocked_tasks, get_direct_blocked_tasks,
    get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers, get_related_tasks, get_task, link_tasks,
    list_eisenhower_matrix, list_known_projects, list_known_users, list_task_notes, list_tasks, list_user_kinds, normalize_project,
    normalize_user, open_file_in_os, parse_deadline_datetime, preview_user_deletion, register_user, remove_attachment,
    resolve_active_db_path, save_clipboard_attachment, search_tasks, transition_task_status, unblock_or_dismiss_subtask, unlink_tasks,
    update_attachment_description, update_task_relation, update_task_with_notices, validate_focus_limit, validate_task_draft,
};

pub struct GuiAppState {
    pub db_path: PathBuf,
    pub config_path: PathBuf,
    pub config: AppConfig,
    pub active_user: String,
    /// Session-only project context (`Ctrl+P` in the GUI). Defaults to `DEFAULT_PROJECT`.
    pub active_project: String,
    pub active_preset: FocusPreset,
}

impl GuiAppState {
    pub fn initialize() -> Self {
        let config_path = default_config_path();
        let config = AppConfig::load_from_path(&config_path).unwrap_or_default();
        let db_path = resolve_active_db_path(None, false);
        let active_user = config.default_user.clone();
        let active_preset = config.focus_preset;
        let active_project = wsid_core::DEFAULT_PROJECT.to_string();
        Self { db_path, config_path, config, active_user, active_project, active_preset }
    }

    pub fn open_db(&self) -> Result<Database, String> {
        Database::open(&self.db_path).map_err(|e| e.to_string())
    }

    pub fn save_config(&self) {
        let _ = self.config.save_to_path(&self.config_path);
    }
}

pub type SharedGuiState = Arc<Mutex<GuiAppState>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSessionSnapshot {
    pub db_path: String,
    pub global_db_path: String,
    pub is_global_db: bool,
    pub primary_user: String,
    pub active_user: String,
    pub active_project: String,
    pub active_preset: String,
    pub focus_limit: usize,
    pub recent_workspaces: Vec<String>,
    pub known_users: Vec<String>,
    /// Kind per known user (`human` unless set, e.g. `claude`). Keys match `known_users`.
    pub user_kinds: std::collections::BTreeMap<String, String>,
    pub known_tags: Vec<String>,
    pub known_projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFullDetail {
    pub task: Task,
    pub blockers: Vec<Task>,
    pub blocking: Vec<Task>,
    pub parents: Vec<Task>,
    pub subtasks: Vec<Task>,
    pub parallel: Vec<Task>,
    pub related: Vec<Task>,
    pub notes: Vec<TaskNote>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuiCreateTaskPayload {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub due_date: Option<String>,
    pub sgdb_date: Option<String>,
    pub time_estimate_minutes: Option<i64>,
    pub difficulty: Option<u8>,
    pub importance: Option<u8>,
    pub urgent_override: Option<bool>,
    pub manual_priority: Option<i8>,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
    pub attachments: Vec<AttachmentInput>,
    pub initial_relations: Vec<GuiInitialRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiInitialRelation {
    pub relation: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuiUpdateTaskPayload {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub due_date: Option<Option<String>>,
    pub sgdb_date: Option<Option<String>>,
    pub time_estimate_minutes: Option<Option<i64>>,
    pub difficulty: Option<u8>,
    pub importance: Option<u8>,
    pub urgent_override: Option<bool>,
    pub manual_priority: Option<i8>,
    pub tags: Option<Vec<String>>,
    pub project: Option<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
}

fn build_session_snapshot(state: &GuiAppState) -> Result<AppSessionSnapshot, String> {
    let mut db = state.open_db()?;
    let _ = register_user(&mut db, &state.config.primary_user);
    let _ = register_user(&mut db, &state.active_user);
    let tasks = list_tasks(&db).map_err(|e| e.to_string())?;

    let known_users =
        list_known_users(&db, &[state.config.primary_user.as_str(), state.active_user.as_str(), state.config.default_user.as_str()])
            .map_err(|e| e.to_string())?;

    let mut tags = std::collections::BTreeSet::new();
    for t in &tasks {
        for tg in &t.tags {
            tags.insert(tg.clone());
        }
    }

    let global_path = default_global_db_path();
    let is_global = state.db_path == global_path;

    Ok(AppSessionSnapshot {
        db_path: state.db_path.to_string_lossy().to_string(),
        global_db_path: global_path.to_string_lossy().to_string(),
        is_global_db: is_global,
        primary_user: state.config.primary_user.clone(),
        active_user: state.active_user.clone(),
        active_project: state.active_project.clone(),
        active_preset: state.active_preset.as_str().to_string(),
        focus_limit: state.config.focus_limit,
        recent_workspaces: state
            .config
            .recent_workspaces
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        known_users,
        user_kinds: list_user_kinds(&db).map_err(|e| e.to_string())?,
        known_tags: tags.into_iter().collect(),
        known_projects: list_known_projects(&db).map_err(|e| e.to_string())?,
    })
}

fn parse_optional_date(raw: Option<&str>, field_name: &str) -> Result<Option<String>, String> {
    match raw.map(str::trim) {
        Some(s) if !s.is_empty() => {
            parse_deadline_datetime(field_name, s).map_err(|e| e.to_string())?;
            Ok(Some(s.to_string()))
        }
        _ => Ok(None),
    }
}

/// Maps the GUI payload onto the shared core draft used for validation and atomic creation.
pub fn payload_to_draft(payload: GuiCreateTaskPayload) -> Result<TaskDraft, String> {
    let status = match payload.status.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(TaskStatus::from_str(s).map_err(|e| e.to_string())?),
        _ => None,
    };
    Ok(TaskDraft {
        title: payload.title,
        description: payload.description,
        status,
        due_date: payload.due_date,
        sgdb_date: payload.sgdb_date,
        time_estimate: payload.time_estimate_minutes.map(Scalar::Int),
        difficulty: payload.difficulty.map(|n| Scalar::Int(i64::from(n))),
        importance: payload.importance.map(|n| Scalar::Int(i64::from(n))),
        manual_priority: payload.manual_priority.map(|n| Scalar::Int(i64::from(n))),
        urgent_override: payload.urgent_override.map(Scalar::Bool),
        tags: payload.tags,
        project: payload.project,
        relations: payload
            .initial_relations
            .into_iter()
            .map(|r| RelationDraft { relation: r.relation, target: r.target_id })
            .collect(),
        attachments: payload.attachments,
        reporter: payload.reporter,
        assignee: payload.assignee,
        evaluator: payload.evaluator,
    })
}

/// Fills an unset or blank project with the session's active project, matching the TUI Add Task form.
fn default_project_to_active(mut payload: GuiCreateTaskPayload, active_project: &str) -> GuiCreateTaskPayload {
    if payload
        .project
        .as_deref()
        .is_none_or(|p| p.trim().is_empty())
    {
        payload.project = Some(active_project.to_string());
    }
    payload
}

/// A saved task plus notices about typed user references (`@kind:user`) in its fields.
#[derive(Debug, Serialize)]
pub struct TaskSaveOutcome {
    pub task: Task,
    pub user_notices: Vec<UserNotice>,
}

pub fn create_task_from_payload(state: &GuiAppState, payload: GuiCreateTaskPayload) -> Result<TaskSaveOutcome, String> {
    let mut db = state.open_db()?;
    let draft = payload_to_draft(default_project_to_active(payload, &state.active_project))?;
    // Atomic: the task, its relations, and its attachments are created together or not at all.
    create_task_from_draft_with_notices(&mut db, &draft, &state.active_user)
        .map(|(task, user_notices)| TaskSaveOutcome { task, user_notices })
        .map_err(|e| e.to_string())
}

pub fn load_task_full_detail(db: &Database, id: &str) -> Result<TaskFullDetail, String> {
    let task = get_task(db, id).map_err(|e| e.to_string())?;
    let blockers = get_direct_blockers(db, &task.id).map_err(|e| e.to_string())?;
    let blocking = get_direct_blocked_tasks(db, &task.id).map_err(|e| e.to_string())?;
    let parents = get_direct_parents(db, &task.id).map_err(|e| e.to_string())?;
    let subtasks = get_direct_subtasks(db, &task.id).map_err(|e| e.to_string())?;
    let parallel = get_parallel_peers(db, &task.id).map_err(|e| e.to_string())?;
    let related = get_related_tasks(db, &task.id).map_err(|e| e.to_string())?;
    let notes = list_task_notes(db, &task.id).map_err(|e| e.to_string())?;

    Ok(TaskFullDetail { task, blockers, blocking, parents, subtasks, parallel, related, notes })
}

pub fn encode_file_to_data_url(path: &Path, media_type: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read attachment '{}': {e}", path.display()))?;
    let b64 = base64_encode(&bytes);
    Ok(format!("data:{media_type};base64,{b64}"))
}

pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let payload = input
        .trim()
        .split_once(";base64,")
        .map(|(_, b)| b)
        .unwrap_or_else(|| input.trim());

    let mut out = Vec::with_capacity(payload.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;

    for b in payload.bytes() {
        if b.is_ascii_whitespace() || b == b'=' {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 character '{}'", b as char)),
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }

    Ok(out)
}

#[tauri::command]
fn get_session_state(state: State<'_, SharedGuiState>) -> Result<AppSessionSnapshot, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    build_session_snapshot(&guard)
}

#[tauri::command]
fn set_user_kind(state: State<'_, SharedGuiState>, user: String, kind: String) -> Result<AppSessionSnapshot, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    wsid_core::set_user_kind(&mut db, &user, &kind).map_err(|e| e.to_string())?;
    build_session_snapshot(&guard)
}

#[tauri::command]
fn set_active_user(state: State<'_, SharedGuiState>, user: String) -> Result<AppSessionSnapshot, String> {
    let norm = normalize_user(&user).map_err(|e| e.to_string())?;
    let mut guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    register_user(&mut db, &norm).map_err(|e| e.to_string())?;
    guard.active_user = norm;
    build_session_snapshot(&guard)
}

#[tauri::command]
fn set_active_project(state: State<'_, SharedGuiState>, project: String) -> Result<AppSessionSnapshot, String> {
    let norm = normalize_project(&project).map_err(|e| e.to_string())?;
    let mut guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    guard.active_project = norm;
    build_session_snapshot(&guard)
}

#[tauri::command]
fn preview_delete_user(state: State<'_, SharedGuiState>, target_user: String) -> Result<UserDeletionPreview, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    if !guard
        .active_user
        .eq_ignore_ascii_case(&guard.config.primary_user)
    {
        return Err(format!("only the main user '@{}' can manage and delete users", guard.config.primary_user));
    }
    let db = guard.open_db()?;
    preview_user_deletion(&db, &target_user, &guard.config.primary_user).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_user(state: State<'_, SharedGuiState>, target_user: String, delete_authored_tasks: bool) -> Result<UserDeletionOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    if !guard
        .active_user
        .eq_ignore_ascii_case(&guard.config.primary_user)
    {
        return Err(format!("only the main user '@{}' can manage and delete users", guard.config.primary_user));
    }
    let mut db = guard.open_db()?;
    delete_user_from_db(&mut db, &target_user, &guard.config.primary_user, delete_authored_tasks, None).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_focus_preset(state: State<'_, SharedGuiState>, preset: String) -> Result<AppSessionSnapshot, String> {
    let parsed = FocusPreset::from_str(preset.trim()).map_err(|e| e.to_string())?;
    let mut guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    guard.active_preset = parsed;
    guard.config.focus_preset = parsed;
    guard.save_config();
    build_session_snapshot(&guard)
}

#[tauri::command]
fn set_focus_limit(state: State<'_, SharedGuiState>, limit: usize) -> Result<AppSessionSnapshot, String> {
    validate_focus_limit(limit).map_err(|err| err.to_string())?;
    let mut guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    guard.config.focus_limit = limit;
    guard.save_config();
    build_session_snapshot(&guard)
}

#[tauri::command]
fn switch_database(
    app_handle: AppHandle,
    state: State<'_, SharedGuiState>,
    path: Option<String>,
    use_global: bool,
) -> Result<AppSessionSnapshot, String> {
    let snapshot = {
        let mut guard = state
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        let next_path = if use_global {
            default_global_db_path()
        } else {
            let raw = path.unwrap_or_default();
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("database path cannot be empty".to_string());
            }
            let candidate = PathBuf::from(trimmed);
            if candidate.is_dir() { candidate.join(".wsid.db") } else { candidate }
        };

        let _ = Database::open(&next_path).map_err(|e| e.to_string())?;
        guard.db_path = next_path.clone();
        if next_path != default_global_db_path() {
            guard.config.record_recent_workspace(next_path);
            guard.save_config();
        }
        build_session_snapshot(&guard)?
    };

    if let Ok(menu) = build_app_menu(&app_handle, state.inner()) {
        let _ = app_handle.set_menu(menu);
    }

    Ok(snapshot)
}

#[tauri::command]
fn pick_workspace_database_dialog() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"POSIX path of (choose file name with prompt "Select or create a WSID workspace database (.wsid.db)" default name ".wsid.db")"#;
        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !chosen.is_empty() {
                return Ok(Some(chosen));
            }
        }
        Ok(None)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

#[tauri::command]
fn pick_attachment_file_dialog() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"POSIX path of (choose file with prompt "Select a file or image to attach")"#;
        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !chosen.is_empty() {
                return Ok(Some(chosen));
            }
        }
        Ok(None)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

#[tauri::command]
fn get_focus_view_data(
    state: State<'_, SharedGuiState>,
    user: Option<String>,
    preset: Option<String>,
    limit: Option<usize>,
    project: Option<String>,
) -> Result<FocusQueueResponse, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let target_user = user
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_user);
    let target_preset = match preset.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(p) => FocusPreset::from_str(p).map_err(|e| e.to_string())?,
        None => guard.active_preset,
    };
    let top_k = limit.unwrap_or(guard.config.focus_limit);
    let today = Utc::now().date_naive();

    compute_focus_queue_in_project(
        &db,
        target_user,
        target_preset,
        guard.config.custom_weights,
        Some(top_k),
        today,
        guard.config.scoring,
        project.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_blocked_tasks_data(
    state: State<'_, SharedGuiState>,
    user: Option<String>,
    tag: Option<String>,
) -> Result<Vec<BlockedTaskEntry>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let target_user = user
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_user);
    get_blocked_tasks(&db, target_user, tag.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_all_tasks_data(state: State<'_, SharedGuiState>) -> Result<Vec<Task>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    list_tasks(&db).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_eisenhower_data(state: State<'_, SharedGuiState>, user_filter: Option<String>) -> Result<Vec<EisenhowerClassification>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let today = Utc::now().date_naive();
    list_eisenhower_matrix(&db, user_filter.as_deref(), today, guard.config.scoring).map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn search_tasks_data(
    state: State<'_, SharedGuiState>,
    query: String,
    status: Option<String>,
    tag: Option<String>,
    assignee: Option<String>,
    reporter: Option<String>,
    evaluator: Option<String>,
    project: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let parsed_status = match status.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(TaskStatus::from_str(s).map_err(|e| e.to_string())?),
        _ => None,
    };
    search_tasks(&db, &SearchQuery { query, status: parsed_status, tag, assignee, reporter, evaluator, project, limit })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_task_detail_data(state: State<'_, SharedGuiState>, id: String) -> Result<TaskFullDetail, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    load_task_full_detail(&db, &id)
}

#[tauri::command]
fn validate_new_task(state: State<'_, SharedGuiState>, payload: GuiCreateTaskPayload) -> Result<Vec<FieldIssue>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let draft = payload_to_draft(default_project_to_active(payload, &guard.active_project))?;
    Ok(validate_task_draft(&db, &draft, &guard.active_user))
}

#[tauri::command]
fn create_new_task(state: State<'_, SharedGuiState>, payload: GuiCreateTaskPayload) -> Result<TaskSaveOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    create_task_from_payload(&guard, payload)
}

#[tauri::command]
fn update_existing_task(state: State<'_, SharedGuiState>, payload: GuiUpdateTaskPayload) -> Result<TaskSaveOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;

    let status = match payload.status.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(TaskStatus::from_str(s).map_err(|e| e.to_string())?),
        _ => None,
    };

    let due_date = match payload.due_date {
        Some(Some(ref s)) => Some(parse_optional_date(Some(s), "due_date")?),
        Some(None) => Some(None),
        None => None,
    };

    let sgdb_date = match payload.sgdb_date {
        Some(Some(ref s)) => Some(parse_optional_date(Some(s), "sgdb_date")?),
        Some(None) => Some(None),
        None => None,
    };

    update_task_with_notices(
        &mut db,
        &payload.id,
        UpdateTaskInput {
            title: payload.title,
            description: payload.description,
            status,
            due_date,
            sgdb_date,
            time_estimate_minutes: payload.time_estimate_minutes,
            difficulty: payload.difficulty,
            importance: payload.importance,
            urgent_override: payload.urgent_override,
            manual_priority: payload.manual_priority,
            tags: payload.tags,
            project: payload.project,
            reporter: payload.reporter,
            assignee: payload.assignee,
            evaluator: payload.evaluator,
        },
    )
    .map(|(task, user_notices)| TaskSaveOutcome { task, user_notices })
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn transition_status(state: State<'_, SharedGuiState>, id: String, status: String) -> Result<StatusTransitionOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let parsed = TaskStatus::from_str(status.trim()).map_err(|e| e.to_string())?;
    transition_task_status(&mut db, &id, parsed).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct GuiBulkPatch {
    status: Option<String>,
    assignee: Option<String>,
    importance: Option<u8>,
    importance_at_least: Option<u8>,
    importance_at_most: Option<u8>,
    difficulty: Option<u8>,
    urgent_override: Option<bool>,
    project: Option<String>,
    #[serde(default)]
    add_tags: Vec<String>,
    #[serde(default)]
    remove_tags: Vec<String>,
}

#[tauri::command]
fn bulk_edit_tasks(state: State<'_, SharedGuiState>, ids: Vec<String>, patch: GuiBulkPatch) -> Result<BulkEditOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let status = match patch.status.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(TaskStatus::from_str(s).map_err(|e| e.to_string())?),
        _ => None,
    };
    let core_patch = BulkTaskPatch {
        status,
        assignee: patch.assignee,
        importance: patch.importance,
        importance_at_least: patch.importance_at_least,
        importance_at_most: patch.importance_at_most,
        difficulty: patch.difficulty,
        urgent_override: patch.urgent_override,
        project: patch.project,
        add_tags: patch.add_tags,
        remove_tags: patch.remove_tags,
    };
    bulk_update_tasks(&mut db, &ids, &core_patch).map_err(|e| e.to_string())
}

#[tauri::command]
fn bulk_remove_tasks(state: State<'_, SharedGuiState>, ids: Vec<String>) -> Result<BulkEditOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    bulk_delete_tasks(&mut db, &ids).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_task(state: State<'_, SharedGuiState>, id: String) -> Result<bool, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    delete_task(&mut db, &id).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
fn add_comment_note(
    state: State<'_, SharedGuiState>,
    task_id: String,
    author: Option<String>,
    body: String,
    spawn_subtasks: bool,
    attachments: Vec<AttachmentInput>,
) -> Result<AddTaskNoteOutcome, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let author_user = author
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_user);

    let outcome = add_task_note(&mut db, &task_id, author_user, &body, spawn_subtasks).map_err(|e| e.to_string())?;

    for att in attachments {
        attach_existing_file(&mut db, &task_id, Some(outcome.note.id), &att.path, att.description.as_deref()).map_err(|e| e.to_string())?;
    }

    Ok(outcome)
}

#[tauri::command]
fn remove_comment_note(state: State<'_, SharedGuiState>, note_id: i64) -> Result<bool, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    delete_task_note(&mut db, note_id).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
fn add_relation(state: State<'_, SharedGuiState>, source_id: String, relation: String, target_id: String) -> Result<TaskRelation, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let kind = RelationInputKind::from_str(relation.trim()).map_err(|e| e.to_string())?;
    link_tasks(&mut db, &source_id, kind, &target_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_relation(state: State<'_, SharedGuiState>, source_id: String, relation: String, target_id: String) -> Result<bool, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let kind = RelationInputKind::from_str(relation.trim()).map_err(|e| e.to_string())?;
    unlink_tasks(&mut db, &source_id, kind, &target_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_relation(
    state: State<'_, SharedGuiState>,
    source_id: String,
    old_relation: String,
    new_relation: String,
    target_id: String,
) -> Result<TaskRelation, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    let old_kind = RelationInputKind::from_str(old_relation.trim()).map_err(|e| e.to_string())?;
    let new_kind = RelationInputKind::from_str(new_relation.trim()).map_err(|e| e.to_string())?;
    update_task_relation(&mut db, &source_id, old_kind, new_kind, &target_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn dismiss_subtask_blocker(state: State<'_, SharedGuiState>, subtask_id: String, dismiss: bool) -> Result<Task, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    unblock_or_dismiss_subtask(&mut db, &subtask_id, dismiss).map_err(|e| e.to_string())
}

#[tauri::command]
fn attach_file_from_disk(
    state: State<'_, SharedGuiState>,
    task_id: String,
    note_id: Option<i64>,
    file_path: String,
    description: Option<String>,
) -> Result<TaskAttachment, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    attach_existing_file(&mut db, &task_id, note_id, &file_path, description.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
fn attach_clipboard_bytes(
    state: State<'_, SharedGuiState>,
    task_id: String,
    note_id: Option<i64>,
    base64_data: String,
    suggested_name: Option<String>,
    description: Option<String>,
) -> Result<TaskAttachment, String> {
    let bytes = base64_decode(&base64_data)?;
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    save_clipboard_attachment(&mut db, &task_id, note_id, &bytes, suggested_name.as_deref(), description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_clipboard_bytes_for_draft(base64_data: String, suggested_name: Option<String>) -> Result<String, String> {
    let bytes = base64_decode(&base64_data)?;
    if bytes.is_empty() {
        return Err("clipboard payload cannot be empty".to_string());
    }
    let dir = wsid_core::default_attachments_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = Utc::now().format("%Y%m%d-%H%M%S-%3f");
    let ext = suggested_name
        .as_deref()
        .and_then(|n| Path::new(n).extension().and_then(|e| e.to_str()))
        .unwrap_or("png");
    let dest = dir.join(format!("clipboard-draft-{stamp}.{ext}"));
    fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
fn edit_attachment_alt_description(
    state: State<'_, SharedGuiState>,
    attachment_id: i64,
    description: String,
) -> Result<TaskAttachment, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    update_attachment_description(&mut db, attachment_id, &description).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_attachment(state: State<'_, SharedGuiState>, attachment_id: i64) -> Result<bool, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let mut db = guard.open_db()?;
    remove_attachment(&mut db, attachment_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_attachment_native(state: State<'_, SharedGuiState>, attachment_id: Option<i64>, file_path: Option<String>) -> Result<(), String> {
    let target_path = if let Some(p) = file_path.filter(|s| !s.trim().is_empty()) {
        PathBuf::from(p)
    } else if let Some(id) = attachment_id {
        let guard = state
            .lock()
            .map_err(|_| "state lock poisoned".to_string())?;
        let db = guard.open_db()?;
        let att = get_attachment(&db, id).map_err(|e| e.to_string())?;
        PathBuf::from(att.file_path)
    } else {
        return Err("either attachment_id or file_path must be provided".to_string());
    };

    open_file_in_os(&target_path).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_attachment_preview_data_url(state: State<'_, SharedGuiState>, attachment_id: i64) -> Result<String, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let att = get_attachment(&db, attachment_id).map_err(|e| e.to_string())?;
    encode_file_to_data_url(Path::new(&att.file_path), &att.media_type)
}

#[tauri::command]
fn get_about_info() -> Result<AboutMetadata, String> {
    Ok(get_about_metadata())
}

pub fn spawn_db_change_watcher(app_handle: AppHandle, state: SharedGuiState) {
    std::thread::spawn(move || {
        let mut current_path: Option<PathBuf> = None;
        let mut cached_db: Option<Database> = None;
        let mut detector: Option<DbChangeDetector> = None;

        loop {
            std::thread::sleep(Duration::from_millis(350));

            let desired_path = match state.lock() {
                Ok(guard) => guard.db_path.clone(),
                Err(_) => continue,
            };

            if current_path.as_ref() != Some(&desired_path) {
                current_path = Some(desired_path.clone());
                cached_db = Database::open(&desired_path).ok();
                detector = cached_db
                    .as_ref()
                    .and_then(|db| DbChangeDetector::new(db).ok());
            }

            if let (Some(db), Some(det)) = (cached_db.as_ref(), detector.as_mut())
                && det.has_external_changes(db).unwrap_or(false)
            {
                let _ = app_handle.emit("wsid://db-changed", ());
            }
        }
    });
}

fn build_app_menu(app_handle: &AppHandle, state: &SharedGuiState) -> tauri::Result<Menu<tauri::Wry>> {
    let recent_workspaces = state
        .lock()
        .map(|g| {
            g.config
                .recent_workspaces
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut recent_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    if recent_workspaces.is_empty() {
        recent_items.push(MenuItem::with_id(app_handle, "menu:recent_db_empty", "(No Recent Workspaces)", false, None::<&str>)?);
    } else {
        for (idx, path) in recent_workspaces.iter().enumerate() {
            let label = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|name| format!("{name} — {path}"))
                .unwrap_or_else(|| path.clone());
            recent_items.push(MenuItem::with_id(app_handle, format!("menu:recent_db:{idx}"), label, true, None::<&str>)?);
        }
    }
    let recent_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = recent_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    let recent_submenu = Submenu::with_items(app_handle, "Recent Workspaces", true, &recent_refs)?;

    #[cfg(target_os = "macos")]
    let app_submenu = Submenu::with_items(
        app_handle,
        "WSID",
        true,
        &[
            &MenuItem::with_id(app_handle, "menu:about", "About WSID & Licenses...", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app_handle)?,
            &MenuItem::with_id(app_handle, "menu:settings", "Settings...", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::services(app_handle, None)?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::hide(app_handle, None)?,
            &PredefinedMenuItem::hide_others(app_handle, None)?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::quit(app_handle, None)?,
        ],
    )?;

    let file_submenu = Submenu::with_items(
        app_handle,
        "File",
        true,
        &[
            &MenuItem::with_id(app_handle, "menu:open_workspace_db", "Open Workspace Database...", true, Some("CmdOrCtrl+O"))?,
            &MenuItem::with_id(app_handle, "menu:switch_global_db", "Switch to Global Database", true, Some("CmdOrCtrl+Shift+G"))?,
            &recent_submenu,
            &PredefinedMenuItem::separator(app_handle)?,
            &MenuItem::with_id(app_handle, "menu:switch_user", "Switch Active User...", true, None::<&str>)?,
            &MenuItem::with_id(app_handle, "menu:switch_project", "Switch Active Project...", true, None::<&str>)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::separator(app_handle)?,
            #[cfg(not(target_os = "macos"))]
            &MenuItem::with_id(app_handle, "menu:settings", "Settings...", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::close_window(app_handle, None)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app_handle, None)?,
        ],
    )?;

    let edit_submenu = Submenu::with_items(
        app_handle,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app_handle, None)?,
            &PredefinedMenuItem::redo(app_handle, None)?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::cut(app_handle, None)?,
            &PredefinedMenuItem::copy(app_handle, None)?,
            &PredefinedMenuItem::paste(app_handle, None)?,
            &PredefinedMenuItem::select_all(app_handle, None)?,
        ],
    )?;

    let view_submenu = Submenu::with_items(
        app_handle,
        "View",
        true,
        &[
            &MenuItem::with_id(app_handle, "menu:view_focus", "Focus Queue", true, Some("CmdOrCtrl+1"))?,
            &MenuItem::with_id(app_handle, "menu:view_kanban", "Kanban Board", true, Some("CmdOrCtrl+2"))?,
            &MenuItem::with_id(app_handle, "menu:view_eisenhower", "Eisenhower Matrix", true, Some("CmdOrCtrl+3"))?,
            &MenuItem::with_id(app_handle, "menu:view_database", "Database", true, Some("CmdOrCtrl+4"))?,
            &MenuItem::with_id(app_handle, "menu:view_entry", "Add Task", true, Some("CmdOrCtrl+5"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &MenuItem::with_id(app_handle, "menu:view_settings", "Settings", true, None::<&str>)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app_handle)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::fullscreen(app_handle, None)?,
        ],
    )?;

    let window_menu = Submenu::with_id_and_items(
        app_handle,
        WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app_handle, None)?,
            &PredefinedMenuItem::maximize(app_handle, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::close_window(app_handle, None)?,
        ],
    )?;

    let help_menu = Submenu::with_id_and_items(
        app_handle,
        HELP_SUBMENU_ID,
        "Help",
        true,
        &[&MenuItem::with_id(app_handle, "menu:help_about", "About & Third-Party Licenses...", true, None::<&str>)?],
    )?;

    Menu::with_items(
        app_handle,
        &[
            #[cfg(target_os = "macos")]
            &app_submenu,
            &file_submenu,
            &edit_submenu,
            &view_submenu,
            &window_menu,
            &help_menu,
        ],
    )
}

fn tauri_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    let shared_state: SharedGuiState = Arc::new(Mutex::new(GuiAppState::initialize()));
    let menu_state = shared_state.clone();

    tauri::Builder::default()
        .manage(shared_state.clone())
        .setup(move |app| {
            if let Ok(menu) = build_app_menu(app.handle(), &shared_state) {
                let _ = app.set_menu(menu);
            }
            spawn_db_change_watcher(app.handle().clone(), shared_state.clone());
            Ok(())
        })
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            match id {
                "menu:open_workspace_db" => {
                    let _ = app.emit("wsid://menu-action", "open_workspace_db");
                }
                "menu:switch_global_db" => {
                    let _ = app.emit("wsid://menu-action", "switch_global_db");
                }
                "menu:switch_user" => {
                    let _ = app.emit("wsid://menu-action", "switch_user");
                }
                "menu:switch_project" => {
                    let _ = app.emit("wsid://menu-action", "switch_project");
                }
                "menu:settings" | "menu:view_settings" => {
                    let _ = app.emit("wsid://menu-action", "open_settings");
                }
                "menu:about" | "menu:help_about" => {
                    let _ = app.emit("wsid://menu-action", "open_about");
                }
                "menu:view_focus" => {
                    let _ = app.emit("wsid://menu-action", "view:focus");
                }
                "menu:view_kanban" => {
                    let _ = app.emit("wsid://menu-action", "view:kanban");
                }
                "menu:view_eisenhower" => {
                    let _ = app.emit("wsid://menu-action", "view:eisenhower");
                }
                "menu:view_database" => {
                    let _ = app.emit("wsid://menu-action", "view:database");
                }
                "menu:view_entry" => {
                    let _ = app.emit("wsid://menu-action", "view:entry");
                }
                other if other.starts_with("menu:recent_db:") => {
                    if let Ok(idx) = other["menu:recent_db:".len()..].parse::<usize>()
                        && let Ok(guard) = menu_state.lock()
                        && let Some(path) = guard.config.recent_workspaces.get(idx)
                    {
                        let _ = app.emit("wsid://menu-action", format!("switch_db:{}", path.to_string_lossy()));
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_session_state,
            set_active_user,
            set_user_kind,
            set_active_project,
            preview_delete_user,
            delete_user,
            set_focus_preset,
            set_focus_limit,
            switch_database,
            pick_workspace_database_dialog,
            pick_attachment_file_dialog,
            get_focus_view_data,
            get_blocked_tasks_data,
            get_all_tasks_data,
            get_eisenhower_data,
            search_tasks_data,
            get_task_detail_data,
            create_new_task,
            validate_new_task,
            update_existing_task,
            transition_status,
            remove_task,
            bulk_edit_tasks,
            bulk_remove_tasks,
            add_comment_note,
            remove_comment_note,
            add_relation,
            remove_relation,
            update_relation,
            dismiss_subtask_blocker,
            attach_file_from_disk,
            attach_clipboard_bytes,
            save_clipboard_bytes_for_draft,
            edit_attachment_alt_description,
            delete_attachment,
            open_attachment_native,
            read_attachment_preview_data_url,
            get_about_info,
        ])
        .run(tauri_context())
        .expect("error while running WSID desktop application");
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn new_task_project_defaults_to_active_project_only_when_unset() {
        let unset = default_project_to_active(GuiCreateTaskPayload::default(), "work");
        assert_eq!(unset.project.as_deref(), Some("work"));

        let blank = default_project_to_active(GuiCreateTaskPayload { project: Some("  ".into()), ..Default::default() }, "work");
        assert_eq!(blank.project.as_deref(), Some("work"));

        let explicit = default_project_to_active(GuiCreateTaskPayload { project: Some("home".into()), ..Default::default() }, "work");
        assert_eq!(explicit.project.as_deref(), Some("home"));
    }

    #[test]
    fn gui_backend_creates_tasks_manages_attachments_and_loads_full_detail() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gui_test.db");
        let config_path = dir.path().join("config.toml");

        let state = GuiAppState {
            db_path,
            config_path,
            config: AppConfig::default(),
            active_user: "alice".to_string(),
            active_project: wsid_core::DEFAULT_PROJECT.to_string(),
            active_preset: FocusPreset::Balanced,
        };

        let sample_img = dir.path().join("mockup.png");
        fs::write(&sample_img, b"\x89PNG\r\n\x1a\nmockup-bytes").unwrap();

        let created = create_task_from_payload(
            &state,
            GuiCreateTaskPayload {
                title: "Build accessible GUI drawer".to_string(),
                description: Some("Support screen readers and image lightbox".to_string()),
                importance: Some(5),
                difficulty: Some(3),
                due_date: Some("2099-11-12".to_string()),
                sgdb_date: Some("2099-11-10".to_string()),
                tags: vec!["gui".to_string(), "a11y".to_string()],
                attachments: vec![AttachmentInput {
                    path: sample_img.to_string_lossy().to_string(),
                    description: Some("Drawer wireframe screenshot".to_string()),
                }],
                ..Default::default()
            },
        )
        .unwrap()
        .task;

        assert_eq!(created.attachments.len(), 1);
        assert_eq!(created.attachments[0].alt_text(), "Drawer wireframe screenshot");

        let db = state.open_db().unwrap();
        let detail = load_task_full_detail(&db, &created.id).unwrap();
        assert_eq!(detail.task.id, created.id);
        assert_eq!(detail.task.tags, vec!["a11y".to_string(), "gui".to_string()]);

        // Verify base64 roundtrip and data URL encoding
        let data_url = encode_file_to_data_url(&sample_img, "image/png").unwrap();
        assert!(data_url.starts_with("data:image/png;base64,"));
        let decoded = base64_decode(&data_url).unwrap();
        assert_eq!(decoded, b"\x89PNG\r\n\x1a\nmockup-bytes");

        // Verify About and third-party license metadata endpoint
        let about = get_about_info().unwrap();
        assert_eq!(about.license_spdx, "Apache-2.0");
        assert_eq!(about.repository_url, "https://github.com/WhatShouldIDoToday/WSID");
        assert!(!about.rust_licenses.licenses.is_empty());
    }

    #[test]
    fn tauri_context_embeds_valid_window_icon_and_frontend_assets() {
        let context = tauri_context();

        let icon = context
            .default_window_icon()
            .expect("default_window_icon should be embedded");
        assert!(icon.width() > 0 && icon.height() > 0);
        assert_eq!(icon.rgba().len(), (icon.width() * icon.height() * 4) as usize, "icon RGBA byte length must match width * height * 4");

        let index_html = context
            .assets()
            .get(&"/index.html".into())
            .expect("/index.html must be embedded in Tauri context assets");
        assert!(!index_html.is_empty());
    }
}
