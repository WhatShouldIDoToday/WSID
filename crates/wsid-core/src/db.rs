use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::Result;

const INITIAL_MIGRATION_SQL: &str = include_str!("migrations/0001_initial.sql");
const MIGRATION_0002_SQL: &str = include_str!("migrations/0002_manual_priority_1_to_5.sql");
const MIGRATION_0003_SQL: &str = include_str!("migrations/0003_users_table.sql");
const SCHEMA_VERSION_1: i64 = 1;
const SCHEMA_VERSION_2: i64 = 2;
const SCHEMA_VERSION_3: i64 = 3;

/// Opaque handle to a WSID task database.
///
/// Wraps the underlying storage connection so downstream crates (`wsid-mcp`, `wsid-cli`, `wsid-gui`)
/// never depend on `rusqlite` directly and can be swapped or extended with alternative backends in the future.
pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    /// Opens a SQLite database at `path` with WAL concurrency pragmas and runs pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        open_db(path)
    }

    /// Opens an in-memory SQLite database with pragmas and schema migrations applied.
    pub fn open_in_memory() -> Result<Self> {
        open_in_memory_db()
    }
}

/// Opens a SQLite database at `path` with WAL concurrency pragmas and runs pending migrations.
pub fn open_db(path: impl AsRef<Path>) -> Result<Database> {
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    configure_and_migrate(conn)
}

/// Opens an in-memory SQLite database with pragmas and schema migrations applied.
pub fn open_in_memory_db() -> Result<Database> {
    let conn = Connection::open_in_memory()?;
    configure_and_migrate(conn)
}

fn configure_and_migrate(conn: Connection) -> Result<Database> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    let mut db = Database { conn };
    run_migrations(&mut db)?;
    Ok(db)
}

pub fn run_migrations(db: &mut Database) -> Result<()> {
    db.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );",
    )?;

    let current_version: i64 = db
        .conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |row| row.get(0))?;

    if current_version < SCHEMA_VERSION_1 {
        let tx = db.conn.transaction()?;
        tx.execute_batch(INITIAL_MIGRATION_SQL)?;
        tx.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [SCHEMA_VERSION_1])?;
        tx.commit()?;
    }

    if current_version < SCHEMA_VERSION_2 {
        let tx = db.conn.transaction()?;
        tx.execute_batch(MIGRATION_0002_SQL)?;
        tx.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [SCHEMA_VERSION_2])?;
        tx.commit()?;
    }

    if current_version < SCHEMA_VERSION_3 {
        let tx = db.conn.transaction()?;
        tx.execute_batch(MIGRATION_0003_SQL)?;
        tx.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [SCHEMA_VERSION_3])?;
        tx.commit()?;
    }

    Ok(())
}

/// Tracks `PRAGMA data_version` on a [`Database`] connection to detect writes committed by other
/// connections or processes (e.g., MCP server, CLI, or GUI) in under a microsecond.
#[derive(Debug, Clone)]
pub struct DbChangeDetector {
    last_data_version: i64,
}

impl DbChangeDetector {
    pub fn new(db: &Database) -> Result<Self> {
        let version = Self::read_data_version(db)?;
        Ok(Self { last_data_version: version })
    }

    pub fn read_data_version(db: &Database) -> Result<i64> {
        let version: i64 = db
            .conn
            .pragma_query_value(None, "data_version", |row| row.get(0))?;
        Ok(version)
    }

    /// Returns `true` if another connection or process has modified the database since the last
    /// check, updating the tracked version.
    pub fn has_external_changes(&mut self, db: &Database) -> Result<bool> {
        let current = Self::read_data_version(db)?;
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

        let db = open_db(&db_path).unwrap();

        let journal_mode: String = db
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let fk_enabled: i64 = db
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk_enabled, 1);

        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn fts_triggers_stay_in_sync_on_insert_update_delete_and_vacuum() {
        let db = open_in_memory_db().unwrap();

        db.conn
            .execute(
                "INSERT INTO tasks (id, title, description, reporter, assignee, evaluator)
                 VALUES ('AB1234', 'Implement accessibility audit', 'Support screen readers', 'alice', 'bob', 'alice')",
                [],
            )
            .unwrap();

        let match_count = |substr: &str| -> i64 {
            db.conn
                .query_row("SELECT COUNT(*) FROM tasks_fts WHERE tasks_fts MATCH ?1", [substr], |r| r.get(0))
                .unwrap()
        };

        assert_eq!(match_count("access"), 1);
        assert_eq!(match_count("AB1234"), 1);

        db.conn
            .execute("UPDATE tasks SET title = 'Build Eisenhower grid', description = 'Quadrant layout' WHERE id = 'AB1234'", [])
            .unwrap();

        assert_eq!(match_count("access"), 0);
        assert_eq!(match_count("Eisenhower"), 1);

        // VACUUM preserves explicit integer primary key rowid and keeps external-content FTS5 intact.
        db.conn.execute_batch("VACUUM;").unwrap();
        assert_eq!(match_count("Eisenhower"), 1);

        db.conn
            .execute("DELETE FROM tasks WHERE id = 'AB1234'", [])
            .unwrap();
        assert_eq!(match_count("Eisenhower"), 0);
    }

    #[test]
    fn data_version_detects_cross_connection_writes() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("concurrent.db");

        let db_ui = open_db(&db_path).unwrap();
        let db_mcp = open_db(&db_path).unwrap();

        let mut detector = DbChangeDetector::new(&db_ui).unwrap();
        assert!(!detector.has_external_changes(&db_ui).unwrap());

        db_mcp
            .conn
            .execute(
                "INSERT INTO tasks (id, title, reporter, assignee, evaluator)
                 VALUES ('CD5678', 'External MCP write', 'claude', 'human', 'human')",
                [],
            )
            .unwrap();

        assert!(detector.has_external_changes(&db_ui).unwrap());
        assert!(!detector.has_external_changes(&db_ui).unwrap());
    }
}
