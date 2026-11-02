CREATE TABLE IF NOT EXISTS users (
    username TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

INSERT OR IGNORE INTO users (username, created_at)
SELECT LOWER(TRIM(reporter)), MIN(created_at) FROM tasks WHERE TRIM(reporter) != '' GROUP BY LOWER(TRIM(reporter));

INSERT OR IGNORE INTO users (username, created_at)
SELECT LOWER(TRIM(assignee)), MIN(created_at) FROM tasks WHERE TRIM(assignee) != '' GROUP BY LOWER(TRIM(assignee));

INSERT OR IGNORE INTO users (username, created_at)
SELECT LOWER(TRIM(evaluator)), MIN(created_at) FROM tasks WHERE TRIM(evaluator) != '' GROUP BY LOWER(TRIM(evaluator));

INSERT OR IGNORE INTO users (username, created_at)
SELECT LOWER(TRIM(author)), MIN(created_at) FROM task_notes WHERE TRIM(author) != '' GROUP BY LOWER(TRIM(author));
