-- 0004_task_project.sql: Organize tasks into named projects.

ALTER TABLE tasks ADD COLUMN project TEXT NOT NULL DEFAULT 'default'
    CHECK (project NOT GLOB '*[^a-z0-9_-]*' AND length(project) > 0 AND project != 'all');

CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project);
