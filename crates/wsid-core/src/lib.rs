pub mod about;
pub mod attachments;
pub mod bulk;
pub mod config;
pub mod db;
pub mod draft;
pub mod error;
pub mod id;
pub mod models;
pub mod notes;
pub mod relations;
pub mod scoring;
pub mod search;
pub mod tasks;

pub use about::{
    APP_FULL_NAME, APP_LICENSE_SPDX, APP_LICENSE_TEXT, APP_NAME, APP_REPOSITORY_URL, APP_VERSION, AboutMetadata, CrateAttribution,
    LicenseGroup, LicenseOverviewCount, RUST_LICENSES_JSON, RustLicenseBundle, THIRD_PARTY_NOTICES_TEXT, get_about_metadata,
    load_rust_license_bundle,
};
pub use attachments::{
    attach_existing_file, attach_existing_file_with_dir, get_attachment, infer_media_type, list_task_attachments, open_file_in_os,
    remove_attachment, remove_attachment_with_dir, save_clipboard_attachment, save_clipboard_attachment_with_dir,
    update_attachment_description,
};
pub use bulk::{BulkEditOutcome, BulkTaskFailure, BulkTaskPatch, bulk_delete_tasks, bulk_update_tasks};
pub use config::{
    APP_DIR_NAME, ATTACHMENTS_DIRNAME, AppConfig, CONFIG_FILENAME, GLOBAL_DB_FILENAME, LOCAL_DB_FILENAME, MAX_FOCUS_LIMIT, MIN_FOCUS_LIMIT,
    WSID_DB_ENV_VAR, default_app_data_dir, default_attachments_dir, default_config_path, default_global_db_path, resolve_active_db_path,
    resolve_db_path, resolve_local_workspace_db_path, validate_focus_limit,
};
pub use db::{Database, DbChangeDetector, open_db, open_in_memory_db, run_migrations};
pub use draft::{
    FieldIssue, IssueLevel, RelationDraft, Scalar, TaskDraft, TaskField, create_task_from_draft, create_task_from_draft_with_notices,
    search_relation_targets, suggest_users, summarize_issues, update_task_from_draft, update_task_from_draft_with_notices,
    validate_task_draft,
};
pub use error::{Result, WsidError};
pub use id::{SHORT_ID_LETTERS, ShortId, ShortIdGenerator, is_valid_task_id, normalize_task_id};
pub use models::{
    AddTaskNoteOutcome, AttachmentInput, CreateTaskInput, FocusPreset, OriginKind, RelationInputKind, RelationType,
    StatusTransitionOutcome, Task, TaskAttachment, TaskNote, TaskRelation, TaskStatus, UpdateTaskInput, UserAffectedTask,
    UserDeletionOutcome, UserDeletionPreview, UserNotice,
};
pub use notes::{
    ParsedNoteTokens, add_task_note, delete_task_note, delete_task_note_with_attachments_dir, list_task_notes, parse_note_tokens,
    transition_task_status, unblock_or_dismiss_subtask,
};
pub use relations::{
    get_direct_blocked_tasks, get_direct_blockers, get_direct_parents, get_direct_subtasks, get_parallel_peers, get_parent_chain,
    get_related_tasks, get_transitive_blocked_tasks, get_transitive_blockers, get_transitive_subtasks, link_tasks, list_task_relations,
    unlink_tasks, update_task_relation, would_create_cycle,
};
pub use scoring::{
    BlockedSummaryGroup, BlockedTaskEntry, BlockerInfo, DEFAULT_FOCUS_LIMIT, DEFAULT_IMPORTANCE_THRESHOLD, DEFAULT_URGENT_HORIZON_DAYS,
    DEFAULT_WORKDAY_MINUTES, EisenhowerClassification, EisenhowerQuadrant, FocusQueueResponse, FocusWeights, ScoreBreakdown, ScoredTask,
    ScoringConfig, classify_task_eisenhower, compute_effective_slack_days, compute_focus_queue, compute_focus_queue_in_project,
    continuous_urgency_score, get_blocked_tasks, list_eisenhower_matrix, resolve_effective_deadline, summarize_blocked_tasks,
};
pub use search::{SearchHit, SearchQuery, query_fts_trigram_ids, search_tasks};
pub use tasks::{
    ALL_PROJECTS, ALL_USERS, DEFAULT_PROJECT, DEFAULT_USER_KIND, UserRef, create_task, create_task_with_notices, delete_task,
    delete_task_with_attachments_dir, delete_user_from_db, format_deadline_date, format_deadline_display, format_duration_minutes,
    get_task, list_known_projects, list_known_users, list_tasks, list_user_kinds, local_tz_abbr, normalize_project, normalize_tag,
    normalize_tags, normalize_user, normalize_user_kind, parse_deadline_datetime, parse_deadline_naive_date, parse_duration_minutes,
    parse_user_ref, preview_user_deletion, register_user, set_user_kind, update_task, update_task_with_notices, validate_dates,
    validate_difficulty, validate_importance, validate_manual_priority, validate_not_past, validate_time_estimate,
};
