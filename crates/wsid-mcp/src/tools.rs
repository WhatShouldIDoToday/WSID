use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use wsid_core::{
    AppConfig, AttachmentInput, CreateTaskInput, Database, FocusPreset, RelationInputKind, SearchQuery, Task, TaskNote, TaskRelation,
    TaskStatus, UpdateTaskInput, WsidError, add_task_note, attach_existing_file, compute_focus_queue, create_task, delete_task,
    delete_task_note, get_blocked_tasks, get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks,
    get_parallel_peers, get_related_tasks, get_task, link_tasks, list_task_notes, list_task_relations, remove_attachment, search_tasks,
    transition_task_status, unblock_or_dismiss_subtask, unlink_tasks, update_attachment_description, update_task,
};

use crate::protocol::CallToolResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetailResponse {
    pub task: Task,
    pub relations: Vec<TaskRelation>,
    pub blockers: Vec<Task>,
    pub blocked_tasks: Vec<Task>,
    pub subtasks: Vec<Task>,
    pub parents: Vec<Task>,
    pub parallel_peers: Vec<Task>,
    pub related_tasks: Vec<Task>,
    pub notes: Vec<TaskNote>,
}

#[derive(Debug, Clone, Deserialize)]
struct GetFocusQueueArgs {
    pub persona: Option<String>,
    pub preset: Option<FocusPreset>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct GetBlockedTasksArgs {
    pub persona: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GetTaskArgs {
    #[serde(alias = "task_id")]
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchTasksArgs {
    #[serde(default)]
    pub query: String,
    pub status: Option<TaskStatus>,
    pub tag: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub evaluator: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct InitialRelationSpec {
    pub relation: RelationInputKind,
    pub target_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateTaskArgs {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub due_date: Option<String>,
    pub sgdb_date: Option<String>,
    pub time_estimate_minutes: Option<i64>,
    pub difficulty: Option<u8>,
    pub importance: Option<u8>,
    pub urgent_override: Option<bool>,
    pub manual_priority: Option<i8>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub reporter: Option<String>,
    pub assignee: Option<String>,
    pub evaluator: Option<String>,
    #[serde(default)]
    pub relations: Vec<InitialRelationSpec>,
    #[serde(default)]
    pub attachments: Vec<AttachmentInput>,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateTaskStatusArgs {
    #[serde(alias = "task_id")]
    pub id: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Deserialize)]
struct AddTaskNoteArgs {
    #[serde(alias = "id")]
    pub task_id: String,
    #[serde(alias = "author_persona")]
    pub author: Option<String>,
    pub body: String,
    pub spawn_subtasks: Option<bool>,
    #[serde(default)]
    pub attachments: Vec<AttachmentInput>,
}

#[derive(Debug, Clone, Deserialize)]
struct AttachFileArgs {
    #[serde(alias = "id")]
    pub task_id: String,
    pub note_id: Option<i64>,
    #[serde(alias = "file_path")]
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateAttachmentArgs {
    #[serde(alias = "id")]
    pub attachment_id: i64,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoveAttachmentArgs {
    #[serde(alias = "id")]
    pub attachment_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct DeleteTaskArgs {
    #[serde(alias = "task_id")]
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DeleteTaskNoteArgs {
    #[serde(alias = "id")]
    pub note_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct LinkTasksArgs {
    pub source_id: String,
    #[serde(alias = "relation_type")]
    pub relation: RelationInputKind,
    pub target_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UnblockSubtaskArgs {
    #[serde(alias = "task_id")]
    pub id: String,
    pub dismiss: Option<bool>,
}

/// Dispatches an MCP `tools/call` invocation against `db` and returns a [`CallToolResult`].
pub fn dispatch_tool(
    db: &mut Database,
    config: &AppConfig,
    tool_name: &str,
    arguments: Option<Value>,
    today: NaiveDate,
) -> Result<CallToolResult, String> {
    let args_val = arguments.unwrap_or_else(|| json!({}));

    let outcome: Result<CallToolResult, WsidError> = match tool_name {
        "get_focus_queue" => {
            let args: GetFocusQueueArgs = parse_args(args_val)?;
            let persona = args
                .persona
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&config.default_persona);
            let preset = args.preset.unwrap_or(config.focus_preset);
            let custom_weights = if args.preset.is_none() { config.custom_weights } else { None };
            let limit = args.limit.or(Some(config.focus_limit));

            compute_focus_queue(db, persona, preset, custom_weights, limit, today, config.scoring)
                .map(|resp| CallToolResult::ok_json(&resp))
        }
        "get_blocked_tasks" => {
            let args: GetBlockedTasksArgs = parse_args(args_val)?;
            let persona = args
                .persona
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&config.default_persona);

            get_blocked_tasks(db, persona, args.tag.as_deref()).map(|entries| {
                CallToolResult::ok_json(&json!({
                    "persona": persona,
                    "blocked_tasks": entries,
                }))
            })
        }
        "get_task" => {
            let args: GetTaskArgs = parse_args(args_val)?;
            build_task_detail(db, &args.id).map(|detail| CallToolResult::ok_json(&detail))
        }
        "search_tasks" => {
            let args: SearchTasksArgs = parse_args(args_val)?;
            let query = SearchQuery {
                query: args.query,
                status: args.status,
                tag: args.tag,
                assignee: args.assignee,
                reporter: args.reporter,
                evaluator: args.evaluator,
                limit: args.limit,
            };
            search_tasks(db, &query).map(|hits| {
                CallToolResult::ok_json(&json!({
                    "count": hits.len(),
                    "hits": hits,
                }))
            })
        }
        "create_task" => {
            let args: CreateTaskArgs = parse_args(args_val)?;
            handle_create_task(db, config, args)
        }
        "update_task" => handle_update_task(db, args_val),
        "update_task_status" => {
            let args: UpdateTaskStatusArgs = parse_args(args_val)?;
            transition_task_status(db, &args.id, args.status).map(|res| CallToolResult::ok_json(&res))
        }
        "add_task_note" => {
            let args: AddTaskNoteArgs = parse_args(args_val)?;
            let author = args
                .author
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&config.default_persona);
            let spawn_subtasks = args.spawn_subtasks.unwrap_or(true);

            add_task_note(db, &args.task_id, author, &args.body, spawn_subtasks).and_then(|mut res| {
                for att in &args.attachments {
                    let saved = attach_existing_file(db, &res.note.task_id, Some(res.note.id), &att.path, att.description.as_deref())?;
                    res.note.attachments.push(saved);
                }
                Ok(CallToolResult::ok_json(&res))
            })
        }
        "attach_file" => {
            let args: AttachFileArgs = parse_args(args_val)?;
            attach_existing_file(db, &args.task_id, args.note_id, &args.path, args.description.as_deref())
                .map(|att| CallToolResult::ok_json(&att))
        }
        "update_attachment" => {
            let args: UpdateAttachmentArgs = parse_args(args_val)?;
            update_attachment_description(db, args.attachment_id, &args.description).map(|att| CallToolResult::ok_json(&att))
        }
        "remove_attachment" => {
            let args: RemoveAttachmentArgs = parse_args(args_val)?;
            remove_attachment(db, args.attachment_id).map(|removed| CallToolResult::ok_json(&json!({ "removed": removed })))
        }
        "delete_task" => {
            let args: DeleteTaskArgs = parse_args(args_val)?;
            delete_task(db, &args.id).map(|()| CallToolResult::ok_json(&json!({ "deleted": true, "id": args.id })))
        }
        "delete_task_note" => {
            let args: DeleteTaskNoteArgs = parse_args(args_val)?;
            delete_task_note(db, args.note_id).map(|()| CallToolResult::ok_json(&json!({ "deleted": true, "note_id": args.note_id })))
        }
        "link_tasks" => {
            let args: LinkTasksArgs = parse_args(args_val)?;
            link_tasks(db, &args.source_id, args.relation, &args.target_id).map(|rel| CallToolResult::ok_json(&rel))
        }
        "unlink_tasks" => {
            let args: LinkTasksArgs = parse_args(args_val)?;
            unlink_tasks(db, &args.source_id, args.relation, &args.target_id)
                .map(|removed| CallToolResult::ok_json(&json!({ "removed": removed })))
        }
        "unblock_subtask" => {
            let args: UnblockSubtaskArgs = parse_args(args_val)?;
            let dismiss = args.dismiss.unwrap_or(true);
            unblock_or_dismiss_subtask(db, &args.id, dismiss).map(|task| CallToolResult::ok_json(&task))
        }
        unknown => return Err(format!("unknown MCP tool: '{unknown}'")),
    };

    Ok(match outcome {
        Ok(res) => res,
        Err(err) => CallToolResult::err_message(err.to_string()),
    })
}

fn parse_args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("invalid tool arguments: {e}"))
}

fn parse_date_field(field_name: &str, raw: Option<&str>) -> Result<Option<NaiveDate>, WsidError> {
    match raw {
        Some(s) if !s.trim().is_empty() => NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .map(Some)
            .map_err(|_| WsidError::Validation(format!("invalid {field_name} '{s}': expected YYYY-MM-DD"))),
        _ => Ok(None),
    }
}

fn handle_create_task(db: &mut Database, config: &AppConfig, args: CreateTaskArgs) -> Result<CallToolResult, WsidError> {
    let due_date = parse_date_field("due_date", args.due_date.as_deref())?;
    let sgdb_date = parse_date_field("sgdb_date", args.sgdb_date.as_deref())?;

    let created = create_task(
        db,
        CreateTaskInput {
            title: args.title,
            description: args.description,
            status: args.status,
            origin_kind: None,
            origin_note_id: None,
            due_date,
            sgdb_date,
            time_estimate_minutes: args.time_estimate_minutes,
            difficulty: args.difficulty,
            importance: args.importance,
            urgent_override: args.urgent_override,
            manual_priority: args.manual_priority,
            tags: args.tags,
            reporter: args.reporter,
            assignee: args.assignee,
            evaluator: args.evaluator,
        },
        &config.default_persona,
    )?;

    let mut linked_relations = Vec::with_capacity(args.relations.len());
    for rel_spec in args.relations {
        let rel = link_tasks(db, &created.id, rel_spec.relation, &rel_spec.target_id)?;
        linked_relations.push(rel);
    }

    for att in &args.attachments {
        attach_existing_file(db, &created.id, None, &att.path, att.description.as_deref())?;
    }
    let refreshed = if args.attachments.is_empty() { created } else { get_task(db, &created.id)? };

    Ok(CallToolResult::ok_json(&json!({
        "task": refreshed,
        "relations": linked_relations,
    })))
}

fn handle_update_task(db: &mut Database, args_val: Value) -> Result<CallToolResult, WsidError> {
    let obj = args_val
        .as_object()
        .ok_or_else(|| WsidError::Validation("update_task arguments must be a JSON object".to_string()))?;

    let id = obj
        .get("id")
        .or_else(|| obj.get("task_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| WsidError::Validation("missing required string field 'id'".to_string()))?;

    let title = parse_optional_string(obj, "title")?;
    let description = parse_optional_string(obj, "description")?;
    let status = match parse_optional_string(obj, "status")? {
        Some(s) => Some(TaskStatus::from_str(&s)?),
        None => None,
    };

    let due_date = parse_nullable_date(obj, "due_date")?;
    let sgdb_date = parse_nullable_date(obj, "sgdb_date")?;
    let time_estimate_minutes = parse_nullable_i64(obj, "time_estimate_minutes")?;
    let difficulty = parse_optional_u8(obj, "difficulty")?;
    let importance = parse_optional_u8(obj, "importance")?;
    let urgent_override = parse_optional_bool(obj, "urgent_override")?;
    let manual_priority = parse_optional_i8(obj, "manual_priority")?;
    let tags = parse_optional_string_vec(obj, "tags")?;
    let reporter = parse_optional_string(obj, "reporter")?;
    let assignee = parse_optional_string(obj, "assignee")?;
    let evaluator = parse_optional_string(obj, "evaluator")?;

    let updated = update_task(
        db,
        id,
        UpdateTaskInput {
            title,
            description,
            status,
            due_date,
            sgdb_date,
            time_estimate_minutes,
            difficulty,
            importance,
            urgent_override,
            manual_priority,
            tags,
            reporter,
            assignee,
            evaluator,
        },
    )?;

    Ok(CallToolResult::ok_json(&updated))
}

fn parse_optional_string(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, WsidError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be a string"))),
    }
}

fn parse_nullable_date(obj: &Map<String, Value>, key: &str) -> Result<Option<Option<NaiveDate>>, WsidError> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => {
            let parsed = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
                .map_err(|_| WsidError::Validation(format!("invalid {key} '{s}': expected YYYY-MM-DD or null")))?;
            Ok(Some(Some(parsed)))
        }
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be a YYYY-MM-DD string or null"))),
    }
}

fn parse_nullable_i64(obj: &Map<String, Value>, key: &str) -> Result<Option<Option<i64>>, WsidError> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(|v| Some(Some(v)))
            .ok_or_else(|| WsidError::Validation(format!("field '{key}' must be an integer"))),
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be an integer or null"))),
    }
}

fn parse_optional_u8(obj: &Map<String, Value>, key: &str) -> Result<Option<u8>, WsidError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| WsidError::Validation(format!("field '{key}' must be an integer between 0 and 255"))),
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be an integer"))),
    }
}

fn parse_optional_i8(obj: &Map<String, Value>, key: &str) -> Result<Option<i8>, WsidError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .and_then(|v| i8::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| WsidError::Validation(format!("field '{key}' must be an integer between -128 and 127"))),
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be an integer"))),
    }
}

fn parse_optional_bool(obj: &Map<String, Value>, key: &str) -> Result<Option<bool>, WsidError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be a boolean"))),
    }
}

fn parse_optional_string_vec(obj: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, WsidError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| WsidError::Validation(format!("all items in '{key}' must be strings")))?;
                out.push(s.to_string());
            }
            Ok(Some(out))
        }
        Some(_) => Err(WsidError::Validation(format!("field '{key}' must be an array of strings"))),
    }
}

pub fn build_task_detail(db: &Database, raw_id: &str) -> Result<TaskDetailResponse, WsidError> {
    let task = get_task(db, raw_id)?;
    let relations = list_task_relations(db, &task.id)?;
    let blockers = get_direct_blockers(db, &task.id)?;
    let blocked_tasks = get_direct_blocked_tasks(db, &task.id)?;
    let subtasks = get_direct_subtasks(db, &task.id)?;
    let parents = get_direct_parents(db, &task.id)?;
    let parallel_peers = get_parallel_peers(db, &task.id)?;
    let related_tasks = get_related_tasks(db, &task.id)?;
    let notes = list_task_notes(db, &task.id)?;

    Ok(TaskDetailResponse { task, relations, blockers, blocked_tasks, subtasks, parents, parallel_peers, related_tasks, notes })
}
