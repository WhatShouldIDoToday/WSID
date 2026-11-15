-- 0007_task_based_on.sql: Optional external URL a task is based on.

ALTER TABLE tasks ADD COLUMN based_on TEXT;
