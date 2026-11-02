-- 0002_manual_priority_1_to_5.sql: Normalize manual_priority to 1..=5 scale (3 = neutral)
UPDATE tasks
SET manual_priority = CASE
    WHEN manual_priority = 0 THEN 3
    WHEN manual_priority < 0 THEN MAX(1, 3 + (manual_priority / 2))
    WHEN manual_priority > 5 THEN 5
    ELSE manual_priority
END
WHERE manual_priority < 1 OR manual_priority > 5;
