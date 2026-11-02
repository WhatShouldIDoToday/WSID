-- 0001_initial.sql: Initial WSID SQLite WAL schema

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS tasks (
    rowid INTEGER PRIMARY KEY,
    id TEXT UNIQUE NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo'
        CHECK (status IN ('backlog', 'todo', 'in_progress', 'in_review', 'stuck', 'completed')),
    origin_kind TEXT NOT NULL DEFAULT 'manual'
        CHECK (origin_kind IN ('manual', 'mention', 'takenote', 'review')),
    origin_note_id INTEGER,
    due_date TEXT,
    sgdb_date TEXT,
    time_estimate_minutes INTEGER
        CHECK (time_estimate_minutes IS NULL OR time_estimate_minutes >= 0),
    difficulty INTEGER NOT NULL DEFAULT 3
        CHECK (difficulty BETWEEN 1 AND 5),
    importance INTEGER NOT NULL DEFAULT 3
        CHECK (importance BETWEEN 1 AND 5),
    urgent_override INTEGER NOT NULL DEFAULT 0
        CHECK (urgent_override IN (0, 1)),
    manual_priority INTEGER NOT NULL DEFAULT 0
        CHECK (manual_priority BETWEEN -5 AND 5),
    reporter TEXT NOT NULL,
    assignee TEXT NOT NULL,
    evaluator TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at TEXT,
    CHECK (sgdb_date IS NULL OR due_date IS NULL OR sgdb_date <= due_date)
);

CREATE INDEX IF NOT EXISTS idx_tasks_assignee_status ON tasks(assignee, status);
CREATE INDEX IF NOT EXISTS idx_tasks_evaluator_status ON tasks(evaluator, status);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_due_date ON tasks(due_date);
CREATE INDEX IF NOT EXISTS idx_tasks_sgdb_date ON tasks(sgdb_date);

CREATE TABLE IF NOT EXISTS task_relations (
    source_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL
        CHECK (relation_type IN ('blocks', 'parent_of', 'parallel', 'related')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (source_id, target_id, relation_type),
    CHECK (source_id != target_id)
);

CREATE INDEX IF NOT EXISTS idx_task_relations_target ON task_relations(target_id, relation_type);
CREATE INDEX IF NOT EXISTS idx_task_relations_source ON task_relations(source_id, relation_type);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL
        CHECK (name NOT GLOB '*[^a-z0-9_-]*' AND length(name) > 0)
);

CREATE TABLE IF NOT EXISTS task_tags (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_task_tags_tag_id ON task_tags(tag_id);

CREATE TABLE IF NOT EXISTS task_notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_task_notes_task_id ON task_notes(task_id, created_at);

CREATE TABLE IF NOT EXISTS task_attachments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    note_id INTEGER REFERENCES task_notes(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    media_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    description TEXT NOT NULL DEFAULT '',
    is_managed INTEGER NOT NULL DEFAULT 0
        CHECK (is_managed IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_task_attachments_task_id ON task_attachments(task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_task_attachments_note_id ON task_attachments(note_id);

CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(
    id,
    title,
    description,
    content='tasks',
    content_rowid='rowid',
    tokenize='trigram case_sensitive 0'
);

CREATE TRIGGER IF NOT EXISTS tasks_ai AFTER INSERT ON tasks BEGIN
    INSERT INTO tasks_fts(rowid, id, title, description)
    VALUES (new.rowid, new.id, new.title, new.description);
END;

CREATE TRIGGER IF NOT EXISTS tasks_ad AFTER DELETE ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, id, title, description)
    VALUES ('delete', old.rowid, old.id, old.title, old.description);
END;

CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE OF id, title, description ON tasks BEGIN
    INSERT INTO tasks_fts(tasks_fts, rowid, id, title, description)
    VALUES ('delete', old.rowid, old.id, old.title, old.description);
    INSERT INTO tasks_fts(rowid, id, title, description)
    VALUES (new.rowid, new.id, new.title, new.description);
END;
