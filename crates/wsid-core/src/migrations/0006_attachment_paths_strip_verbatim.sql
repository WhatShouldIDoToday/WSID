-- Windows `fs::canonicalize` stored verbatim paths. Strip the `\\?\` prefix from drive paths
-- (`\\?\C:\x` -> `C:\x`) and convert verbatim UNC paths (`\\?\UNC\srv\share` -> `\\srv\share`).
-- Other verbatim forms (for example `\\?\Volume{...}`) are left untouched.
UPDATE task_attachments
SET file_path = substr(file_path, 5)
WHERE substr(file_path, 1, 4) = '\\?\'
  AND substr(file_path, 6, 2) = ':\';

UPDATE task_attachments
SET file_path = '\\' || substr(file_path, 9)
WHERE substr(file_path, 1, 8) = '\\?\UNC\';
