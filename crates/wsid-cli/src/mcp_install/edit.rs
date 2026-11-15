//! Comment-preserving edits to JSON, JSONC, and TOML client config files.
//!
//! Every function takes and returns text, so nothing here touches the disk except `write_atomic`.

use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// JSON, also accepting comments and trailing commas (JSONC).
    Json,
    Toml,
}

/// Read the entry `section.key` from the config text.
pub fn get(format: Format, text: &str, section: &[&str], key: &str) -> Result<Option<Value>, String> {
    match format {
        Format::Json => json_get(text, section, key),
        Format::Toml => toml_get(text, section, key),
    }
}

/// Set `section.key` to `entry`, creating missing parents and keeping everything else.
pub fn set(format: Format, text: &str, section: &[&str], key: &str, entry: &Value) -> Result<String, String> {
    match format {
        Format::Json => json_set(text, section, key, entry),
        Format::Toml => toml_set(text, section, key, entry),
    }
}

/// Remove `section.key`. Returns the new text and whether anything was removed.
pub fn remove(format: Format, text: &str, section: &[&str], key: &str) -> Result<(String, bool), String> {
    match format {
        Format::Json => json_remove(text, section, key),
        Format::Toml => toml_remove(text, section, key),
    }
}

// ---------------------------------------------------------------- JSON / JSONC

fn json_root(text: &str) -> Result<CstRootNode, String> {
    let text = if text.trim().is_empty() { "{}\n" } else { text };
    CstRootNode::parse(text, &ParseOptions::default()).map_err(|err| format!("could not parse as JSON: {err}"))
}

fn json_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(json_input).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_input(v)))
                .collect(),
        ),
    }
}

/// Walk to the object that holds `key`, without creating anything.
fn json_find_parent(root: &CstRootNode, section: &[&str]) -> Result<Option<CstObject>, String> {
    let Some(value) = root.value() else { return Ok(None) };
    let Some(mut object) = value.as_object() else {
        return Err("the top level of the file is not a JSON object".to_string());
    };
    for name in section {
        match object.get(name) {
            None => return Ok(None),
            Some(prop) => match prop.object_value() {
                Some(next) => object = next,
                None => return Err(format!("`{name}` exists but is not an object")),
            },
        }
    }
    Ok(Some(object))
}

fn json_get(text: &str, section: &[&str], key: &str) -> Result<Option<Value>, String> {
    let root = json_root(text)?;
    let Some(parent) = json_find_parent(&root, section)? else { return Ok(None) };
    Ok(parent
        .get(key)
        .and_then(|prop| prop.value())
        .and_then(|node| node.to_serde_value()))
}

fn json_set(text: &str, section: &[&str], key: &str, entry: &Value) -> Result<String, String> {
    let root = json_root(text)?;
    if root.value().is_some_and(|v| v.as_object().is_none()) {
        return Err("the top level of the file is not a JSON object".to_string());
    }
    let mut object = root.object_value_or_set();
    for name in section {
        if let Some(prop) = object.get(name)
            && prop.object_value().is_none()
        {
            return Err(format!("`{name}` exists but is not an object"));
        }
        object = object.object_value_or_set(name);
    }
    match object.get(key) {
        Some(prop) => prop.set_value(json_input(entry)),
        None => {
            object.append(key, json_input(entry));
        }
    }
    Ok(root.to_string())
}

fn json_remove(text: &str, section: &[&str], key: &str) -> Result<(String, bool), String> {
    let root = json_root(text)?;
    let Some(parent) = json_find_parent(&root, section)? else {
        return Ok((text.to_string(), false));
    };
    match parent.get(key) {
        Some(prop) => {
            prop.remove();
            Ok((root.to_string(), true))
        }
        None => Ok((text.to_string(), false)),
    }
}

// ----------------------------------------------------------------------- TOML

fn toml_doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|err| format!("could not parse as TOML: {err}"))
}

fn toml_item(entry: &Value) -> Result<Item, String> {
    Ok(match entry {
        Value::Object(map) => {
            let mut table = Table::new();
            for (k, v) in map {
                table.insert(k, toml_item(v)?);
            }
            Item::Table(table)
        }
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                match toml_item(item)? {
                    Item::Value(v) => array.push(v),
                    _ => return Err("nested tables inside arrays are not supported".to_string()),
                }
            }
            value(array)
        }
        Value::String(s) => value(s.as_str()),
        Value::Bool(b) => value(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => value(i),
            None => value(n.as_f64().unwrap_or_default()),
        },
        Value::Null => return Err("TOML has no null".to_string()),
    })
}

fn toml_to_json(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(v) => toml_value_to_json(v),
        Item::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_to_json(v)))
                .collect(),
        ),
        Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| toml_to_json(&Item::Table(t.clone())))
                .collect(),
        ),
    }
}

fn toml_value_to_json(v: &toml_edit::Value) -> Value {
    match v {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::from(*i.value()),
        toml_edit::Value::Float(f) => Value::from(*f.value()),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
        toml_edit::Value::Array(a) => Value::Array(a.iter().map(toml_value_to_json).collect()),
        toml_edit::Value::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), toml_value_to_json(v)))
                .collect(),
        ),
    }
}

fn toml_get(text: &str, section: &[&str], key: &str) -> Result<Option<Value>, String> {
    let doc = toml_doc(text)?;
    let mut item = doc.as_item();
    for name in section.iter().chain(std::iter::once(&key)) {
        match item.get(name) {
            Some(next) => item = next,
            None => return Ok(None),
        }
    }
    Ok(Some(toml_to_json(item)))
}

fn toml_set(text: &str, section: &[&str], key: &str, entry: &Value) -> Result<String, String> {
    let mut doc = toml_doc(text)?;
    let mut table = doc.as_table_mut();
    for name in section {
        if !table.contains_key(name) {
            let mut parent = Table::new();
            parent.set_implicit(true);
            table.insert(name, Item::Table(parent));
        }
        table = table
            .get_mut(name)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| format!("`{name}` exists but is not a table"))?;
    }
    table.insert(key, toml_item(entry)?);
    Ok(doc.to_string())
}

fn toml_remove(text: &str, section: &[&str], key: &str) -> Result<(String, bool), String> {
    let mut doc = toml_doc(text)?;
    let mut table = doc.as_table_mut();
    for name in section {
        match table.get_mut(name).and_then(Item::as_table_mut) {
            Some(next) => table = next,
            None => return Ok((text.to_string(), false)),
        }
    }
    let removed = table.remove(key).is_some();
    Ok((if removed { doc.to_string() } else { text.to_string() }, removed))
}

// ----------------------------------------------------------------------- disk

/// The sibling path used to keep the original file before the first change.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".wsid-backup");
    path.with_file_name(name)
}

/// Write `text` to `path` through a temp file in the same directory, then rename.
/// An existing file is copied to `<file>.wsid-backup` first, unless a backup is already there.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| format!("could not create {}: {err}", parent.display()))?;

    if path.exists() {
        let backup = backup_path(path);
        if !backup.exists() {
            fs::copy(path, &backup).map_err(|err| format!("could not back up {}: {err}", path.display()))?;
        }
    }

    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    tmp_name.push(format!(".wsid-tmp-{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    fs::write(&tmp, text).map_err(|err| format!("could not write {}: {err}", tmp.display()))?;
    // Keep the permissions of the file being replaced.
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        format!("could not replace {}: {err}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_set_creates_parents_in_an_empty_file() {
        let out = set(Format::Json, "", &["mcpServers"], "wsid", &json!({"command": "/bin/wsid", "args": ["mcp"]})).unwrap();
        let got = get(Format::Json, &out, &["mcpServers"], "wsid")
            .unwrap()
            .unwrap();
        assert_eq!(got["command"], "/bin/wsid");
        assert_eq!(got["args"], json!(["mcp"]));
    }

    #[test]
    fn jsonc_edits_keep_comments_and_other_servers() {
        let text =
            "{\n  // my servers\n  \"mcpServers\": {\n    \"other\": { \"command\": \"x\" }, // keep me\n  },\n  \"theme\": \"dark\",\n}\n";
        let out = set(Format::Json, text, &["mcpServers"], "wsid", &json!({"command": "w"})).unwrap();
        assert!(out.contains("// my servers"));
        assert!(out.contains("// keep me"));
        assert!(out.contains("\"theme\": \"dark\""));
        assert_eq!(
            get(Format::Json, &out, &["mcpServers"], "other")
                .unwrap()
                .unwrap()["command"],
            "x"
        );
        assert_eq!(
            get(Format::Json, &out, &["mcpServers"], "wsid")
                .unwrap()
                .unwrap()["command"],
            "w"
        );
    }

    #[test]
    fn json_set_replaces_an_existing_entry_and_remove_deletes_only_it() {
        let text = r#"{"mcpServers":{"wsid":{"command":"old"},"keep":{"command":"k"}}}"#;
        let out = set(Format::Json, text, &["mcpServers"], "wsid", &json!({"command": "new"})).unwrap();
        assert_eq!(
            get(Format::Json, &out, &["mcpServers"], "wsid")
                .unwrap()
                .unwrap()["command"],
            "new"
        );
        let (out, removed) = remove(Format::Json, &out, &["mcpServers"], "wsid").unwrap();
        assert!(removed);
        assert!(
            get(Format::Json, &out, &["mcpServers"], "wsid")
                .unwrap()
                .is_none()
        );
        assert!(
            get(Format::Json, &out, &["mcpServers"], "keep")
                .unwrap()
                .is_some()
        );
        assert!(
            !remove(Format::Json, &out, &["mcpServers"], "wsid")
                .unwrap()
                .1
        );
    }

    #[test]
    fn json_refuses_files_it_cannot_parse_or_shapes_it_would_break() {
        assert!(set(Format::Json, "{ not json", &["mcpServers"], "wsid", &json!({})).is_err());
        assert!(set(Format::Json, "[1, 2]", &["mcpServers"], "wsid", &json!({})).is_err());
        assert!(set(Format::Json, r#"{"mcpServers": []}"#, &["mcpServers"], "wsid", &json!({})).is_err());
    }

    #[test]
    fn toml_edits_keep_comments_and_other_tables() {
        let text = "# my codex config\nmodel = \"x\"\n\n[mcp_servers.other]\ncommand = \"o\" # keep\n";
        let out = set(Format::Toml, text, &["mcp_servers"], "wsid", &json!({"command": "/bin/wsid", "args": ["mcp"]})).unwrap();
        assert!(out.contains("# my codex config"));
        assert!(out.contains("# keep"));
        assert!(out.contains("[mcp_servers.wsid]"));
        let got = get(Format::Toml, &out, &["mcp_servers"], "wsid")
            .unwrap()
            .unwrap();
        assert_eq!(got, json!({"command": "/bin/wsid", "args": ["mcp"]}));
        assert_eq!(
            get(Format::Toml, &out, &["mcp_servers"], "other")
                .unwrap()
                .unwrap()["command"],
            "o"
        );
    }

    #[test]
    fn toml_creates_a_missing_file_and_removes_an_entry() {
        let out = set(Format::Toml, "", &["mcp_servers"], "wsid", &json!({"command": "w", "args": ["mcp"]})).unwrap();
        assert!(out.contains("[mcp_servers.wsid]"));
        assert!(!out.contains("[mcp_servers]\n"));
        let (out, removed) = remove(Format::Toml, &out, &["mcp_servers"], "wsid").unwrap();
        assert!(removed);
        assert!(
            get(Format::Toml, &out, &["mcp_servers"], "wsid")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn write_atomic_backs_up_once_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cfg.json");
        write_atomic(&path, "first").unwrap();
        assert!(!backup_path(&path).exists());
        write_atomic(&path, "second").unwrap();
        write_atomic(&path, "third").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "third");
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), "first");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2);
    }
}
