use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const JSONRPC_VERSION: &str = "2.0";
pub const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2024-11-05";

pub const PARSE_ERROR_CODE: i64 = -32700;
pub const INVALID_REQUEST_CODE: i64 = -32600;
pub const METHOD_NOT_FOUND_CODE: i64 = -32601;
pub const INVALID_PARAMS_CODE: i64 = -32602;
pub const INTERNAL_ERROR_CODE: i64 = -32603;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.to_string(), id, result: Some(result), error: None }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self { jsonrpc: JSONRPC_VERSION.to_string(), id, result: None, error: Some(JsonRpcError { code, message: message.into(), data }) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    #[serde(default)]
    pub protocol_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTextContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl ToolTextContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self { content_type: "text".to_string(), text: text.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolTextContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    pub fn ok_json<T: Serialize>(payload: &T) -> Self {
        let structured = serde_json::to_value(payload).unwrap_or(Value::Null);
        let pretty = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "{}".to_string());
        Self { content: vec![ToolTextContent::text(pretty)], structured_content: Some(structured), is_error: Some(false) }
    }

    pub fn err_message(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self { content: vec![ToolTextContent::text(msg.clone())], structured_content: Some(json!({ "error": msg })), is_error: Some(true) }
    }
}

pub fn build_initialize_result(requested_protocol_version: Option<&str>) -> Value {
    let protocol_version = requested_protocol_version.unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "wsid-mcp",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "WSID ('What Should I Do?') MCP server. Use `get_focus_queue` to see top prioritized actionable tasks and a summary of blocked tasks for a persona; use `get_blocked_tasks` to inspect blockers in detail; use `add_task_note` with `@persona`, `@takenote:<persona>`, or `#AB1234` to collaborate across human and AI personas."
    })
}

pub fn list_mcp_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "get_focus_queue".to_string(),
            description: "Returns the top-K prioritized actionable tasks for a persona along with a grouped summary of tasks that are blocked or waiting on review.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "persona": {
                        "type": "string",
                        "description": "Persona identifier (e.g. 'claude', 'human'). Defaults to the configured default persona when omitted."
                    },
                    "preset": {
                        "type": "string",
                        "enum": ["balanced", "quick_wins", "eat_the_frog", "unblock_others"],
                        "description": "Optional scoring preset override."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of actionable tasks to return."
                    }
                }
            }),
        },
        ToolDefinition {
            name: "get_blocked_tasks".to_string(),
            description: "Returns incomplete tasks assigned to a persona that are currently blocked, waiting on subtasks, or in_review, annotated with who and what they are waiting on.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "persona": {
                        "type": "string",
                        "description": "Persona identifier. Defaults to the configured default persona when omitted."
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional single-word tag/project filter."
                    }
                }
            }),
        },
        ToolDefinition {
            name: "get_task".to_string(),
            description: "Fetches full details for a task by short ID (e.g. 'AB1234'), including its tags, canonical relations, direct blockers, subtasks, and chronological task_notes.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Short task ID (2 letters + 4 digits, e.g. 'AB1234')."
                    }
                },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "search_tasks".to_string(),
            description: "Hybrid FTS5 trigram + fuzzy search across task IDs, titles, descriptions, tags, and personas with optional structured filters.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Fuzzy search query string (may be empty when using structured filters)."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["backlog", "todo", "in_progress", "in_review", "stuck", "completed"]
                    },
                    "tag": {
                        "type": "string"
                    },
                    "assignee": {
                        "type": "string"
                    },
                    "reporter": {
                        "type": "string"
                    },
                    "evaluator": {
                        "type": "string"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200
                    }
                }
            }),
        },
        ToolDefinition {
            name: "create_task".to_string(),
            description: "Creates a new task with optional SGDB/due dates (YYYY-MM-DD), time estimate in minutes, difficulty (1..=5), importance (1..=5), single-word tags, roles, and initial relations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "status": {
                        "type": "string",
                        "enum": ["backlog", "todo", "in_progress", "in_review", "stuck", "completed"]
                    },
                    "due_date": { "type": "string", "description": "Optional hard deadline in YYYY-MM-DD format." },
                    "sgdb_date": { "type": "string", "description": "Optional 'Should Get Done By' date in YYYY-MM-DD format (must be <= due_date if both are set)." },
                    "time_estimate_minutes": { "type": "integer", "minimum": 0 },
                    "difficulty": { "type": "integer", "minimum": 1, "maximum": 5 },
                    "importance": { "type": "integer", "minimum": 1, "maximum": 5 },
                    "urgent_override": { "type": "boolean" },
                    "manual_priority": { "type": "integer", "minimum": -5, "maximum": 5 },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "reporter": { "type": "string" },
                    "assignee": { "type": "string" },
                    "evaluator": { "type": "string" },
                    "relations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "relation": {
                                    "type": "string",
                                    "enum": ["blocks", "blocked_by", "parent_task_of", "sub_task_of", "parallel", "related"]
                                },
                                "target_id": { "type": "string" }
                            },
                            "required": ["relation", "target_id"]
                        }
                    }
                },
                "required": ["title"]
            }),
        },
        ToolDefinition {
            name: "update_task".to_string(),
            description: "Updates an existing task's metadata, dates (pass null to clear due_date or sgdb_date), estimates, roles, or tags.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "due_date": { "type": ["string", "null"], "description": "YYYY-MM-DD date string or null to clear." },
                    "sgdb_date": { "type": ["string", "null"], "description": "YYYY-MM-DD date string or null to clear." },
                    "time_estimate_minutes": { "type": ["integer", "null"], "minimum": 0 },
                    "difficulty": { "type": "integer", "minimum": 1, "maximum": 5 },
                    "importance": { "type": "integer", "minimum": 1, "maximum": 5 },
                    "urgent_override": { "type": "boolean" },
                    "manual_priority": { "type": "integer", "minimum": -5, "maximum": 5 },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "reporter": { "type": "string" },
                    "assignee": { "type": "string" },
                    "evaluator": { "type": "string" }
                },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Transitions a task's status. Completing a task when assignee != evaluator automatically transitions it to 'in_review' and spawns a blocking review subtask for the evaluator; completing a review subtask automatically completes the parent task.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "status": {
                        "type": "string",
                        "enum": ["backlog", "todo", "in_progress", "in_review", "stuck", "completed"]
                    }
                },
                "required": ["id", "status"]
            }),
        },
        ToolDefinition {
            name: "add_task_note".to_string(),
            description: "Appends a note to a task. Supports `@<persona>` (spawns an urgent blocking subtask), `@takenote:<persona>` (spawns an urgent non-blocking related task), and `#<TASK_ID>` (links via related).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Target task ID (e.g. 'AB1234')." },
                    "author": { "type": "string", "description": "Author persona name (defaults to configured default persona if omitted)." },
                    "body": { "type": "string", "description": "Comment markdown text." },
                    "spawn_subtasks": {
                        "type": "boolean",
                        "default": true,
                        "description": "Whether `@persona` and `@takenote:<persona>` mentions should spawn subtasks."
                    }
                },
                "required": ["task_id", "body"]
            }),
        },
        ToolDefinition {
            name: "link_tasks".to_string(),
            description: "Links two tasks using 'blocks', 'blocked_by', 'parent_task_of', 'sub_task_of', 'parallel', or 'related', enforcing DAG cycle prevention.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string" },
                    "relation": {
                        "type": "string",
                        "enum": ["blocks", "blocked_by", "parent_task_of", "sub_task_of", "parallel", "related"]
                    },
                    "target_id": { "type": "string" }
                },
                "required": ["source_id", "relation", "target_id"]
            }),
        },
        ToolDefinition {
            name: "unlink_tasks".to_string(),
            description: "Removes a relation between two tasks.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string" },
                    "relation": {
                        "type": "string",
                        "enum": ["blocks", "blocked_by", "parent_task_of", "sub_task_of", "parallel", "related"]
                    },
                    "target_id": { "type": "string" }
                },
                "required": ["source_id", "relation", "target_id"]
            }),
        },
        ToolDefinition {
            name: "unblock_subtask".to_string(),
            description: "One-step cleanup for overzealous mention/subtask blockers: removes blocking/subtask edges and either marks the subtask completed (dismiss=true) or converts it into a non-blocking related task (dismiss=false).".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "dismiss": { "type": "boolean", "default": true }
                },
                "required": ["id"]
            }),
        },
    ]
}
