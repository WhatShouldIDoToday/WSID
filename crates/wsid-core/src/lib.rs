pub mod config;
pub mod db;
pub mod error;
pub mod id;
pub mod models;
pub mod notes;
pub mod relations;
pub mod scoring;
pub mod search;
pub mod tasks;

pub use config::{
    AppConfig, CONFIG_FILENAME, GLOBAL_DB_FILENAME, LOCAL_DB_FILENAME, WSID_DB_ENV_VAR, default_config_path, default_global_db_path,
    resolve_active_db_path, resolve_db_path, resolve_local_workspace_db_path,
};
pub use db::{DbChangeDetector, open_db, open_in_memory_db, run_migrations};
pub use error::{Result, WsidError};
pub use id::{SHORT_ID_LETTERS, ShortId, ShortIdGenerator, is_valid_task_id, normalize_task_id};
pub use models::{
    AddTaskNoteOutcome, CreateTaskInput, FocusPreset, OriginKind, RelationInputKind, RelationType, StatusTransitionOutcome, Task, TaskNote,
    TaskRelation, TaskStatus, UpdateTaskInput,
};
pub use notes::{ParsedNoteTokens, add_task_note, list_task_notes, parse_note_tokens, transition_task_status, unblock_or_dismiss_subtask};
pub use relations::{
    get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers, get_parent_chain,
    get_related_tasks, get_transitive_blocked_tasks, get_transitive_blockers, get_transitive_subtasks, link_tasks, list_task_relations,
    unlink_tasks, would_create_cycle,
};
pub use scoring::{
    BlockedSummaryGroup, BlockedTaskEntry, BlockerInfo, DEFAULT_FOCUS_LIMIT, DEFAULT_IMPORTANCE_THRESHOLD, DEFAULT_URGENT_HORIZON_DAYS,
    DEFAULT_WORKDAY_MINUTES, EisenhowerClassification, EisenhowerQuadrant, FocusQueueResponse, FocusWeights, ScoreBreakdown, ScoredTask,
    ScoringConfig, classify_task_eisenhower, compute_effective_slack_days, compute_focus_queue, continuous_urgency_score,
    get_blocked_tasks, list_eisenhower_matrix, resolve_effective_deadline, summarize_blocked_tasks,
};
pub use search::{SearchHit, SearchQuery, query_fts_trigram_ids, search_tasks};
pub use tasks::{
    create_task, delete_task, get_task, list_tasks, normalize_persona, normalize_tag, normalize_tags, update_task, validate_dates,
    validate_difficulty, validate_importance, validate_manual_priority, validate_time_estimate,
};
