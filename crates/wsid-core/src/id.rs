use std::fmt;
use std::str::FromStr;

use rand::Rng;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::{Result, WsidError};

/// 24 unambiguous uppercase ASCII letters (`I` and `O` excluded).
pub const SHORT_ID_LETTERS: &[u8; 24] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const MAX_GENERATION_ATTEMPTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShortId(String);

impl ShortId {
    pub fn parse(raw: &str) -> Result<Self> {
        let normalized = normalize_task_id(raw)?;
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ShortId {
    type Err = WsidError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Validates and normalizes a task ID token (such as `"ab1234"` or `"#ab1234"`) into `"AB1234"`.
pub fn normalize_task_id(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if stripped.len() != 6 || !stripped.is_ascii() {
        return Err(WsidError::InvalidTaskId(raw.to_string()));
    }

    let upper = stripped.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let valid_letters = SHORT_ID_LETTERS.contains(&bytes[0]) && SHORT_ID_LETTERS.contains(&bytes[1]);
    let valid_digits = bytes[2..6].iter().all(|b| b.is_ascii_digit());

    if !valid_letters || !valid_digits {
        return Err(WsidError::InvalidTaskId(raw.to_string()));
    }

    Ok(upper)
}

pub fn is_valid_task_id(raw: &str) -> bool {
    normalize_task_id(raw).is_ok()
}

pub struct ShortIdGenerator;

impl ShortIdGenerator {
    /// Generates a random short ID matching `^[A-HJ-NP-Z]{2}[0-9]{4}$`.
    pub fn random_candidate<R: Rng + ?Sized>(rng: &mut R) -> String {
        let first = SHORT_ID_LETTERS[rng.random_range(0..SHORT_ID_LETTERS.len())] as char;
        let second = SHORT_ID_LETTERS[rng.random_range(0..SHORT_ID_LETTERS.len())] as char;
        let digits: u16 = rng.random_range(0..10_000);
        format!("{first}{second}{digits:04}")
    }

    /// Generates a unique short ID verified against the `tasks` table, retrying on collisions.
    pub fn generate_unique(db: &Database) -> Result<String> {
        Self::generate_unique_conn(&db.conn)
    }

    pub fn generate_unique_with<F>(db: &Database, next_candidate: F) -> Result<String>
    where
        F: FnMut() -> String,
    {
        Self::generate_unique_conn_with(&db.conn, next_candidate)
    }

    pub(crate) fn generate_unique_conn(conn: &Connection) -> Result<String> {
        let mut rng = rand::rng();
        Self::generate_unique_conn_with(conn, || Self::random_candidate(&mut rng))
    }

    pub(crate) fn generate_unique_conn_with<F>(conn: &Connection, mut next_candidate: F) -> Result<String>
    where
        F: FnMut() -> String,
    {
        let mut stmt = conn.prepare_cached("SELECT 1 FROM tasks WHERE id = ?1")?;
        for _ in 0..MAX_GENERATION_ATTEMPTS {
            let candidate = next_candidate();
            let exists: Option<i32> = stmt.query_row([&candidate], |row| row.get(0)).optional()?;
            if exists.is_none() {
                return Ok(candidate);
            }
        }
        Err(WsidError::IdCollisionExhausted(MAX_GENERATION_ATTEMPTS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;

    #[test]
    fn normalizes_valid_ids_and_rejects_ambiguous_or_malformed_ids() {
        assert_eq!(normalize_task_id("ab1234").unwrap(), "AB1234");
        assert_eq!(normalize_task_id("  #zp0042 ").unwrap(), "ZP0042");

        // Ambiguous letters I and O must be rejected
        assert!(normalize_task_id("IA1234").is_err());
        assert!(normalize_task_id("AO1234").is_err());
        assert!(normalize_task_id("io9999").is_err());

        // Malformed lengths or positions
        assert!(normalize_task_id("ABC123").is_err());
        assert!(normalize_task_id("A1234").is_err());
        assert!(normalize_task_id("AB12345").is_err());
        assert!(normalize_task_id("").is_err());
    }

    #[test]
    fn retries_on_collision_until_unique_id_found() {
        let db = open_in_memory_db().unwrap();

        db.conn
            .execute("INSERT INTO tasks (id, title, reporter, assignee, evaluator) VALUES ('AB1234', 'Existing', 'me', 'me', 'me')", [])
            .unwrap();

        let mut candidates = ["AB1234", "AB1234", "CD5678"].into_iter();
        let generated = ShortIdGenerator::generate_unique_with(&db, || candidates.next().unwrap().to_string()).unwrap();
        assert_eq!(generated, "CD5678");
        assert!(is_valid_task_id(&generated));
    }

    #[test]
    fn returns_error_when_all_collision_attempts_exhausted() {
        let db = open_in_memory_db().unwrap();

        db.conn
            .execute("INSERT INTO tasks (id, title, reporter, assignee, evaluator) VALUES ('AB1234', 'Existing', 'me', 'me', 'me')", [])
            .unwrap();

        let err = ShortIdGenerator::generate_unique_with(&db, || "AB1234".to_string()).unwrap_err();
        assert!(matches!(err, WsidError::IdCollisionExhausted(MAX_GENERATION_ATTEMPTS)));
    }
}
