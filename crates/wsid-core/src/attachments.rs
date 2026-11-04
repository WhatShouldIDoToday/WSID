use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use rand::Rng;
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::default_attachments_dir;
use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::normalize_task_id;
use crate::models::TaskAttachment;
use crate::relations::ensure_task_exists;
use crate::tasks::{format_utc, parse_utc};

/// Infers a MIME media type from a file extension.
pub fn infer_media_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());

    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("tif" | "tiff") => "image/tiff",
        Some("avif") => "image/avif",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "markdown" | "log" | "csv") => "text/plain",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        _ => "application/octet-stream",
    }
}

/// Attaches an existing file on disk (from drag-and-drop, file picker, CLI flag, or MCP tool)
/// to a task description (`note_id = None`) or a specific comment (`note_id = Some(id)`).
///
/// If `description` is `None` or blank, `description` defaults to the file's basename so screen
/// readers always have an accessible `alt` label out of the box.
pub fn attach_existing_file(
    db: &mut Database,
    raw_task_id: &str,
    note_id: Option<i64>,
    file_path: impl AsRef<Path>,
    description: Option<&str>,
) -> Result<TaskAttachment> {
    attach_existing_file_with_dir(db, raw_task_id, note_id, file_path, description, None)
}

pub fn attach_existing_file_with_dir(
    db: &mut Database,
    raw_task_id: &str,
    note_id: Option<i64>,
    file_path: impl AsRef<Path>,
    description: Option<&str>,
    managed_dir_override: Option<&Path>,
) -> Result<TaskAttachment> {
    let tx = db.conn.transaction()?;
    let id = attach_existing_file_in_tx(&tx, raw_task_id, note_id, file_path.as_ref(), description, managed_dir_override)?;
    tx.commit()?;
    get_attachment(db, id)
}

pub(crate) fn attach_existing_file_in_tx(
    conn: &Connection,
    raw_task_id: &str,
    note_id: Option<i64>,
    file_path: &Path,
    description: Option<&str>,
    managed_dir_override: Option<&Path>,
) -> Result<i64> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(conn, &task_id)?;

    if let Some(nid) = note_id {
        ensure_note_belongs_to_task(conn, nid, &task_id)?;
    }

    let raw_str = file_path.to_string_lossy();
    if raw_str.trim().is_empty() {
        return Err(WsidError::Validation("attachment file_path cannot be empty".to_string()));
    }

    let resolved_path = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    let file_name = resolved_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("attachment")
        .to_string();

    let media_type = infer_media_type(&resolved_path);
    let effective_desc = match description.map(str::trim) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => file_name.clone(),
    };

    let managed_root = managed_dir_override
        .map(Path::to_path_buf)
        .unwrap_or_else(default_attachments_dir);
    let is_managed = is_path_inside_managed_dir(&resolved_path, &managed_root);

    let now_str = format_utc(Utc::now());
    let stored_path = resolved_path.to_string_lossy().to_string();

    conn.execute(
        "INSERT INTO task_attachments (
            task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![task_id, note_id, stored_path, file_name, media_type, effective_desc, i64::from(is_managed), now_str,],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Saves raw bytes (such as a pasted clipboard image) into the OS app-data attachments folder
/// (`~/Library/Application Support/wsid/attachments/`, `%LOCALAPPDATA%\wsid\attachments\`, or
/// `$XDG_DATA_HOME/wsid/attachments/`) and links it as a managed [`TaskAttachment`].
///
/// Managed attachments are automatically deleted from disk when their owning task, note, or
/// attachment record is deleted.
pub fn save_clipboard_attachment(
    db: &mut Database,
    raw_task_id: &str,
    note_id: Option<i64>,
    bytes: &[u8],
    suggested_name_or_ext: Option<&str>,
    description: Option<&str>,
) -> Result<TaskAttachment> {
    save_clipboard_attachment_with_dir(db, raw_task_id, note_id, bytes, suggested_name_or_ext, description, None)
}

pub fn save_clipboard_attachment_with_dir(
    db: &mut Database,
    raw_task_id: &str,
    note_id: Option<i64>,
    bytes: &[u8],
    suggested_name_or_ext: Option<&str>,
    description: Option<&str>,
    managed_dir_override: Option<&Path>,
) -> Result<TaskAttachment> {
    if bytes.is_empty() {
        return Err(WsidError::Validation("clipboard attachment payload cannot be empty".to_string()));
    }

    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;
    if let Some(nid) = note_id {
        ensure_note_belongs_to_task(&db.conn, nid, &task_id)?;
    }

    let managed_root = managed_dir_override
        .map(Path::to_path_buf)
        .unwrap_or_else(default_attachments_dir);
    fs::create_dir_all(&managed_root)?;

    let ext = sanitize_extension(suggested_name_or_ext, bytes);
    let now = Utc::now();
    let stamp = now.format("%Y%m%d-%H%M%S");
    let suffix: u16 = rand::rng().random_range(1000..10_000);
    let file_name = format!("clipboard-{task_id}-{stamp}-{suffix}.{ext}");
    let dest_path = managed_root.join(&file_name);

    fs::write(&dest_path, bytes)?;
    let canonical_dest = fs::canonicalize(&dest_path).unwrap_or(dest_path);

    let media_type = infer_media_type(&canonical_dest);
    let effective_desc = match description.map(str::trim) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => file_name.clone(),
    };
    let now_str = format_utc(now);
    let stored_path = canonical_dest.to_string_lossy().to_string();

    db.conn.execute(
        "INSERT INTO task_attachments (
            task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
        params![task_id, note_id, stored_path, file_name, media_type, effective_desc, now_str,],
    )?;
    let id = db.conn.last_insert_rowid();
    get_attachment(db, id)
}

/// Updates the `description` (screen-reader alt text) of an attachment.
/// If `description` is empty, resets it to the attachment's `file_name`.
pub fn update_attachment_description(db: &mut Database, attachment_id: i64, description: &str) -> Result<TaskAttachment> {
    let existing = get_attachment(db, attachment_id)?;
    let trimmed = description.trim();
    let effective_desc = if trimmed.is_empty() { existing.file_name.clone() } else { trimmed.to_string() };

    db.conn
        .execute("UPDATE task_attachments SET description = ?1 WHERE id = ?2", params![effective_desc, attachment_id])?;

    get_attachment(db, attachment_id)
}

/// Retrieves a single [`TaskAttachment`] by its numeric ID.
pub fn get_attachment(db: &Database, attachment_id: i64) -> Result<TaskAttachment> {
    get_attachment_conn(&db.conn, attachment_id)
}

pub(crate) fn get_attachment_conn(conn: &Connection, attachment_id: i64) -> Result<TaskAttachment> {
    let opt = conn
        .query_row(
            "SELECT id, task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
             FROM task_attachments
             WHERE id = ?1",
            [attachment_id],
            map_attachment_row,
        )
        .optional()?;

    opt.ok_or_else(|| WsidError::Validation(format!("attachment not found: {attachment_id}")))
}

/// Lists all attachments on `raw_task_id` (both task-description and comment attachments).
pub fn list_task_attachments(db: &Database, raw_task_id: &str) -> Result<Vec<TaskAttachment>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;
    load_all_attachments_for_task(&db.conn, &task_id)
}

/// Removes an attachment record and deletes its underlying file iff it is saved in WSID's managed app-data directory.
pub fn remove_attachment(db: &mut Database, attachment_id: i64) -> Result<bool> {
    remove_attachment_with_dir(db, attachment_id, None)
}

pub fn remove_attachment_with_dir(db: &mut Database, attachment_id: i64, managed_dir_override: Option<&Path>) -> Result<bool> {
    let existing = match get_attachment_conn(&db.conn, attachment_id) {
        Ok(att) => att,
        Err(_) => return Ok(false),
    };

    let affected = db
        .conn
        .execute("DELETE FROM task_attachments WHERE id = ?1", [attachment_id])?;
    if affected > 0 {
        cleanup_managed_files_after_delete(&db.conn, &[existing], managed_dir_override);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Opens a file on disk using the operating system's native default application
/// (`open` on macOS, `cmd /C start` on Windows, `xdg-open` on Linux).
pub fn open_file_in_os(path: impl AsRef<Path>) -> Result<()> {
    let path_ref = path.as_ref();
    if !path_ref.exists() {
        return Err(WsidError::Validation(format!("file does not exist on disk: '{}'", path_ref.display())));
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(path_ref).status()?;

    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path_ref)
        .status()?;

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let status = Command::new("xdg-open").arg(path_ref).status()?;

    if !status.success() {
        return Err(WsidError::Validation(format!(
            "native OS file opener exited with non-zero status ({status}) for '{}'",
            path_ref.display()
        )));
    }

    Ok(())
}

pub(crate) fn load_task_description_attachments(conn: &Connection, task_id: &str) -> Result<Vec<TaskAttachment>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
         FROM task_attachments
         WHERE task_id = ?1 AND note_id IS NULL
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([task_id], map_attachment_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn load_note_attachments(conn: &Connection, note_id: i64) -> Result<Vec<TaskAttachment>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
         FROM task_attachments
         WHERE note_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([note_id], map_attachment_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn load_all_attachments_for_task(conn: &Connection, task_id: &str) -> Result<Vec<TaskAttachment>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, task_id, note_id, file_path, file_name, media_type, description, is_managed, created_at
         FROM task_attachments
         WHERE task_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([task_id], map_attachment_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn cleanup_managed_files_after_delete(
    conn: &Connection,
    deleted_attachments: &[TaskAttachment],
    managed_dir_override: Option<&Path>,
) {
    let managed_root = managed_dir_override
        .map(Path::to_path_buf)
        .unwrap_or_else(default_attachments_dir);

    for att in deleted_attachments {
        if !att.is_managed {
            continue;
        }

        // Do not delete if another attachment record still references the same file path
        let still_referenced: bool = conn
            .query_row("SELECT 1 FROM task_attachments WHERE file_path = ?1 LIMIT 1", [&att.file_path], |_| Ok(true))
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false);
        if still_referenced {
            continue;
        }

        let candidate_path = PathBuf::from(&att.file_path);
        if is_path_inside_managed_dir(&candidate_path, &managed_root) {
            let _ = fs::remove_file(&candidate_path);
        }
    }
}

fn is_path_inside_managed_dir(file_path: &Path, managed_root: &Path) -> bool {
    let canon_root = fs::canonicalize(managed_root).unwrap_or_else(|_| managed_root.to_path_buf());
    let canon_file = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    canon_file.starts_with(&canon_root)
}

fn ensure_note_belongs_to_task(conn: &Connection, note_id: i64, task_id: &str) -> Result<()> {
    let note_task: Option<String> = conn
        .query_row("SELECT task_id FROM task_notes WHERE id = ?1", [note_id], |r| r.get(0))
        .optional()?;
    match note_task {
        Some(owner) if owner == task_id => Ok(()),
        Some(owner) => Err(WsidError::Validation(format!("note #{note_id} belongs to task {owner}, not {task_id}"))),
        None => Err(WsidError::Validation(format!("task note #{note_id} not found"))),
    }
}

fn sanitize_extension(suggested_name_or_ext: Option<&str>, bytes: &[u8]) -> String {
    if let Some(raw) = suggested_name_or_ext {
        let trimmed = raw.trim().trim_start_matches('.');
        let ext_candidate = Path::new(trimmed)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or(trimmed);
        let cleaned: String = ext_candidate
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if !cleaned.is_empty() && cleaned.len() <= 10 {
            return cleaned;
        }
    }

    // Magic-byte sniff for common clipboard image formats
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png".to_string()
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "jpg".to_string()
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif".to_string()
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp".to_string()
    } else {
        "png".to_string()
    }
}

fn map_attachment_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskAttachment> {
    let is_managed_i64: i64 = row.get(7)?;
    let created_str: String = row.get(8)?;
    Ok(TaskAttachment {
        id: row.get(0)?,
        task_id: row.get(1)?,
        note_id: row.get(2)?,
        file_path: row.get(3)?,
        file_name: row.get(4)?,
        media_type: row.get(5)?,
        description: row.get(6)?,
        is_managed: is_managed_i64 != 0,
        created_at: parse_utc(&created_str),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::db::open_in_memory_db;
    use crate::models::CreateTaskInput;
    use crate::notes::{add_task_note, delete_task_note_with_attachments_dir};
    use crate::tasks::{create_task, delete_task_with_attachments_dir, get_task};

    #[test]
    fn external_files_are_preserved_while_clipboard_managed_files_are_deleted_with_task_or_note() {
        let mut db = open_in_memory_db().unwrap();
        let user_downloads_dir = tempdir().unwrap();
        let app_attachments_dir = tempdir().unwrap();

        let external_img = user_downloads_dir.path().join("architecture-diagram.png");
        fs::write(&external_img, b"\x89PNG\r\n\x1a\nexternal").unwrap();

        let task = create_task(&mut db, CreateTaskInput { title: "Review UI mockups".to_string(), ..Default::default() }, "alice").unwrap();

        // 1. Attach external file without explicit description -> defaults description/alt_text to file_name, is_managed = false
        let ext_att =
            attach_existing_file_with_dir(&mut db, &task.id, None, &external_img, None, Some(app_attachments_dir.path())).unwrap();
        assert_eq!(ext_att.file_name, "architecture-diagram.png");
        assert_eq!(ext_att.description, "architecture-diagram.png");
        assert_eq!(ext_att.alt_text(), "architecture-diagram.png");
        assert_eq!(ext_att.media_type, "image/png");
        assert!(!ext_att.is_managed);

        // Update description for screen reader alt text
        let updated_ext = update_attachment_description(&mut db, ext_att.id, "High-contrast Kanban layout diagram").unwrap();
        assert_eq!(updated_ext.alt_text(), "High-contrast Kanban layout diagram");

        // 2. Save clipboard image on task description -> stored in app_attachments_dir, is_managed = true
        let task_clip = save_clipboard_attachment_with_dir(
            &mut db,
            &task.id,
            None,
            b"\x89PNG\r\n\x1a\nclipboard-task",
            Some("png"),
            Some("Pasted screenshot of focus ring"),
            Some(app_attachments_dir.path()),
        )
        .unwrap();
        assert!(task_clip.is_managed);
        assert_eq!(task_clip.alt_text(), "Pasted screenshot of focus ring");
        let task_clip_path = PathBuf::from(&task_clip.file_path);
        assert!(task_clip_path.exists());

        // Verify `get_task` includes task description attachments
        let fetched_task = get_task(&db, &task.id).unwrap();
        assert_eq!(fetched_task.attachments.len(), 2);

        // 3. Add a note and attach both an external file and a clipboard image to the note
        let note_outcome = add_task_note(&mut db, &task.id, "alice", "Here is the updated screenshot", true).unwrap();
        let note_id = note_outcome.note.id;

        let note_clip = save_clipboard_attachment_with_dir(
            &mut db,
            &task.id,
            Some(note_id),
            b"\x89PNG\r\n\x1a\nclipboard-note",
            None,
            None,
            Some(app_attachments_dir.path()),
        )
        .unwrap();
        let note_clip_path = PathBuf::from(&note_clip.file_path);
        assert!(note_clip_path.exists());
        // Default description is the generated filename
        assert_eq!(note_clip.alt_text(), note_clip.file_name);

        let note_ext =
            attach_existing_file_with_dir(&mut db, &task.id, Some(note_id), &external_img, None, Some(app_attachments_dir.path())).unwrap();
        assert!(!note_ext.is_managed);

        // 4. Delete the note -> `note_clip` (in app data dir) MUST be deleted from disk, while `external_img` MUST remain untouched!
        delete_task_note_with_attachments_dir(&mut db, note_id, Some(app_attachments_dir.path())).unwrap();
        assert!(!note_clip_path.exists(), "managed clipboard file for deleted note should be removed from disk");
        assert!(external_img.exists(), "external file in user folder must never be deleted");

        // 5. Delete the task -> `task_clip` (in app data dir) MUST be deleted from disk, while `external_img` MUST still remain untouched!
        delete_task_with_attachments_dir(&mut db, &task.id, Some(app_attachments_dir.path())).unwrap();
        assert!(!task_clip_path.exists(), "managed clipboard file for deleted task should be removed from disk");
        assert!(external_img.exists(), "external file in user folder must never be deleted");
    }
}
