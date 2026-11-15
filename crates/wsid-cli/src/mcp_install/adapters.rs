//! The table of supported AI clients. Paths and entry shapes live here, so a client change is one edit.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::edit::Format;

/// Where the running machine keeps its home and config directories.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// `dirs::config_dir()`: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows, `~/.config` on Linux.
    pub config: PathBuf,
}

impl Env {
    pub fn from_system() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or_else(|| "could not find your home directory".to_string())?;
        let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        Ok(Self { home, config })
    }
}

/// How WSID gets into a client.
pub enum Method {
    /// Edit the client's config file.
    File,
    /// Ask the client's own CLI to do it. The file is only read, to report status.
    Command { program: &'static str, add: fn(&str, &str) -> Vec<String>, remove: fn(&str) -> Vec<String> },
}

pub struct Client {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub label: &'static str,
    pub method: Method,
    /// Candidate config files. WSID is written to every one that exists or whose folder exists.
    pub paths: fn(&Env) -> Vec<PathBuf>,
    pub format: Format,
    /// Object path to the table of servers inside the file.
    pub section: &'static [&'static str],
    /// The entry for a server: `(binary, args)`.
    pub entry: fn(&str, &[&str]) -> Value,
}

/// Arguments WSID's server is launched with.
pub const SERVER_ARGS: &[&str] = &["mcp"];

fn plain(command: &str, args: &[&str]) -> Value {
    json!({ "command": command, "args": args })
}

fn typed(command: &str, args: &[&str]) -> Value {
    json!({ "type": "stdio", "command": command, "args": args })
}

fn existing_or_first(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    match candidates.iter().find(|p| p.exists()) {
        Some(found) => vec![found.clone()],
        None => candidates.into_iter().take(1).collect(),
    }
}

fn claude_desktop(env: &Env) -> Vec<PathBuf> {
    vec![env.config.join("Claude").join("claude_desktop_config.json")]
}

fn claude_code(env: &Env) -> Vec<PathBuf> {
    vec![env.home.join(".claude.json")]
}

fn cursor(env: &Env) -> Vec<PathBuf> {
    vec![env.home.join(".cursor").join("mcp.json")]
}

fn vscode(env: &Env) -> Vec<PathBuf> {
    vec![env.config.join("Code").join("User").join("mcp.json")]
}

fn devin(env: &Env) -> Vec<PathBuf> {
    let current = env.config.join("devin").join("mcp_config.json");
    let legacy = env
        .home
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json");
    if !current.exists() && legacy.exists() { vec![legacy] } else { vec![current] }
}

fn zed(env: &Env) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![env.config.join("Zed").join("settings.json")]
    } else {
        vec![env.home.join(".config").join("zed").join("settings.json")]
    }
}

fn opencode(env: &Env) -> Vec<PathBuf> {
    let dir = env.home.join(".config").join("opencode");
    existing_or_first(vec![dir.join("opencode.json"), dir.join("opencode.jsonc")])
}

fn cline(env: &Env) -> Vec<PathBuf> {
    vec![
        env.home
            .join(".cline")
            .join("data")
            .join("settings")
            .join("cline_mcp_settings.json"),
        env.config
            .join("Code")
            .join("User")
            .join("globalStorage")
            .join("saoudrizwan.claude-dev")
            .join("settings")
            .join("cline_mcp_settings.json"),
    ]
}

fn codex(env: &Env) -> Vec<PathBuf> {
    vec![env.home.join(".codex").join("config.toml")]
}

fn gemini(env: &Env) -> Vec<PathBuf> {
    vec![env.home.join(".gemini").join("settings.json")]
}

fn claude_code_add(bin: &str, key: &str) -> Vec<String> {
    let mut args: Vec<String> = ["mcp", "add", "--transport", "stdio", "--scope", "user", key, "--", bin]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    args.extend(SERVER_ARGS.iter().map(|s| (*s).to_string()));
    args
}

fn claude_code_remove(key: &str) -> Vec<String> {
    ["mcp", "remove", "--scope", "user", key]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

fn vscode_add(bin: &str, key: &str) -> Vec<String> {
    let mut entry = typed(bin, SERVER_ARGS);
    entry["name"] = json!(key);
    vec!["--add-mcp".to_string(), entry.to_string()]
}

fn vscode_remove(_key: &str) -> Vec<String> {
    Vec::new()
}

pub fn clients() -> Vec<Client> {
    vec![
        Client {
            id: "claude-desktop",
            aliases: &["claude"],
            label: "Claude Desktop",
            method: Method::File,
            paths: claude_desktop,
            format: Format::Json,
            section: &["mcpServers"],
            entry: plain,
        },
        Client {
            id: "claude-code",
            aliases: &[],
            label: "Claude Code",
            method: Method::Command { program: "claude", add: claude_code_add, remove: claude_code_remove },
            paths: claude_code,
            format: Format::Json,
            section: &["mcpServers"],
            entry: plain,
        },
        Client {
            id: "cursor",
            aliases: &[],
            label: "Cursor",
            method: Method::File,
            paths: cursor,
            format: Format::Json,
            section: &["mcpServers"],
            entry: typed,
        },
        Client {
            id: "vscode",
            aliases: &["code"],
            label: "VS Code",
            method: Method::Command { program: "code", add: vscode_add, remove: vscode_remove },
            paths: vscode,
            format: Format::Json,
            section: &["servers"],
            entry: typed,
        },
        Client {
            id: "devin",
            aliases: &["windsurf"],
            label: "Devin for Terminal / Windsurf",
            method: Method::File,
            paths: devin,
            format: Format::Json,
            section: &["mcpServers"],
            entry: plain,
        },
        Client {
            id: "zed",
            aliases: &[],
            label: "Zed",
            method: Method::File,
            paths: zed,
            format: Format::Json,
            section: &["context_servers"],
            entry: |bin, args| json!({ "command": bin, "args": args, "env": {} }),
        },
        Client {
            id: "opencode",
            aliases: &[],
            label: "OpenCode",
            method: Method::File,
            paths: opencode,
            format: Format::Json,
            section: &["mcp"],
            entry: |bin, args| {
                let mut command = vec![bin.to_string()];
                command.extend(args.iter().map(|s| (*s).to_string()));
                json!({ "type": "local", "command": command, "enabled": true })
            },
        },
        Client {
            id: "cline",
            aliases: &[],
            label: "Cline",
            method: Method::File,
            paths: cline,
            format: Format::Json,
            section: &["mcpServers"],
            entry: |bin, args| json!({ "command": bin, "args": args, "disabled": false }),
        },
        Client {
            id: "codex",
            aliases: &[],
            label: "Codex CLI",
            method: Method::File,
            paths: codex,
            format: Format::Toml,
            section: &["mcp_servers"],
            entry: plain,
        },
        Client {
            id: "gemini",
            aliases: &[],
            label: "Gemini CLI",
            method: Method::File,
            paths: gemini,
            format: Format::Json,
            section: &["mcpServers"],
            entry: plain,
        },
    ]
}

/// Find a client by ID or alias.
pub fn find<'a>(all: &'a [Client], name: &str) -> Option<&'a Client> {
    let name = name.trim().to_ascii_lowercase();
    all.iter()
        .find(|c| c.id == name || c.aliases.contains(&name.as_str()))
}

/// The command and arguments an existing entry launches, whatever shape the client uses.
pub fn launch_line(entry: &Value) -> Vec<String> {
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    match entry.get("command") {
        Some(Value::String(command)) => {
            let mut line = vec![command.clone()];
            line.extend(entry.get("args").map(strings).unwrap_or_default());
            line
        }
        Some(array @ Value::Array(_)) => strings(array),
        _ => Vec::new(),
    }
}

/// True when the config folder or file exists, which is how file clients are detected.
pub fn path_is_present(path: &Path) -> bool {
    path.exists() || path.parent().is_some_and(Path::exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_aliases_are_unique_and_resolve() {
        let all = clients();
        let mut seen = std::collections::HashSet::new();
        for c in &all {
            assert!(seen.insert(c.id), "duplicate id {}", c.id);
            for alias in c.aliases {
                assert!(seen.insert(alias), "duplicate alias {alias}");
            }
        }
        assert_eq!(find(&all, "Windsurf").unwrap().id, "devin");
        assert!(find(&all, "nope").is_none());
        assert_eq!(all.len(), 10);
    }

    #[test]
    fn launch_line_reads_string_and_array_commands() {
        assert_eq!(launch_line(&json!({"command": "/w", "args": ["mcp"]})), ["/w", "mcp"]);
        assert_eq!(launch_line(&json!({"type": "local", "command": ["/w", "mcp"]})), ["/w", "mcp"]);
        assert!(launch_line(&json!({"url": "https://x"})).is_empty());
    }

    #[test]
    fn devin_falls_back_to_the_legacy_windsurf_path_only_when_it_alone_exists() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env { home: dir.path().join("home"), config: dir.path().join("cfg") };
        let legacy = env.home.join(".codeium/windsurf/mcp_config.json");
        assert_eq!(devin(&env), vec![env.config.join("devin/mcp_config.json")]);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{}").unwrap();
        assert_eq!(devin(&env), vec![legacy]);
    }
}
