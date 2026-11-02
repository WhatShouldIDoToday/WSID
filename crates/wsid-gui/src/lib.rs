use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use wsid_core::{
    AboutMetadata, AddTaskNoteOutcome, AppConfig, AttachmentInput, BlockedTaskEntry, CreateTaskInput, Database, DbChangeDetector,
    EisenhowerClassification, FocusPreset, FocusQueueResponse, RelationInputKind, SearchHit, SearchQuery, StatusTransitionOutcome, Task,
    TaskAttachment, TaskNote, TaskRelation, TaskStatus, UpdateTaskInput, add_task_note, attach_existing_file, compute_focus_queue,
    create_task, default_config_path, default_global_db_path, delete_task, delete_task_note, get_about_metadata, get_attachment,
    get_blocked_tasks, get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers,
    get_related_tasks, get_task, link_tasks, list_eisenhower_matrix, list_task_notes, list_tasks, normalize_persona, open_file_in_os,
    remove_attachment, resolve_active_db_path, save_clipboard_attachment, search_tasks, transition_task_status, unblock_or_dismiss_subtask,
    unlink_tasks, update_attachment_description, update_task,
};

pub struct GuiAppState {
    pub db_path: PathBuf,
    pub config_path: PathBuf,
    pub config: AppConfig,
    pub active_persona: String,
    pub active_preset: FocusPreset,
}

impl GuiAppState {
    pub fn initialize() -> Self {
        let config_path = default_config_path();
        let config = AppConfig::load_from_path(&config_path).unwrap_or_default();
        let db_path = resolve_active_db_path(None, false);
        let active_persona = config.default_persona.clone();
        let active_preset = config.focus_preset;
        Self { db_path, config_path, config, active_persona, active_preset }
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
    pub active_persona: String,
    pub active_preset: String,
    pub focus_limit: usize,
    pub recent_workspaces: Vec<String>,
    pub known_personas: Vec<String>,
    pub known_tags: Vec<String>,
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
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
}

fn build_session_snapshot(state: &GuiAppState) -> Result<AppSessionSnapshot, String> {
    let db = state.open_db()?;
    let tasks = list_tasks(&db).map_err(|e| e.to_string())?;

    let mut personas = std::collections::BTreeSet::new();
    personas.insert(state.active_persona.clone());
    personas.insert(state.config.default_persona.clone());
    let mut tags = std::collections::BTreeSet::new();

    for t in &tasks {
        personas.insert(t.assignee.clone());
        personas.insert(t.reporter.clone());
        personas.insert(t.evaluator.clone());
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
        active_persona: state.active_persona.clone(),
        active_preset: state.active_preset.as_str().to_string(),
        focus_limit: state.config.focus_limit,
        recent_workspaces: state
            .config
            .recent_workspaces
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
        known_personas: personas.into_iter().collect(),
        known_tags: tags.into_iter().collect(),
    })
}

fn parse_optional_date(raw: Option<&str>, field_name: &str) -> Result<Option<NaiveDate>, String> {
    match raw.map(str::trim) {
        Some(s) if !s.is_empty() => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| format!("invalid {field_name} '{s}': expected YYYY-MM-DD")),
        _ => Ok(None),
    }
}

pub fn create_task_from_payload(state: &GuiAppState, payload: GuiCreateTaskPayload) -> Result<Task, String> {
    let mut db = state.open_db()?;
    let status = match payload.status.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(TaskStatus::from_str(s).map_err(|e| e.to_string())?),
        _ => None,
    };
    let due_date = parse_optional_date(payload.due_date.as_deref(), "due_date")?;
    let sgdb_date = parse_optional_date(payload.sgdb_date.as_deref(), "sgdb_date")?;

    let created = create_task(
        &mut db,
        CreateTaskInput {
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
            reporter: payload.reporter,
            assignee: payload.assignee,
            evaluator: payload.evaluator,
            ..Default::default()
        },
        &state.active_persona,
    )
    .map_err(|e| e.to_string())?;

    for att in payload.attachments {
        attach_existing_file(&mut db, &created.id, None, &att.path, att.description.as_deref()).map_err(|e| e.to_string())?;
    }

    for rel in payload.initial_relations {
        let kind = RelationInputKind::from_str(rel.relation.trim()).map_err(|e| e.to_string())?;
        link_tasks(&mut db, &created.id, kind, rel.target_id.trim()).map_err(|e| e.to_string())?;
    }

    get_task(&db, &created.id).map_err(|e| e.to_string())
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
fn set_active_persona(state: State<'_, SharedGuiState>, persona: String) -> Result<AppSessionSnapshot, String> {
    let norm = normalize_persona(&persona).map_err(|e| e.to_string())?;
    let mut guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    guard.active_persona = norm.clone();
    guard.config.default_persona = norm;
    guard.save_config();
    build_session_snapshot(&guard)
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
fn switch_database(state: State<'_, SharedGuiState>, path: Option<String>, use_global: bool) -> Result<AppSessionSnapshot, String> {
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
    build_session_snapshot(&guard)
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
    persona: Option<String>,
    preset: Option<String>,
    limit: Option<usize>,
) -> Result<FocusQueueResponse, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let target_persona = persona
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_persona);
    let target_preset = match preset.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(p) => FocusPreset::from_str(p).map_err(|e| e.to_string())?,
        None => guard.active_preset,
    };
    let top_k = limit.unwrap_or(guard.config.focus_limit);
    let today = Utc::now().date_naive();

    compute_focus_queue(&db, target_persona, target_preset, guard.config.custom_weights, Some(top_k), today, guard.config.scoring)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_blocked_tasks_data(
    state: State<'_, SharedGuiState>,
    persona: Option<String>,
    tag: Option<String>,
) -> Result<Vec<BlockedTaskEntry>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let target_persona = persona
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_persona);
    get_blocked_tasks(&db, target_persona, tag.as_deref()).map_err(|e| e.to_string())
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
fn get_eisenhower_data(state: State<'_, SharedGuiState>, persona_filter: Option<String>) -> Result<Vec<EisenhowerClassification>, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    let db = guard.open_db()?;
    let today = Utc::now().date_naive();
    list_eisenhower_matrix(&db, persona_filter.as_deref(), today, guard.config.scoring).map_err(|e| e.to_string())
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
    search_tasks(&db, &SearchQuery { query, status: parsed_status, tag, assignee, reporter, evaluator, limit }).map_err(|e| e.to_string())
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
fn create_new_task(state: State<'_, SharedGuiState>, payload: GuiCreateTaskPayload) -> Result<Task, String> {
    let guard = state
        .lock()
        .map_err(|_| "state lock poisoned".to_string())?;
    create_task_from_payload(&guard, payload)
}

#[tauri::command]
fn update_existing_task(state: State<'_, SharedGuiState>, payload: GuiUpdateTaskPayload) -> Result<Task, String> {
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

    update_task(
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
            reporter: payload.reporter,
            assignee: payload.assignee,
            evaluator: payload.evaluator,
        },
    )
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
    let author_persona = author
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&guard.active_persona);

    let outcome = add_task_note(&mut db, &task_id, author_persona, &body, spawn_subtasks).map_err(|e| e.to_string())?;

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

fn tauri_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

pub fn run() {
    let shared_state: SharedGuiState = Arc::new(Mutex::new(GuiAppState::initialize()));

    tauri::Builder::default()
        .manage(shared_state.clone())
        .setup(move |app| {
            spawn_db_change_watcher(app.handle().clone(), shared_state.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_session_state,
            set_active_persona,
            set_focus_preset,
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
            update_existing_task,
            transition_status,
            remove_task,
            add_comment_note,
            remove_comment_note,
            add_relation,
            remove_relation,
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
    fn gui_backend_creates_tasks_manages_attachments_and_loads_full_detail() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gui_test.db");
        let config_path = dir.path().join("config.toml");

        let state = GuiAppState {
            db_path,
            config_path,
            config: AppConfig::default(),
            active_persona: "alice".to_string(),
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
                due_date: Some("2026-11-12".to_string()),
                sgdb_date: Some("2026-11-10".to_string()),
                tags: vec!["gui".to_string(), "a11y".to_string()],
                attachments: vec![AttachmentInput {
                    path: sample_img.to_string_lossy().to_string(),
                    description: Some("Drawer wireframe screenshot".to_string()),
                }],
                ..Default::default()
            },
        )
        .unwrap();

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
        assert_eq!(about.repository_url, "https://github.com/dmbeta/WSID");
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
