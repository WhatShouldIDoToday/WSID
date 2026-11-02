use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::Result;

const INITIAL_MIGRATION_SQL: &str = include_str!("migrations/0001_initial.sql");
const SCHEMA_VERSION_1: i64 = 1;

/// Opens a SQLite database at `path` with WAL concurrency pragmas and runs pending migrations.
pub fn open_db(path: impl AsRef<Path>) -> Result<Connection> {
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    configure_and_migrate(conn)
}

/// Opens an in-memory SQLite database with pragmas and schema migrations applied.
pub fn open_in_memory_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure_and_migrate(conn)
}

fn configure_and_migrate(mut conn: Connection) -> Result<Connection> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    run_migrations(&mut conn)?;
    Ok(conn)
}

pub fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );",
    )?;

    let current_version: i64 = conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |row| row.get(0))?;

    if current_version < SCHEMA_VERSION_1 {
        let tx = conn.transaction()?;
        tx.execute_batch(INITIAL_MIGRATION_SQL)?;
        tx.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [SCHEMA_VERSION_1])?;
        tx.commit()?;
    }

    Ok(())
}

/// Tracks `PRAGMA data_version` on a SQLite connection to detect writes committed by other
/// connections or processes (e.g., MCP server, CLI, or GUI) in under a microsecond.
#[derive(Debug, Clone)]
pub struct DbChangeDetector {
    last_data_version: i64,
}

impl DbChangeDetector {
    pub fn new(conn: &Connection) -> Result<Self> {
        let version = Self::read_data_version(conn)?;
        Ok(Self { last_data_version: version })
    }

    pub fn read_data_version(conn: &Connection) -> Result<i64> {
        let version: i64 = conn.pragma_query_value(None, "data_version", |row| row.get(0))?;
        Ok(version)
    }

    /// Returns `true` if another connection or process has modified the database since the last
    /// check, updating the tracked version.
    pub fn has_external_changes(&mut self, conn: &Connection) -> Result<bool> {
        let current = Self::read_data_version(conn)?;
        if current != self.last_data_version {
            self.last_data_version = current;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn last_version(&self) -> i64 {
        self.last_data_version
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn initializes_wal_and_creates_schema() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("wsid_test.db");

        let conn = open_db(&db_path).unwrap();

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let fk_enabled: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk_enabled, 1);

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn fts_triggers_stay_in_sync_on_insert_update_delete_and_vacuum() {
        let conn = open_in_memory_db().unwrap();

        conn.execute(
            "INSERT INTO tasks (id, title, description, reporter, assignee, evaluator)
             VALUES ('AB1234', 'Implement accessibility audit', 'Support screen readers', 'alice', 'bob', 'alice')",
            [],
        )
        .unwrap();

        let match_count = |substr: &str| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM tasks_fts WHERE tasks_fts MATCH ?1", [substr], |r| r.get(0))
                .unwrap()
        };

        assert_eq!(match_count("access"), 1);
        assert_eq!(match_count("AB1234"), 1);

        conn.execute("UPDATE tasks SET title = 'Build Eisenhower grid', description = 'Quadrant layout' WHERE id = 'AB1234'", [])
            .unwrap();

        assert_eq!(match_count("access"), 0);
        assert_eq!(match_count("Eisenhower"), 1);

        // VACUUM preserves explicit integer primary key rowid and keeps external-content FTS5 intact.
        conn.execute_batch("VACUUM;").unwrap();
        assert_eq!(match_count("Eisenhower"), 1);

        conn.execute("DELETE FROM tasks WHERE id = 'AB1234'", [])
            .unwrap();
        assert_eq!(match_count("Eisenhower"), 0);
    }

    #[test]
    fn data_version_detects_cross_connection_writes() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("concurrent.db");

        let conn_ui = open_db(&db_path).unwrap();
        let conn_mcp = open_db(&db_path).unwrap();

        let mut detector = DbChangeDetector::new(&conn_ui).unwrap();
        assert!(!detector.has_external_changes(&conn_ui).unwrap());

        conn_mcp
            .execute(
                "INSERT INTO tasks (id, title, reporter, assignee, evaluator)
                 VALUES ('CD5678', 'External MCP write', 'claude', 'human', 'human')",
                [],
            )
            .unwrap();

        assert!(detector.has_external_changes(&conn_ui).unwrap());
        assert!(!detector.has_external_changes(&conn_ui).unwrap());
    }
}
