use std::io::{self, BufRead, Write};

use chrono::{NaiveDate, Utc};
use serde_json::{Value, json};
use wsid_core::{AppConfig, Database};

use crate::protocol::{
    CallToolParams, INVALID_PARAMS_CODE, INVALID_REQUEST_CODE, InitializeParams, JSONRPC_VERSION, JsonRpcRequest, JsonRpcResponse,
    META_CLIENT_CAPABILITIES, META_PROTOCOL_VERSION, META_SERVER_INFO, METHOD_NOT_FOUND_CODE, MODERN_MCP_PROTOCOL_VERSIONS,
    PARSE_ERROR_CODE, UNSUPPORTED_PROTOCOL_VERSION_CODE, all_supported_versions, build_discover_result, build_initialize_result,
    list_mcp_tools, modern_request_version, server_info,
};
use crate::tools::dispatch_tool;

pub struct McpServer {
    db: Database,
    config: AppConfig,
    stderr_logging: bool,
    today_override: Option<NaiveDate>,
}

impl McpServer {
    pub fn new(db: Database, config: AppConfig) -> Self {
        Self { db, config, stderr_logging: false, today_override: None }
    }

    pub fn with_stderr_logging(mut self, enabled: bool) -> Self {
        self.stderr_logging = enabled;
        self
    }

    pub fn with_today_override(mut self, today: NaiveDate) -> Self {
        self.today_override = Some(today);
        self
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn db_mut(&mut self) -> &mut Database {
        &mut self.db
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Runs the MCP JSON-RPC 2.0 loop over process `stdin` and `stdout`.
    /// All diagnostic logs (when enabled) are written strictly to `stderr`.
    pub fn run_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let reader = stdin.lock();
        let writer = stdout.lock();
        self.run_io(reader, writer)
    }

    /// Runs the newline-delimited JSON-RPC 2.0 loop over any `BufRead` and `Write` streams.
    pub fn run_io<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(response_json) = self.handle_json_line(trimmed) {
                writeln!(writer, "{response_json}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Parses and handles a single JSON-RPC line. Returns `Some(serialized_response)` for requests,
    /// or `None` for notifications.
    pub fn handle_json_line(&mut self, line: &str) -> Option<String> {
        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                self.log_stderr(&format!("MCP parse error: {err}"));
                let resp = JsonRpcResponse::error(Value::Null, PARSE_ERROR_CODE, format!("Parse error: {err}"), None);
                return serde_json::to_string(&resp).ok();
            }
        };

        let request: JsonRpcRequest = match serde_json::from_value(parsed.clone()) {
            Ok(req) => req,
            Err(err) => {
                let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                let resp = JsonRpcResponse::error(id, INVALID_REQUEST_CODE, format!("Invalid Request: {err}"), None);
                return serde_json::to_string(&resp).ok();
            }
        };

        let response = self.handle_request(request)?;
        serde_json::to_string(&response).ok()
    }

    /// Handles a parsed [`JsonRpcRequest`]. Returns `None` if the incoming message is a notification.
    pub fn handle_request(&mut self, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        if request.jsonrpc != JSONRPC_VERSION {
            let id = request.id.unwrap_or(Value::Null);
            return Some(JsonRpcResponse::error(
                id,
                INVALID_REQUEST_CODE,
                format!("Unsupported jsonrpc version '{}'; expected '2.0'", request.jsonrpc),
                None,
            ));
        }

        // Notifications do not have an `id` and must not produce a response on stdout.
        let id = match request.id.clone() {
            Some(id) => id,
            None => {
                self.log_stderr(&format!("MCP notification: {}", request.method));
                return None;
            }
        };

        self.log_stderr(&format!("MCP request: {}", request.method));

        // `initialize` selects legacy semantics. Any other request whose `_meta` carries the
        // modern protocol version is served statelessly under the 2026-07-28 rules.
        if request.method != "initialize"
            && let Some(version) = modern_request_version(request.params.as_ref()).map(str::to_owned)
        {
            return self.handle_modern_request(id, &version, request);
        }

        if request.method == "server/discover" {
            // A probe without modern `_meta` is malformed; probing clients then fall back.
            return Some(JsonRpcResponse::error(
                id,
                INVALID_PARAMS_CODE,
                format!("server/discover requires params._meta with '{META_PROTOCOL_VERSION}' and '{META_CLIENT_CAPABILITIES}'"),
                None,
            ));
        }

        self.dispatch_method(id, request)
    }

    /// Serves one stateless 2026-07-28 request: validates `_meta`, then dispatches and decorates
    /// a successful result with `resultType` and `serverInfo`.
    fn handle_modern_request(&mut self, id: Value, version: &str, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        if !MODERN_MCP_PROTOCOL_VERSIONS.contains(&version) {
            return Some(JsonRpcResponse::error(
                id,
                UNSUPPORTED_PROTOCOL_VERSION_CODE,
                "Unsupported protocol version",
                Some(json!({ "supported": all_supported_versions(), "requested": version })),
            ));
        }
        let has_capabilities = request
            .params
            .as_ref()
            .and_then(|p| p.get("_meta"))
            .and_then(|m| m.get(META_CLIENT_CAPABILITIES))
            .is_some_and(Value::is_object);
        if !has_capabilities {
            return Some(JsonRpcResponse::error(
                id,
                INVALID_PARAMS_CODE,
                format!("Missing required _meta field '{META_CLIENT_CAPABILITIES}'"),
                None,
            ));
        }
        if request.method == "server/discover" {
            return Some(JsonRpcResponse::success(id, build_discover_result()));
        }

        let mut response = self.dispatch_method(id, request)?;
        if let Some(Value::Object(result)) = response.result.as_mut() {
            result
                .entry("resultType")
                .or_insert_with(|| json!("complete"));
            if let Value::Object(meta) = result.entry("_meta").or_insert_with(|| json!({})) {
                meta.insert(META_SERVER_INFO.to_string(), server_info());
            }
        }
        Some(response)
    }

    fn dispatch_method(&mut self, id: Value, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        match request.method.as_str() {
            "initialize" => {
                let init_params: Option<InitializeParams> = request.params.and_then(|p| serde_json::from_value(p).ok());
                let requested_version = init_params
                    .as_ref()
                    .and_then(|p| p.protocol_version.as_deref());
                Some(JsonRpcResponse::success(id, build_initialize_result(requested_version)))
            }
            "ping" => Some(JsonRpcResponse::success(id, json!({}))),
            "tools/list" => {
                let tools = list_mcp_tools();
                Some(JsonRpcResponse::success(id, json!({ "tools": tools })))
            }
            "resources/list" => Some(JsonRpcResponse::success(id, json!({ "resources": [] }))),
            "prompts/list" => Some(JsonRpcResponse::success(id, json!({ "prompts": [] }))),
            "tools/call" => {
                let params_val = match request.params {
                    Some(p) => p,
                    None => {
                        return Some(JsonRpcResponse::error(id, INVALID_PARAMS_CODE, "Missing params for tools/call", None));
                    }
                };
                let call_params: CallToolParams = match serde_json::from_value(params_val) {
                    Ok(p) => p,
                    Err(err) => {
                        return Some(JsonRpcResponse::error(id, INVALID_PARAMS_CODE, format!("Invalid tools/call params: {err}"), None));
                    }
                };

                let today = self
                    .today_override
                    .unwrap_or_else(|| Utc::now().date_naive());
                match dispatch_tool(&mut self.db, &self.config, &call_params.name, call_params.arguments, today) {
                    Ok(tool_result) => {
                        let val = serde_json::to_value(tool_result).unwrap_or_else(|_| json!({}));
                        Some(JsonRpcResponse::success(id, val))
                    }
                    Err(param_err) => Some(JsonRpcResponse::error(id, INVALID_PARAMS_CODE, param_err, None)),
                }
            }
            other => Some(JsonRpcResponse::error(id, METHOD_NOT_FOUND_CODE, format!("Method not found: '{other}'"), None)),
        }
    }

    fn log_stderr(&self, message: &str) {
        if self.stderr_logging {
            eprintln!("[wsid-mcp] {message}");
        }
    }
}
