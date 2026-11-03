//! WSID Model Context Protocol (MCP) stdio server library.

pub mod protocol;
pub mod server;
pub mod tools;

pub use protocol::{
    CallToolParams, CallToolResult, DEFAULT_MCP_PROTOCOL_VERSION, INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, INVALID_REQUEST_CODE,
    InitializeParams, JSONRPC_VERSION, JsonRpcError, JsonRpcRequest, JsonRpcResponse, METHOD_NOT_FOUND_CODE, PARSE_ERROR_CODE,
    ToolDefinition, ToolTextContent, build_initialize_result, list_mcp_tools,
};
pub use server::McpServer;
pub use tools::{TaskDetailResponse, build_task_detail, dispatch_tool};

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::NaiveDate;
    use serde_json::{Value, json};
    use wsid_core::{AppConfig, open_in_memory_db};

    use super::*;

    fn test_server() -> McpServer {
        let db = open_in_memory_db().unwrap();
        let config = AppConfig { default_user: "claude".to_string(), ..AppConfig::default() };
        let today = NaiveDate::from_ymd_opt(2099, 11, 1).unwrap();
        McpServer::new(db, config).with_today_override(today)
    }

    fn call_tool(server: &mut McpServer, id: i64, name: &str, arguments: Value) -> Value {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(id)),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": name,
                "arguments": arguments,
            })),
        };
        let resp = server.handle_request(req).expect("expected response");
        assert!(resp.error.is_none(), "unexpected JSON-RPC error: {:?}", resp.error);
        let result = resp.result.expect("expected result");
        assert_eq!(result.get("isError").and_then(Value::as_bool), Some(false), "tool returned error: {result}");
        result
            .get("structuredContent")
            .cloned()
            .expect("expected structuredContent")
    }

    #[test]
    fn handles_stdio_jsonrpc_lifecycle_without_polluting_stdout_on_notifications() {
        let mut server = test_server();

        let input_lines = [
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05" }
            })
            .to_string(),
            // Notification (no `id`) must produce zero stdout lines
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            })
            .to_string(),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list"
            })
            .to_string(),
            // Malformed JSON line produces a parse error response
            "{not valid json".to_string(),
        ]
        .join("\n")
            + "\n";

        let reader = Cursor::new(input_lines.into_bytes());
        let mut output = Vec::new();

        server.run_io(reader, &mut output).unwrap();

        let stdout_str = String::from_utf8(output).unwrap();
        let responses: Vec<Value> = stdout_str
            .lines()
            .map(|line| serde_json::from_str(line).expect("every stdout line must be valid JSON"))
            .collect();

        // 3 responses (initialize, tools/list, and parse error); notification produced no line
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[0]["result"]["protocolVersion"], "2024-11-05");

        assert_eq!(responses[1]["id"], 2);
        let tool_names: Vec<&str> = responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(tool_names.contains(&"get_focus_queue"));
        assert!(tool_names.contains(&"get_blocked_tasks"));
        assert!(tool_names.contains(&"get_task"));
        assert!(tool_names.contains(&"search_tasks"));
        assert!(tool_names.contains(&"create_task"));
        assert!(tool_names.contains(&"update_task"));
        assert!(tool_names.contains(&"update_task_status"));
        assert!(tool_names.contains(&"add_task_note"));
        assert!(tool_names.contains(&"link_tasks"));
        assert!(tool_names.contains(&"unlink_tasks"));

        assert_eq!(responses[2]["error"]["code"], PARSE_ERROR_CODE);
    }

    #[test]
    fn executes_end_to_end_mcp_collaboration_focus_and_review_workflow() {
        let mut server = test_server();

        // 1. Create a task assigned to `claude` with evaluator `human`
        let created_res = call_tool(
            &mut server,
            1,
            "create_task",
            json!({
                "title": "Implement MCP stdio server",
                "description": "Support get_focus_queue and get_blocked_tasks",
                "due_date": "2099-11-05",
                "sgdb_date": "2099-11-03",
                "time_estimate_minutes": 90,
                "difficulty": 3,
                "importance": 5,
                "tags": ["mcp", "backend"],
                "reporter": "human",
                "assignee": "claude",
                "evaluator": "human"
            }),
        );
        let task_id = created_res["task"]["id"].as_str().unwrap().to_string();

        // 2. Create a second task for cross-reference testing
        let other_res = call_tool(
            &mut server,
            2,
            "create_task",
            json!({
                "title": "Document CLI flags",
                "tags": ["docs"],
                "assignee": "docs-agent",
                "evaluator": "human"
            }),
        );
        let other_id = other_res["task"]["id"].as_str().unwrap().to_string();

        // 3. Query `get_focus_queue` for `claude` -> `task_id` is actionable
        let focus_1 = call_tool(&mut server, 3, "get_focus_queue", json!({ "user": "claude" }));
        let actionable_1 = focus_1["actionable_tasks"].as_array().unwrap();
        assert_eq!(actionable_1.len(), 1);
        assert_eq!(actionable_1[0]["task"]["id"], task_id);

        // 4. `claude` posts a note with `@bob` (blocking question) and `@takenote:docs-agent` referencing `#<other_id>`
        let note_res = call_tool(
            &mut server,
            4,
            "add_task_note",
            json!({
                "task_id": task_id,
                "author": "claude",
                "body": format!("@bob should we include structuredContent in CallToolResult? Also @takenote:docs-agent see #{other_id}")
            }),
        );
        let spawned = note_res["spawned_tasks"].as_array().unwrap();
        assert_eq!(spawned.len(), 2);

        let bob_subtask_id = spawned.iter().find(|t| t["assignee"] == "bob").unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        // 5. Now `get_focus_queue` for `claude` has 0 actionable tasks, and `blocked_summary` explains it's waiting on `bob`
        let focus_blocked = call_tool(&mut server, 5, "get_focus_queue", json!({ "user": "claude" }));
        assert!(
            focus_blocked["actionable_tasks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let summary = focus_blocked["blocked_summary"].as_array().unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0]["waiting_on_assignee"], "bob");
        assert_eq!(summary[0]["blocker_kind"], "mention");

        // `get_blocked_tasks` returns full blocker details
        let blocked_detail = call_tool(&mut server, 6, "get_blocked_tasks", json!({ "user": "claude", "tag": "backend" }));
        let blocked_list = blocked_detail["blocked_tasks"].as_array().unwrap();
        assert_eq!(blocked_list.len(), 1);
        assert_eq!(blocked_list[0]["task_id"], task_id);
        assert_eq!(blocked_list[0]["blocked_by"][0]["blocker_id"], bob_subtask_id);

        // 6. `bob` completes the mention subtask -> `claude` is unblocked
        call_tool(&mut server, 7, "update_task_status", json!({ "id": bob_subtask_id, "status": "completed" }));

        // 7. `claude` completes `task_id` -> since `assignee ("claude") != evaluator ("human")`,
        //    it transitions to `in_review` and spawns a review subtask for `human`
        let transition_res = call_tool(&mut server, 8, "update_task_status", json!({ "id": task_id, "status": "completed" }));
        assert_eq!(transition_res["task"]["status"], "in_review");
        let review_id = transition_res["spawned_review_task"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        // 8. `human` approves by completing `review_id` -> parent `task_id` auto-transitions to `completed`
        let approve_res = call_tool(&mut server, 9, "update_task_status", json!({ "id": review_id, "status": "completed" }));
        assert_eq!(approve_res["auto_completed_parent_tasks"][0]["id"], task_id);

        // 9. Verify `get_task` and `search_tasks`
        let detail_res = call_tool(&mut server, 10, "get_task", json!({ "id": task_id }));
        assert_eq!(detail_res["task"]["status"], "completed");
        assert_eq!(detail_res["notes"].as_array().unwrap().len(), 1);

        let search_res = call_tool(&mut server, 11, "search_tasks", json!({ "query": "stdio", "status": "completed" }));
        assert_eq!(search_res["count"], 3);
        let hit_ids: Vec<&str> = search_res["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["task"]["id"].as_str().unwrap())
            .collect();
        assert!(hit_ids.contains(&task_id.as_str()));
    }

    #[test]
    fn update_task_clears_nullable_dates_and_link_tasks_rejects_cycles() {
        let mut server = test_server();

        let a = call_tool(
            &mut server,
            1,
            "create_task",
            json!({
                "title": "Task A",
                "due_date": "2099-11-10",
                "sgdb_date": "2099-11-08"
            }),
        )["task"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let b = call_tool(&mut server, 2, "create_task", json!({ "title": "Task B" }))["task"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        // Clear `sgdb_date` via explicit `null` while updating `title`
        let updated = call_tool(
            &mut server,
            3,
            "update_task",
            json!({
                "id": a,
                "title": "Task A Updated",
                "sgdb_date": null
            }),
        );
        assert_eq!(updated["title"], "Task A Updated");
        assert!(updated["sgdb_date"].is_null());
        assert_eq!(
            updated["due_date"].as_str().unwrap(),
            wsid_core::parse_deadline_datetime("due_date", "2099-11-10")
                .unwrap()
                .0
        );

        // Link A blocks B, then attempt B blocks A (must return tool error with cycle message)
        call_tool(&mut server, 4, "link_tasks", json!({ "source_id": a, "relation": "blocks", "target_id": b }));

        let cycle_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(5)),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "link_tasks",
                "arguments": { "source_id": b, "relation": "blocks", "target_id": a }
            })),
        };
        let cycle_resp = server.handle_request(cycle_req).unwrap();
        let res_obj = cycle_resp.result.unwrap();
        assert_eq!(res_obj["isError"], true);
        assert!(
            res_obj["structuredContent"]["error"]
                .as_str()
                .unwrap()
                .contains("cycle detected")
        );
    }

    #[test]
    fn exposes_user_matrix_bulk_tools_and_duration_strings() {
        let mut server = test_server();
        let a = call_tool(&mut server, 1, "create_task", json!({ "title": "A", "time_estimate_minutes": "1h30m", "importance": 1 }));
        assert_eq!(a["task"]["time_estimate_minutes"], 90);
        let b = call_tool(&mut server, 2, "create_task", json!({ "title": "B" }));
        let (a, b) = (a["task"]["id"].as_str().unwrap().to_string(), b["task"]["id"].as_str().unwrap().to_string());

        let users = call_tool(&mut server, 3, "list_users", json!({}));
        assert!(
            users["users"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u == "claude")
        );
        call_tool(&mut server, 4, "register_user", json!({ "user": "newguy" }));
        let kind = call_tool(&mut server, 4, "set_user_kind", json!({ "user": "newguy", "kind": "Claude" }));
        assert_eq!(kind["kind"], "claude");
        let users = call_tool(&mut server, 4, "list_users", json!({}));
        assert_eq!(users["kinds"]["newguy"], "claude");
        let users = call_tool(&mut server, 5, "list_users", json!({}));
        assert!(
            users["users"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u == "newguy")
        );

        let matrix = call_tool(&mut server, 6, "get_eisenhower_matrix", json!({}));
        assert_eq!(matrix["count"], 2);

        let bulk = call_tool(
            &mut server,
            7,
            "bulk_update_tasks",
            json!({ "ids": [a, b, "ZZ9999"], "importance_at_least": 4, "add_tags": ["bulk"] }),
        );
        assert_eq!(bulk["updated"].as_array().unwrap().len(), 2);
        assert_eq!(bulk["failures"].as_array().unwrap().len(), 1);
        assert_eq!(bulk["updated"][0]["importance"], 4);

        let del = call_tool(&mut server, 8, "bulk_delete_tasks", json!({ "ids": [a, b] }));
        assert_eq!(del["deleted"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn create_task_with_bad_relation_is_rejected_atomically() {
        let mut server = test_server();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": "create_task",
                "arguments": { "title": "Orphan", "relations": [{ "relation": "blocks", "target_id": "AB1333" }] }
            })),
        };
        let res = server.handle_request(req).unwrap().result.unwrap();
        assert_eq!(res["isError"], true);
        assert!(
            res["structuredContent"]["error"]
                .as_str()
                .unwrap()
                .contains("No task AB1333")
        );

        let search = call_tool(&mut server, 2, "search_tasks", json!({}));
        assert_eq!(search["count"], 0, "no half-created task left behind");
    }
}
