use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, WsidError};
use crate::models::FocusPreset;
use crate::scoring::{DEFAULT_FOCUS_LIMIT, FocusWeights, ScoringConfig};
use crate::tasks::normalize_user;

pub const LOCAL_DB_FILENAME: &str = ".wsid.db";
pub const GLOBAL_DB_FILENAME: &str = "wsid.db";
pub const CONFIG_FILENAME: &str = "config.toml";
pub const ATTACHMENTS_DIRNAME: &str = "attachments";
pub const WSID_DB_ENV_VAR: &str = "WSID_DB";
/// Folder name for config and app data. Beta builds (`WSID_CHANNEL=beta`) use their own, so a beta
/// schema migration can never touch stable data.
pub const APP_DIR_NAME: &str = app_dir_name(option_env!("WSID_CHANNEL"));

const fn app_dir_name(channel: Option<&str>) -> &'static str {
    match channel {
        Some(value) if value.eq_ignore_ascii_case("beta") => "wsid-beta",
        _ => "wsid",
    }
}

pub const MIN_FOCUS_LIMIT: usize = 1;
pub const MAX_FOCUS_LIMIT: usize = 50;

pub fn validate_focus_limit(limit: usize) -> Result<()> {
    if (MIN_FOCUS_LIMIT..=MAX_FOCUS_LIMIT).contains(&limit) {
        Ok(())
    } else {
        Err(WsidError::Validation(format!("focus limit must be between {MIN_FOCUS_LIMIT} and {MAX_FOCUS_LIMIT}")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_os_user")]
    pub primary_user: String,
    #[serde(alias = "default_persona")]
    pub default_user: String,
    pub focus_preset: FocusPreset,
    pub custom_weights: Option<FocusWeights>,
    pub focus_limit: usize,
    pub scoring: ScoringConfig,
    pub recent_workspaces: Vec<PathBuf>,
}

pub fn default_os_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .and_then(|u| normalize_user(&u).ok())
        .unwrap_or_else(|| "human".to_string())
}

impl Default for AppConfig {
    fn default() -> Self {
        let default_user = default_os_user();

        Self {
            primary_user: default_user.clone(),
            default_user,
            focus_preset: FocusPreset::Balanced,
            custom_weights: None,
            focus_limit: DEFAULT_FOCUS_LIMIT,
            scoring: ScoringConfig::default(),
            recent_workspaces: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        toml::from_str(&raw).map_err(|e| WsidError::Validation(format!("failed to parse config TOML at '{}': {e}", path.display())))
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let serialized =
            toml::to_string_pretty(self).map_err(|e| WsidError::Validation(format!("failed to serialize config TOML: {e}")))?;
        fs::write(path, serialized)?;
        Ok(())
    }

    pub fn record_recent_workspace(&mut self, db_path: PathBuf) {
        self.recent_workspaces.retain(|p| p != &db_path);
        self.recent_workspaces.insert(0, db_path);
        self.recent_workspaces.truncate(10);
    }
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
        .join(CONFIG_FILENAME)
}

/// Returns the OS-native local application data directory for WSID:
/// - macOS: `~/Library/Application Support/wsid`
/// - Windows: `%LOCALAPPDATA%\wsid`
/// - Linux: `$XDG_DATA_HOME/wsid` (or `~/.local/share/wsid`)
///
/// Beta builds use `wsid-beta` in place of `wsid`.
pub fn default_app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
}

pub fn default_global_db_path() -> PathBuf {
    default_app_data_dir().join(GLOBAL_DB_FILENAME)
}

/// Returns the managed directory where clipboard-pasted attachments are saved:
/// `<default_app_data_dir>/attachments`.
pub fn default_attachments_dir() -> PathBuf {
    default_app_data_dir().join(ATTACHMENTS_DIRNAME)
}

/// Searches `start_dir` and its ancestor directories for an existing `.wsid.db`.
/// If none is found, returns `start_dir.join(".wsid.db")`.
pub fn resolve_local_workspace_db_path(start_dir: &Path) -> PathBuf {
    for ancestor in start_dir.ancestors() {
        let candidate = ancestor.join(LOCAL_DB_FILENAME);
        if candidate.exists() {
            return candidate;
        }
    }
    start_dir.join(LOCAL_DB_FILENAME)
}

/// Resolves the active database path according to precedence:
/// 1. Explicit `--db <path>` / GUI selection
/// 2. `--local` (`.wsid.db` in `current_dir` or nearest ancestor)
/// 3. `WSID_DB` environment variable (`env_db_path`)
/// 4. Global OS app data default (`wsid.db`)
pub fn resolve_db_path(
    explicit_db: Option<&Path>,
    use_local: bool,
    current_dir: &Path,
    env_db_path: Option<&str>,
    global_default: &Path,
) -> PathBuf {
    if let Some(explicit) = explicit_db {
        return explicit.to_path_buf();
    }

    if use_local {
        return resolve_local_workspace_db_path(current_dir);
    }

    if let Some(env_val) = env_db_path {
        let trimmed = env_val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    global_default.to_path_buf()
}

/// Convenience helper using the process environment (`std::env::current_dir()`, `WSID_DB`, and OS app data dir).
pub fn resolve_active_db_path(explicit_db: Option<&Path>, use_local: bool) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_val = std::env::var(WSID_DB_ENV_VAR).ok();
    let global_default = default_global_db_path();
    resolve_db_path(explicit_db, use_local, &cwd, env_val.as_deref(), &global_default)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn app_dir_name_separates_only_the_beta_channel() {
        assert_eq!(app_dir_name(None), "wsid");
        assert_eq!(app_dir_name(Some("")), "wsid");
        assert_eq!(app_dir_name(Some("stable")), "wsid");
        assert_eq!(app_dir_name(Some("beta")), "wsid-beta");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wsid_cfg_{label}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_db_path_precedence_and_ancestor_local_db() {
        let root = unique_temp_dir("precedence");
        let nested = root.join("crates").join("sub");
        fs::create_dir_all(&nested).unwrap();

        let root_local_db = root.join(LOCAL_DB_FILENAME);
        fs::write(&root_local_db, b"").unwrap();

        let explicit = PathBuf::from("/custom/explicit.db");
        let global = PathBuf::from("/global/wsid.db");

        // 1. Explicit wins over everything
        assert_eq!(resolve_db_path(Some(&explicit), true, &nested, Some("/env/wsid.db"), &global), explicit);

        // 2. `--local` finds `.wsid.db` in ancestor `root` and wins over env & global
        assert_eq!(resolve_db_path(None, true, &nested, Some("/env/wsid.db"), &global), root_local_db);

        // 3. `WSID_DB` env var wins over global when `--local` is false
        assert_eq!(resolve_db_path(None, false, &nested, Some("/env/wsid.db"), &global), PathBuf::from("/env/wsid.db"));

        // 4. Global default when nothing else is set
        assert_eq!(resolve_db_path(None, false, &nested, None, &global), global);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn serializes_and_deserializes_app_config_toml() {
        let temp_dir = unique_temp_dir("toml");
        let cfg_path = temp_dir.join("config.toml");

        let mut cfg = AppConfig {
            primary_user: "david".to_string(),
            default_user: "david".to_string(),
            focus_preset: FocusPreset::UnblockOthers,
            custom_weights: Some(FocusWeights::for_preset(FocusPreset::QuickWins)),
            focus_limit: 7,
            scoring: ScoringConfig::default(),
            recent_workspaces: Vec::new(),
        };
        cfg.record_recent_workspace(PathBuf::from("/repo/a/.wsid.db"));
        cfg.record_recent_workspace(PathBuf::from("/repo/b/.wsid.db"));
        cfg.record_recent_workspace(PathBuf::from("/repo/a/.wsid.db")); // deduplicated to front

        cfg.save_to_path(&cfg_path).unwrap();
        let loaded = AppConfig::load_from_path(&cfg_path).unwrap();

        assert_eq!(loaded, cfg);
        assert_eq!(loaded.recent_workspaces.len(), 2);
        assert_eq!(loaded.recent_workspaces[0], PathBuf::from("/repo/a/.wsid.db"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
