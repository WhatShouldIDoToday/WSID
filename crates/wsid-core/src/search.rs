use std::collections::HashSet;

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::attachments::load_task_description_attachments;
use crate::db::Database;
use crate::error::Result;
use crate::models::{Task, TaskStatus};
use crate::tasks::{
    ALL_PROJECTS, load_task_tags, map_task_row, normalize_project, normalize_tag, normalize_user, parse_deadline_datetime,
    parse_duration_minutes, parse_utc,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub status: Option<TaskStatus>,
    pub tag: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub evaluator: Option<String>,
    /// Restricts results to one project. [`crate::tasks::ALL_PROJECTS`] (or `None`) means every project.
    pub project: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub task: Task,
    pub score: u32,
}

/// Hybrid search combining SQLite structured filters + FTS5 trigram pre-filtering
/// with `nucleo-matcher` subsequence & typo-tolerant scoring.
///
/// Any whitespace-separated `<field><op><value>` token (e.g. `assignee==davidb`,
/// `imp>=3`, `due<=2026-11-03`) is applied as a structured predicate; the
/// remaining words are matched fuzzily, so filters and free text combine.
pub fn search_tasks(db: &Database, search: &SearchQuery) -> Result<Vec<SearchHit>> {
    let candidates = load_filtered_tasks(&db.conn, search)?;
    let (filters, free_text) = parse_query_filters(&search.query);
    let candidates: Vec<Task> = if filters.is_empty() {
        candidates
    } else {
        candidates
            .into_iter()
            .filter(|task| filters.iter().all(|filter| matches_filter(task, filter)))
            .collect()
    };

    let raw_query = free_text.trim();
    let limit = search.limit.unwrap_or(usize::MAX);

    if raw_query.is_empty() {
        return Ok(candidates
            .into_iter()
            .take(limit)
            .map(|task| SearchHit { task, score: 0 })
            .collect());
    }

    let cleaned_query = raw_query.strip_prefix('#').unwrap_or(raw_query).trim();
    let upper_query = cleaned_query.to_ascii_uppercase();
    let lower_query = cleaned_query.to_ascii_lowercase();

    // Also pull FTS5 trigram matches when any query token has >= 3 chars so we can boost exact trigram hits.
    let fts_ids = query_fts_trigram_ids(db, cleaned_query).unwrap_or_default();

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(cleaned_query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
    let mut utf32_buf = Vec::new();

    let mut hits = Vec::new();
    for task in candidates {
        let mut best_score: Option<u32> = None;

        // 1. Exact or prefix short-ID match gets top priority.
        if task.id == upper_query {
            best_score = Some(10_000);
        } else if task.id.starts_with(&upper_query) {
            best_score = Some(8_000);
        }

        // 2. Exact tag match gets a strong boost.
        if task.tags.iter().any(|t| t == &lower_query) {
            best_score = Some(best_score.unwrap_or(0).max(6_000));
        }

        // 3. Nucleo fuzzy match across ID, title, tags, description, and users.
        let composite = format!(
            "{} {} {} {} {} {} {}",
            task.id,
            task.title,
            task.tags.join(" "),
            task.description,
            task.assignee,
            task.reporter,
            task.evaluator
        );

        let haystack = Utf32Str::new(&composite, &mut utf32_buf);
        if let Some(nucleo_score) = pattern.score(haystack, &mut matcher) {
            let mut combined = nucleo_score.saturating_add(1_000);
            if fts_ids.contains(&task.id) {
                combined = combined.saturating_add(500);
            }
            best_score = Some(best_score.unwrap_or(0).max(combined));
        } else if let Some(typo_score) = score_typo_fallback(&lower_query, &task) {
            // 4. Typo fallback (bounded Damerau-Levenshtein edit distance on words) for single-keystroke typos
            // like "acessibility" -> "accessibility" where a character is omitted or transposed.
            let mut combined = typo_score;
            if fts_ids.contains(&task.id) {
                combined = combined.saturating_add(300);
            }
            best_score = Some(best_score.unwrap_or(0).max(combined));
        }

        if let Some(score) = best_score {
            hits.push(SearchHit { task, score });
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.task.importance.cmp(&a.task.importance))
            .then_with(|| a.task.id.cmp(&b.task.id))
    });
    hits.truncate(limit);

    Ok(hits)
}

/// Comparison operator parsed from a `<field><op><value>` search token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

impl ComparisonOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Lt => "<",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterField {
    Id,
    Title,
    Status,
    Project,
    Assignee,
    Reporter,
    Evaluator,
    Tag,
    Importance,
    Difficulty,
    Priority,
    Estimate,
    Due,
    Sgdb,
    Deadline,
}

impl FilterField {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "id" => Self::Id,
            "title" => Self::Title,
            "status" => Self::Status,
            "project" | "proj" => Self::Project,
            "assignee" | "assig" | "user" => Self::Assignee,
            "reporter" => Self::Reporter,
            "evaluator" => Self::Evaluator,
            "tag" => Self::Tag,
            "imp" | "importance" => Self::Importance,
            "diff" | "difficulty" => Self::Difficulty,
            "prio" | "priority" => Self::Priority,
            "est" | "estimate" => Self::Estimate,
            "due" | "due_date" => Self::Due,
            "sgdb" | "sgdb_date" => Self::Sgdb,
            "deadline" => Self::Deadline,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldFilter {
    field: FilterField,
    op: ComparisonOp,
    value: String,
}

/// Splits a query on whitespace while keeping double-quoted values intact, so
/// `title=="Fix login bug"` stays a single token.
fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in query.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unquote(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(trimmed)
}

/// Parses `<field><op><value>`; the operator must directly follow a known field name.
fn parse_field_filter(token: &str) -> Option<FieldFilter> {
    for (idx, ch) in token.char_indices() {
        let rest = &token[idx..];
        let (op, len) = if rest.starts_with("==") {
            (ComparisonOp::Eq, 2)
        } else if rest.starts_with("!=") {
            (ComparisonOp::Ne, 2)
        } else if rest.starts_with(">=") {
            (ComparisonOp::Ge, 2)
        } else if rest.starts_with("<=") {
            (ComparisonOp::Le, 2)
        } else if ch == '>' {
            (ComparisonOp::Gt, 1)
        } else if ch == '<' {
            (ComparisonOp::Lt, 1)
        } else if ch == '=' || ch == ':' {
            (ComparisonOp::Eq, 1)
        } else {
            continue;
        };
        let field = FilterField::from_name(&token[..idx])?;
        return Some(FieldFilter { field, op, value: unquote(&token[idx + len..]).to_string() });
    }
    None
}

fn parse_query_filters(query: &str) -> (Vec<FieldFilter>, String) {
    let mut filters = Vec::new();
    let mut free = Vec::new();
    for token in tokenize_query(query) {
        match parse_field_filter(&token) {
            Some(filter) => filters.push(filter),
            None => free.push(token),
        }
    }
    (filters, free.join(" "))
}

fn normalize_status(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn text_matches(actual: &str, needle: &str, op: ComparisonOp) -> bool {
    let actual = actual.trim().to_ascii_lowercase();
    let needle = needle.trim().to_ascii_lowercase();
    match op {
        ComparisonOp::Eq => actual == needle,
        ComparisonOp::Ne => actual != needle,
        ComparisonOp::Ge => actual >= needle,
        ComparisonOp::Le => actual <= needle,
        ComparisonOp::Gt => actual > needle,
        ComparisonOp::Lt => actual < needle,
    }
}

fn user_matches(actual: &str, needle: &str, op: ComparisonOp) -> bool {
    text_matches(actual.trim_start_matches('@'), needle.trim_start_matches('@'), op)
}

fn parse_numeric_value(raw: &str) -> Option<i64> {
    let value = raw.trim();
    let numeric = value.strip_suffix("/5").unwrap_or(value);
    if let Ok(parsed) = numeric.parse::<i64>() {
        return Some(parsed);
    }
    parse_duration_minutes(value).ok().map(i64::from)
}

fn ordinal_matches(actual: Option<i64>, filter: &FieldFilter) -> bool {
    let Some(needle) = parse_numeric_value(&filter.value) else {
        return false;
    };
    let Some(actual) = actual else {
        // A field with no value only satisfies "is not".
        return filter.op == ComparisonOp::Ne;
    };
    match filter.op {
        ComparisonOp::Eq => actual == needle,
        ComparisonOp::Ne => actual != needle,
        ComparisonOp::Ge => actual >= needle,
        ComparisonOp::Le => actual <= needle,
        ComparisonOp::Gt => actual > needle,
        ComparisonOp::Lt => actual < needle,
    }
}

/// Inclusive UTC window for a filter date. A date-only value means the whole
/// calendar day in the machine's local time zone; a date-time is one instant.
fn date_bounds(raw: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    if let Ok(date) = NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d") {
        let start = date.and_hms_opt(0, 0, 0)?;
        let end = date.succ_opt()?.and_hms_opt(0, 0, 0)? - Duration::milliseconds(1);
        let lo = start
            .and_local_timezone(Local)
            .earliest()?
            .with_timezone(&Utc);
        let hi = end
            .and_local_timezone(Local)
            .earliest()?
            .with_timezone(&Utc);
        return Some((lo, hi));
    }
    let instant = parse_utc(&parse_deadline_datetime("filter", raw).ok()?.0);
    Some((instant, instant))
}

fn date_matches(actual: Option<&str>, filter: &FieldFilter) -> bool {
    let needle = filter.value.trim().to_ascii_lowercase();
    if matches!(needle.as_str(), "" | "none" | "null" | "-" | "undated") {
        return match filter.op {
            ComparisonOp::Eq => actual.is_none(),
            ComparisonOp::Ne => actual.is_some(),
            _ => false,
        };
    }
    let Some(actual) = actual else {
        // Undated tasks never satisfy an ordered comparison, only "is not".
        return filter.op == ComparisonOp::Ne;
    };
    let actual = parse_utc(actual);
    let Some((lo, hi)) = date_bounds(&filter.value) else {
        return false;
    };
    match filter.op {
        ComparisonOp::Eq => actual >= lo && actual <= hi,
        ComparisonOp::Ne => actual < lo || actual > hi,
        ComparisonOp::Ge => actual >= lo,
        ComparisonOp::Le => actual <= hi,
        ComparisonOp::Gt => actual > hi,
        ComparisonOp::Lt => actual < lo,
    }
}

fn matches_filter(task: &Task, filter: &FieldFilter) -> bool {
    match filter.field {
        FilterField::Id => text_matches(&task.id, &filter.value, filter.op),
        FilterField::Title => text_matches(&task.title, &filter.value, filter.op),
        FilterField::Status => text_matches(&normalize_status(task.status.as_str()), &normalize_status(&filter.value), filter.op),
        FilterField::Project => text_matches(&task.project, &filter.value, filter.op),
        FilterField::Assignee => user_matches(&task.assignee, &filter.value, filter.op),
        FilterField::Reporter => user_matches(&task.reporter, &filter.value, filter.op),
        FilterField::Evaluator => user_matches(&task.evaluator, &filter.value, filter.op),
        FilterField::Tag => {
            let needle = filter.value.trim().trim_start_matches('#');
            let has_tag = task.tags.iter().any(|tag| tag.eq_ignore_ascii_case(needle));
            if filter.op == ComparisonOp::Ne { !has_tag } else { has_tag }
        }
        FilterField::Importance => ordinal_matches(Some(i64::from(task.importance)), filter),
        FilterField::Difficulty => ordinal_matches(Some(i64::from(task.difficulty)), filter),
        FilterField::Priority => ordinal_matches(Some(i64::from(task.manual_priority)), filter),
        FilterField::Estimate => ordinal_matches(task.time_estimate_minutes, filter),
        FilterField::Due => date_matches(task.due_date.as_deref(), filter),
        FilterField::Sgdb => date_matches(task.sgdb_date.as_deref(), filter),
        FilterField::Deadline => date_matches(task.sgdb_date.as_deref().or(task.due_date.as_deref()), filter),
    }
}

fn load_filtered_tasks(conn: &Connection, search: &SearchQuery) -> Result<Vec<Task>> {
    let mut sql = String::from(
        "SELECT
            t.id, t.title, t.description, t.status, t.origin_kind, t.origin_note_id,
            t.due_date, t.sgdb_date, t.time_estimate_minutes, t.difficulty, t.importance,
            t.urgent_override, t.manual_priority, t.reporter, t.assignee, t.evaluator,
            t.created_at, t.updated_at, t.completed_at, t.project
         FROM tasks t
         WHERE 1 = 1",
    );
    let mut bind_values: Vec<String> = Vec::new();

    if let Some(ref raw_project) = search.project
        && !raw_project.trim().is_empty()
        && raw_project.trim() != ALL_PROJECTS
    {
        bind_values.push(normalize_project(raw_project)?);
        sql.push_str(&format!(" AND t.project = ?{}", bind_values.len()));
    }

    if let Some(status) = search.status {
        bind_values.push(status.as_str().to_string());
        sql.push_str(&format!(" AND t.status = ?{}", bind_values.len()));
    }

    if let Some(ref raw_assignee) = search.assignee
        && !raw_assignee.trim().is_empty()
    {
        bind_values.push(normalize_user(raw_assignee)?);
        sql.push_str(&format!(" AND t.assignee = ?{}", bind_values.len()));
    }

    if let Some(ref raw_reporter) = search.reporter
        && !raw_reporter.trim().is_empty()
    {
        bind_values.push(normalize_user(raw_reporter)?);
        sql.push_str(&format!(" AND t.reporter = ?{}", bind_values.len()));
    }

    if let Some(ref raw_evaluator) = search.evaluator
        && !raw_evaluator.trim().is_empty()
    {
        bind_values.push(normalize_user(raw_evaluator)?);
        sql.push_str(&format!(" AND t.evaluator = ?{}", bind_values.len()));
    }

    if let Some(ref raw_tag) = search.tag
        && !raw_tag.trim().is_empty()
    {
        bind_values.push(normalize_tag(raw_tag)?);
        sql.push_str(&format!(
            " AND EXISTS (
                SELECT 1 FROM task_tags tt
                INNER JOIN tags tg ON tg.id = tt.tag_id
                WHERE tt.task_id = t.id AND tg.name = ?{}
            )",
            bind_values.len()
        ));
    }

    sql.push_str(" ORDER BY t.created_at ASC, t.rowid ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(bind_values.iter()), map_task_row)?;
    let mut tasks = Vec::new();
    for row in rows {
        let mut task = row?;
        task.tags = load_task_tags(conn, &task.id)?;
        task.attachments = load_task_description_attachments(conn, &task.id)?;
        tasks.push(task);
    }
    Ok(tasks)
}

/// Queries SQLite FTS5 trigram index for 3+ character tokens.
pub fn query_fts_trigram_ids(db: &Database, query: &str) -> Result<HashSet<String>> {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|tok| {
            tok.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|tok| tok.chars().count() >= 3)
        .collect();

    if tokens.is_empty() {
        return Ok(HashSet::new());
    }

    let match_expr = tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");

    let mut stmt = db
        .conn
        .prepare("SELECT id FROM tasks_fts WHERE tasks_fts MATCH ?1")?;
    let rows = stmt.query_map([&match_expr], |r| r.get::<_, String>(0))?;

    let mut ids = HashSet::new();
    for row in rows {
        ids.push_or_insert(row?);
    }
    Ok(ids)
}

trait SetInsertExt<T> {
    fn push_or_insert(&mut self, value: T);
}

impl<T: Eq + std::hash::Hash> SetInsertExt<T> for HashSet<T> {
    fn push_or_insert(&mut self, value: T) {
        self.insert(value);
    }
}

/// Evaluates typo-tolerant word similarity for queries of length >= 4 when strict subsequence matching misses
/// (for example, `"acessibility"` with one missing `'c'` matching `"accessibility"`).
fn score_typo_fallback(lower_query: &str, task: &Task) -> Option<u32> {
    let query_tokens: Vec<&str> = lower_query
        .split_whitespace()
        .filter(|t| t.len() >= 4)
        .collect();
    if query_tokens.is_empty() {
        return None;
    }

    let candidate_text = format!("{} {} {}", task.title, task.description, task.tags.join(" ")).to_ascii_lowercase();
    let candidate_words: Vec<&str> = candidate_text
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| !w.is_empty())
        .collect();

    let mut total_score = 0u32;
    for q_tok in &query_tokens {
        let max_dist = if q_tok.len() >= 8 { 2 } else { 1 };
        let best_dist = candidate_words
            .iter()
            .filter(|w| w.len().abs_diff(q_tok.len()) <= max_dist)
            .map(|w| damerau_levenshtein(q_tok, w))
            .min()?;

        if best_dist <= max_dist {
            total_score += (500u32).saturating_sub((best_dist as u32) * 150);
        } else {
            return None;
        }
    }

    Some(total_score)
}

fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);

            if i > 1 && j > 1 && a_chars[i - 1] == b_chars[j - 2] && a_chars[i - 2] == b_chars[j - 1] {
                dp[i][j] = dp[i][j].min(dp[i - 2][j - 2] + 1);
            }
        }
    }

    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;
    use crate::models::CreateTaskInput;
    use crate::tasks::create_task;

    #[test]
    fn supports_exact_id_short_queries_typos_and_structured_filters() {
        let mut db = open_in_memory_db().unwrap();

        let t1 = create_task(
            &mut db,
            CreateTaskInput {
                title: "Audit keyboard accessibility in Kanban".to_string(),
                description: Some("Ensure WCAG 2.2 focus rings and ARIA labels work".to_string()),
                status: Some(TaskStatus::InProgress),
                tags: vec!["a11y".to_string(), "frontend".to_string()],
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "david",
        )
        .unwrap();

        let t2 = create_task(
            &mut db,
            CreateTaskInput {
                title: "Optimize SQLite WAL checkpointing".to_string(),
                description: Some("Verify PRAGMA data_version polling".to_string()),
                status: Some(TaskStatus::Todo),
                tags: vec!["backend".to_string(), "sqlite".to_string()],
                assignee: Some("bob".to_string()),
                ..Default::default()
            },
            "david",
        )
        .unwrap();

        // 1. Exact short ID lookup (with or without leading '#')
        let id_hits = search_tasks(&db, &SearchQuery { query: format!("#{}", t1.id.to_ascii_lowercase()), ..Default::default() }).unwrap();
        assert_eq!(id_hits.first().unwrap().task.id, t1.id);

        // 2. 1-2 character query ("kb" subsequence matching "Kanban" / "keyboard")
        let short_hits = search_tasks(&db, &SearchQuery { query: "kb".to_string(), ..Default::default() }).unwrap();
        assert!(!short_hits.is_empty());
        assert_eq!(short_hits[0].task.id, t1.id);

        // 3. Typo tolerance ("acessibility" missing one 'c' matches "accessibility")
        let typo_hits = search_tasks(&db, &SearchQuery { query: "acessibility".to_string(), ..Default::default() }).unwrap();
        assert!(!typo_hits.is_empty());
        assert_eq!(typo_hits[0].task.id, t1.id);

        // 4. Combined tag + status + assignee structured filtering
        let filtered_hits = search_tasks(
            &db,
            &SearchQuery {
                query: String::new(),
                status: Some(TaskStatus::Todo),
                tag: Some("sqlite".to_string()),
                assignee: Some("@bob".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered_hits.len(), 1);
        assert_eq!(filtered_hits[0].task.id, t2.id);
    }

    #[test]
    fn parses_structured_filter_tokens_and_keeps_free_text() {
        let (filters, free) = parse_query_filters("assignee==davidb imp>=3 due<=2026-11-03 release");
        assert_eq!(free, "release");
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].field, FilterField::Assignee);
        assert_eq!(filters[0].op, ComparisonOp::Eq);
        assert_eq!(filters[0].value, "davidb");
        assert_eq!(filters[1].field, FilterField::Importance);
        assert_eq!(filters[1].op, ComparisonOp::Ge);
        assert_eq!(filters[2].field, FilterField::Due);
        assert_eq!(filters[2].op, ComparisonOp::Le);

        // Quoted values keep their spaces, and non-field words stay free text.
        let (filters, free) = parse_query_filters("title==\"Fix login bug\" hello:world");
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].value, "Fix login bug");
        assert_eq!(free, "hello:world");
    }

    #[test]
    fn filters_by_assignee_importance_and_status_aliases() {
        let mut db = open_in_memory_db().unwrap();
        let low = create_task(
            &mut db,
            CreateTaskInput {
                title: "Low priority chore".to_string(),
                status: Some(TaskStatus::Todo),
                assignee: Some("davidb".to_string()),
                importance: Some(2),
                ..Default::default()
            },
            "david",
        )
        .unwrap();
        let high = create_task(
            &mut db,
            CreateTaskInput {
                title: "High priority fix".to_string(),
                status: Some(TaskStatus::InProgress),
                assignee: Some("alice".to_string()),
                importance: Some(5),
                ..Default::default()
            },
            "david",
        )
        .unwrap();

        let mine = search_tasks(&db, &SearchQuery { query: "assignee==davidb".to_string(), ..Default::default() }).unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].task.id, low.id);

        let urgent = search_tasks(&db, &SearchQuery { query: "imp>=5".to_string(), ..Default::default() }).unwrap();
        assert_eq!(urgent.len(), 1);
        assert_eq!(urgent[0].task.id, high.id);

        let in_progress = search_tasks(&db, &SearchQuery { query: "status==in-progress".to_string(), ..Default::default() }).unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].task.id, high.id);

        let in_progress = search_tasks(&db, &SearchQuery { query: "status:in_progress".to_string(), ..Default::default() }).unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].task.id, high.id);

        // Filters and free text combine.
        let combined = search_tasks(&db, &SearchQuery { query: "assignee==davidb chore".to_string(), ..Default::default() }).unwrap();
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].task.id, low.id);
    }

    #[test]
    fn filters_dates_by_local_calendar_day() {
        let mut db = open_in_memory_db().unwrap();
        let dated = create_task(
            &mut db,
            CreateTaskInput { title: "Dated task".to_string(), due_date: Some("2026-11-03T15:00:00".to_string()), ..Default::default() },
            "david",
        )
        .unwrap();
        create_task(&mut db, CreateTaskInput { title: "Undated task".to_string(), ..Default::default() }, "david").unwrap();

        let same_day = search_tasks(&db, &SearchQuery { query: "due==2026-11-03".to_string(), ..Default::default() }).unwrap();
        assert_eq!(same_day.len(), 1);
        assert_eq!(same_day[0].task.id, dated.id);

        let before = search_tasks(&db, &SearchQuery { query: "due<2026-11-03".to_string(), ..Default::default() }).unwrap();
        assert!(before.is_empty());

        let after = search_tasks(&db, &SearchQuery { query: "due>2026-11-03".to_string(), ..Default::default() }).unwrap();
        assert!(after.is_empty());

        let from_day = search_tasks(&db, &SearchQuery { query: "due>=2026-11-03".to_string(), ..Default::default() }).unwrap();
        assert_eq!(from_day.len(), 1);

        // Undated tasks are addressable, and never match ordered comparisons.
        let undated = search_tasks(&db, &SearchQuery { query: "due==none".to_string(), ..Default::default() }).unwrap();
        assert_eq!(undated.len(), 1);
        assert_eq!(undated[0].task.title, "Undated task");
        let undated_ge = search_tasks(&db, &SearchQuery { query: "due>=2026-01-01".to_string(), ..Default::default() }).unwrap();
        assert_eq!(undated_ge.len(), 1);
    }
}
